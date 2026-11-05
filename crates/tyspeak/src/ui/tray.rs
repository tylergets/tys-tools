//! The tray daemon: owns the microphone, the model and its downloads, and serves D-Bus.

use std::cell::{Cell, OnceCell, RefCell};
use std::path::PathBuf;
use std::process::{self, Command};
use std::rc::{Rc, Weak};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use gtk::glib::{self, ControlFlow, SourceId};
use serde::Serialize;
use ty_common::icons::Tool;
use ty_common::tray::{
    self, drain_actions, spawn_blocking, Applet, IconState, MenuEntry, Receiver, TraySpec,
};

use crate::config::AppConfig;
use crate::core::dbus::{Reply, Request, Service};
use crate::core::saved_state::{self, SavedState};
use crate::core::status::{format_elapsed, DownloadStatus, ModelState, State, Status};
use crate::platform::output;
use crate::speech::audio::{self, level_meter, Chime, Recorder, SAMPLE_RATE};
use crate::speech::download::{self, Download};
use crate::speech::engine::Engine;
use crate::speech::models::{self, ModelInfo, MODELS};

const APP_ID: &str = "tyspeak";
const NOTIFY_TIMEOUT: Duration = Duration::from_millis(4000);
/// Menu, status and download refresh rate while recording, transcribing or downloading.
const TICK_INTERVAL: Duration = Duration::from_millis(250);
/// Recordings shorter than this are dropped.
const MIN_SAMPLES: usize = SAMPLE_RATE as usize / 4;
/// Recordings whose peak stays below this are treated as silence (models tend to invent
/// text for silence).
const SILENCE_PEAK_DB: f32 = -50.0;

pub fn run(config: AppConfig) -> Result<()> {
    if std::env::var_os("DISPLAY").is_none() {
        bail!("DISPLAY is not set; tyspeak requires an X11 session");
    }
    gtk::init()?;
    let model_dir = config.resolved_model_dir()?;

    // Claim the bus name first, so a second instance exits before showing a tray icon.
    let app_slot: Rc<OnceCell<Weak<App>>> = Rc::new(OnceCell::new());
    let service = {
        let app_slot = app_slot.clone();
        Service::start(
            move |request| match app_slot.get().and_then(Weak::upgrade) {
                Some(app) => app.handle(request),
                None => bail!("tyspeak is starting up"),
            },
        )?
    };

    let saved = saved_state::load();
    let selected = saved
        .model
        .as_deref()
        .and_then(models::find)
        .or_else(|| models::find(&config.model))
        .context("no model selected")?;

    let (tray_tx, tray_rx) = tray::channel();
    let placeholder = TraySpec {
        icon: IconState::Normal,
        title: APP_ID.to_string(),
        menu: Vec::new(),
        click_action: None,
    };
    let applet = Applet::new(APP_ID, &Tool::Speak.tray_icons(), placeholder, tray_tx)?;
    let app = App::new(config, model_dir, selected, applet, service);
    let _ = app_slot.set(Rc::downgrade(&app));

    app.listen_for_tray(tray_rx);
    let auto_download = app.config.auto_download;
    app.ensure_model(auto_download);
    app.tick();

    gtk::main();
    Ok(())
}

/// Tray menu and click actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    Toggle,
    Cancel,
    SelectModel(&'static str),
    CancelDownload,
    RetryModel,
    OpenModelsFolder,
    Quit,
}

struct App {
    /// Handle for callbacks that outlive a borrow of the app.
    this: Weak<App>,
    config: AppConfig,
    model_dir: PathBuf,
    applet: RefCell<Applet<TrayAction>>,
    service: Service,
    engine: Engine,
    /// The model picked in the tray / config.
    selected: Cell<&'static ModelInfo>,
    /// The model in the engine, usable for dictation. Can differ from `selected` while the
    /// selected one downloads.
    active: Cell<Option<&'static ModelInfo>>,
    loading: Cell<Option<&'static ModelInfo>>,
    /// Model id and error of the last failed download or load.
    failed: RefCell<Option<(&'static str, String)>>,
    /// Ids of the models on disk.
    downloaded: RefCell<Vec<&'static str>>,
    download: RefCell<Option<Download>>,
    recorder: RefCell<Option<Recorder>>,
    /// A stop waiting out `audio.tail_ms`. A start in that window cancels it, which keeps
    /// hold-to-talk bindings working through X key autorepeat (a release/press pair every
    /// ~30 ms while the key is held).
    pending_stop: RefCell<Option<SourceId>>,
    /// Recordings handed to the engine and not yet returned.
    transcribing: Cell<u32>,
    /// Whether the refresh timer runs (only while [`App::busy`]).
    ticking: Cell<bool>,
    last_status: RefCell<Option<Status>>,
}

#[derive(Serialize)]
struct ModelListing {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    size_bytes: u64,
    downloaded: bool,
    selected: bool,
    active: bool,
}

impl App {
    fn new(
        config: AppConfig,
        model_dir: PathBuf,
        selected: &'static ModelInfo,
        applet: Applet<TrayAction>,
        service: Service,
    ) -> Rc<Self> {
        let app = Rc::new_cyclic(|this| Self {
            this: this.clone(),
            config,
            model_dir,
            applet: RefCell::new(applet),
            service,
            engine: Engine::default(),
            selected: Cell::new(selected),
            active: Cell::new(None),
            loading: Cell::new(None),
            failed: RefCell::new(None),
            downloaded: RefCell::new(Vec::new()),
            download: RefCell::new(None),
            recorder: RefCell::new(None),
            pending_stop: RefCell::new(None),
            transcribing: Cell::new(0),
            ticking: Cell::new(false),
            last_status: RefCell::new(None),
        });
        app.refresh_downloaded();
        app
    }

    fn listen_for_tray(self: &Rc<Self>, tray_actions: Receiver<TrayAction>) {
        let weak = Rc::downgrade(self);
        drain_actions(tray_actions, move |action| {
            let Some(app) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            let result = match action {
                TrayAction::Toggle => app.toggle(),
                TrayAction::Cancel => {
                    app.cancel();
                    Ok(())
                }
                TrayAction::SelectModel(id) => app.select_model(id),
                TrayAction::CancelDownload => {
                    app.download.borrow_mut().take();
                    Ok(())
                }
                TrayAction::RetryModel => {
                    app.retry_model();
                    Ok(())
                }
                TrayAction::OpenModelsFolder => app.open_models_folder(),
                TrayAction::Quit => app.quit(),
            };
            app.report(result);
            app.tick();
            ControlFlow::Continue
        });
    }

    /// A D-Bus call.
    fn handle(&self, request: Request) -> Result<Reply> {
        let reply = match request {
            Request::Start => self.start().map(|_| Reply::Empty),
            Request::Stop => {
                self.stop();
                Ok(Reply::Empty)
            }
            Request::Toggle => self.toggle().map(|_| Reply::Empty),
            Request::Cancel => {
                self.cancel();
                Ok(Reply::Empty)
            }
            Request::Status => Ok(Reply::Text(serde_json::to_string(&self.snapshot())?)),
            Request::ListModels => Ok(Reply::Text(serde_json::to_string(&self.listing())?)),
            Request::SetModel(id) => self.select_model(&id).map(|_| Reply::Empty),
            Request::Retry => {
                self.retry_model();
                Ok(Reply::Empty)
            }
        };
        self.tick();
        reply
    }

    // ---- Dictation ------------------------------------------------------------------------

    fn toggle(&self) -> Result<()> {
        if self.recorder.borrow().is_some() {
            self.stop();
            Ok(())
        } else {
            self.start()
        }
    }

    fn start(&self) -> Result<()> {
        if self.recorder.borrow().is_some() {
            // Still the same utterance: keep recording.
            if let Some(source) = self.pending_stop.borrow_mut().take() {
                source.remove();
            }
            return Ok(());
        }
        if self.active.get().is_none() && self.loading.get().is_none() {
            let selected = self.selected.get();
            if let Some(download) = self.download.borrow().as_ref() {
                bail!(
                    "{} is still downloading ({:.0}%)",
                    download.model.name,
                    download.progress().fraction() * 100.0
                );
            }
            if !selected.is_downloaded(&self.model_dir) {
                self.start_download(selected);
                bail!("{} isn't downloaded yet; downloading it now", selected.name);
            }
            // Downloaded but failed to load: try again.
            self.load(selected);
        }
        let recorder = Recorder::start(&self.config.audio.source)?;
        if self.config.audio.start_sound {
            audio::play_chime(Chime::Start);
        }
        *self.recorder.borrow_mut() = Some(recorder);
        Ok(())
    }

    /// Stop recording and transcribe in the background.
    /// Stop after `audio.tail_ms`, then transcribe in the background.
    fn stop(&self) {
        if self.recorder.borrow().is_none() || self.pending_stop.borrow().is_some() {
            return;
        }
        let tail = Duration::from_millis(u64::from(self.config.audio.tail_ms));
        let weak = self.this.clone();
        let source = glib::timeout_add_local_once(tail, move || {
            if let Some(app) = weak.upgrade() {
                app.finish_stop();
                app.tick();
            }
        });
        *self.pending_stop.borrow_mut() = Some(source);
    }

    fn finish_stop(&self) {
        // The timer fired, so its source is already gone; just forget it.
        self.pending_stop.borrow_mut().take();
        let Some(recorder) = self.recorder.borrow_mut().take() else {
            return;
        };
        self.transcribing.set(self.transcribing.get() + 1);
        let engine = self.engine.clone();
        let language = self.config.language.clone();
        let stop_sound = self.config.audio.stop_sound;
        let weak = self.this.clone();
        spawn_blocking(
            move || -> Result<Option<String>> {
                let pcm = recorder.finish();
                // After the mic is closed, so the chime doesn't end up in the recording.
                if stop_sound {
                    audio::play_chime(Chime::Stop);
                }
                if pcm.len() < MIN_SAMPLES || audio::peak_db(&pcm) < SILENCE_PEAK_DB {
                    return Ok(None);
                }
                engine.transcribe(&pcm, language.as_deref()).map(Some)
            },
            move |result| {
                let Some(app) = weak.upgrade() else { return };
                app.transcribing
                    .set(app.transcribing.get().saturating_sub(1));
                app.deliver(result);
                app.tick();
            },
        );
    }

    fn cancel(&self) {
        if let Some(source) = self.pending_stop.borrow_mut().take() {
            source.remove();
        }
        self.recorder.borrow_mut().take();
    }

    fn deliver(&self, result: Result<Option<String>>) {
        let text = match result {
            Ok(Some(text)) if !text.trim().is_empty() => text.trim().to_string(),
            Ok(_) => {
                eprintln!("no speech recognised");
                return;
            }
            Err(err) => return self.report(Err(err)),
        };
        println!("transcribed: {text}");
        self.service.emit_transcribed(&text);

        let output = &self.config.output;
        let mut delivered = text;
        if output.trailing_space {
            delivered.push(' ');
        }
        if output.mode.copies() {
            self.report(output::copy_to_clipboard(&delivered));
        }
        if output.mode.types() {
            let delay = output.type_delay_ms;
            let notify_errors = self.config.notifications.enabled;
            spawn_blocking(
                move || output::type_text(&delivered, delay),
                move |result| {
                    if let Err(err) = result {
                        report_error(&err, notify_errors);
                    }
                },
            );
        }
    }

    // ---- Models ---------------------------------------------------------------------------

    fn select_model(&self, id: &str) -> Result<()> {
        let model = models::find(id).with_context(|| format!("unknown model '{id}'"))?;
        let changed = self.selected.get() != model;
        self.selected.set(model);
        if changed {
            let state = SavedState {
                model: Some(model.id.to_string()),
            };
            if let Err(err) = saved_state::save(&state) {
                eprintln!("warning: failed to remember the model: {err:#}");
            }
        }
        if self.active.get() == Some(model) || self.loading.get() == Some(model) {
            return Ok(());
        }
        self.failed.borrow_mut().take();
        self.ensure_model(true);
        Ok(())
    }

    /// Download (or load) the selected model again, e.g. after a failure. A partial download
    /// is resumed. Does nothing while it is already downloading, loading or loaded.
    fn retry_model(&self) {
        let model = self.selected.get();
        if self.active.get() == Some(model) || self.loading.get() == Some(model) {
            return;
        }
        self.failed.borrow_mut().take();
        self.ensure_model(true);
    }

    /// Load the selected model, downloading it first if needed (and allowed).
    fn ensure_model(&self, download_if_missing: bool) {
        let model = self.selected.get();
        if model.is_downloaded(&self.model_dir) {
            self.load(model);
        } else if download_if_missing {
            self.start_download(model);
        }
    }

    fn start_download(&self, model: &'static ModelInfo) {
        if self
            .download
            .borrow()
            .as_ref()
            .is_some_and(|download| download.model == model)
        {
            return;
        }
        // Replacing a download cancels it; its partial file is kept for resuming.
        *self.download.borrow_mut() = Some(Download::start(model, self.model_dir.clone()));
        self.notify(
            &format!("Downloading {}", model.name),
            &format!("{} from Hugging Face", model.size_label()),
        );
    }

    fn poll_download(&self) {
        let finished = self
            .download
            .borrow()
            .as_ref()
            .is_some_and(Download::is_finished);
        if !finished {
            return;
        }
        let Some(download) = self.download.borrow_mut().take() else {
            return;
        };
        let model = download.model;
        match download.join() {
            Ok(_) => {
                self.refresh_downloaded();
                if self.selected.get() == model {
                    self.load(model);
                } else {
                    self.notify(&format!("Downloaded {}", model.name), "");
                }
            }
            Err(err) if download::is_cancelled(&err) => {}
            Err(err) => {
                *self.failed.borrow_mut() = Some((model.id, format!("{err:#}")));
                self.report(Err(err.context(format!(
                    "downloading {} failed; use Retry Download in the tray or `tyspeak retry`",
                    model.name
                ))));
            }
        }
    }

    fn load(&self, model: &'static ModelInfo) {
        let generation = self.engine.next_generation();
        self.loading.set(Some(model));
        self.failed.borrow_mut().take();
        let engine = self.engine.clone();
        let path = model.path_in(&self.model_dir);
        let threads = self.config.threads;
        let weak = self.this.clone();
        spawn_blocking(
            move || engine.load(generation, model.id, &path, threads),
            move |result| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                if !app.engine.is_current(generation) {
                    return;
                }
                app.loading.set(None);
                match result {
                    Ok(true) => app.active.set(Some(model)),
                    Ok(false) => {}
                    Err(err) => {
                        // The engine dropped the previous model before loading.
                        app.active.set(None);
                        *app.failed.borrow_mut() = Some((model.id, format!("{err:#}")));
                        app.report(Err(err));
                    }
                }
                app.tick();
            },
        );
    }

    fn refresh_downloaded(&self) {
        *self.downloaded.borrow_mut() = MODELS
            .iter()
            .filter(|model| model.is_downloaded(&self.model_dir))
            .map(|model| model.id)
            .collect();
    }

    fn listing(&self) -> Vec<ModelListing> {
        let downloaded = self.downloaded.borrow();
        MODELS
            .iter()
            .map(|model| ModelListing {
                id: model.id,
                name: model.name,
                summary: model.summary,
                size_bytes: model.bytes,
                downloaded: downloaded.contains(&model.id),
                selected: self.selected.get() == model,
                active: self.active.get() == Some(model),
            })
            .collect()
    }

    // ---- Status ---------------------------------------------------------------------------

    /// Refresh now, and keep refreshing while something is in progress.
    fn tick(&self) {
        self.refresh();
        if self.ticking.get() || !self.busy() {
            return;
        }
        self.ticking.set(true);
        let weak = self.this.clone();
        glib::timeout_add_local(TICK_INTERVAL, move || {
            let Some(app) = weak.upgrade() else {
                return ControlFlow::Break;
            };
            app.refresh();
            if app.busy() {
                return ControlFlow::Continue;
            }
            app.ticking.set(false);
            ControlFlow::Break
        });
    }

    /// Something changes on its own (elapsed time, levels, progress) and needs refreshing.
    fn busy(&self) -> bool {
        self.recorder.borrow().is_some()
            || self.pending_stop.borrow().is_some()
            || self.transcribing.get() > 0
            || self.loading.get().is_some()
            || self.download.borrow().is_some()
    }

    fn refresh(&self) {
        self.poll_download();
        self.check_recorder();
        let status = self.snapshot();
        self.applet.borrow_mut().update(self.tray_spec(&status));
        let changed = self.last_status.borrow().as_ref() != Some(&status);
        if changed {
            self.service.emit_status(&status);
            *self.last_status.borrow_mut() = Some(status);
        }
    }

    /// Notice parec dying, and stop at the configured maximum length.
    fn check_recorder(&self) {
        let (failure, too_long) = match self.recorder.borrow_mut().as_mut() {
            Some(recorder) => (
                recorder.failure(),
                recorder.elapsed().as_secs() >= u64::from(self.config.audio.max_duration),
            ),
            None => return,
        };
        if let Some(failure) = failure {
            self.recorder.borrow_mut().take();
            self.report(Err(anyhow::anyhow!("recording failed: {failure}")));
        } else if too_long {
            self.stop();
        }
    }

    fn model_state(&self) -> ModelState {
        let selected = self.selected.get();
        if self.active.get() == Some(selected) {
            ModelState::Ready
        } else if self.loading.get() == Some(selected) {
            ModelState::Loading
        } else if self
            .download
            .borrow()
            .as_ref()
            .is_some_and(|download| download.model == selected)
        {
            ModelState::Downloading
        } else if self
            .failed
            .borrow()
            .as_ref()
            .is_some_and(|(id, _)| *id == selected.id)
        {
            ModelState::Failed
        } else {
            ModelState::Missing
        }
    }

    fn snapshot(&self) -> Status {
        let recorder = self.recorder.borrow();
        let state = if recorder.is_some() {
            State::Recording
        } else if self.transcribing.get() > 0 {
            State::Transcribing
        } else {
            State::Idle
        };
        Status {
            state,
            model: self.selected.get().id.to_string(),
            model_state: self.model_state(),
            download: self
                .download
                .borrow()
                .as_ref()
                .map(|download| DownloadStatus {
                    model: download.model.id.to_string(),
                    done_bytes: download.progress().done(),
                    total_bytes: download.progress().total(),
                }),
            elapsed_secs: recorder.as_ref().map_or(0, |r| r.elapsed().as_secs()),
            mic_level: recorder.as_ref().map(Recorder::level),
        }
    }

    fn tray_spec(&self, status: &Status) -> TraySpec<TrayAction> {
        let selected = self.selected.get();
        let download = self.download.borrow();
        let download = download.as_ref();
        let headline = match status.state {
            State::Recording => format!(
                "● Recording {}  {}",
                format_elapsed(status.elapsed_secs),
                level_meter(status.mic_level.unwrap_or(0.0))
            ),
            State::Transcribing => "Transcribing…".to_string(),
            State::Idle | State::Offline => match (self.active.get(), self.loading.get()) {
                (Some(active), _) => format!("Ready — {}", active.name),
                (None, Some(loading)) => format!("Loading {}…", loading.name),
                (None, None) if download.is_some() => "Waiting for the download…".to_string(),
                (None, None) if status.model_state == ModelState::Failed => {
                    format!("{} failed", selected.name)
                }
                (None, None) => "No model — pick one below".to_string(),
            },
        };

        let action = |label: String, enabled: bool, action: TrayAction| MenuEntry::Action {
            label,
            enabled,
            action,
        };
        let recording = status.state == State::Recording;

        let mut menu = vec![MenuEntry::Label(headline.clone())];
        if let (Some(loading), Some(_)) = (self.loading.get(), self.active.get()) {
            menu.push(MenuEntry::Label(format!("Loading {}…", loading.name)));
        }
        if let Some(download) = download {
            menu.push(MenuEntry::Label(format!(
                "Downloading {} — {}",
                download.model.name,
                describe_download(download)
            )));
        }
        menu.extend([
            MenuEntry::Separator,
            action(
                if recording {
                    "Stop Dictation".into()
                } else {
                    "Start Dictation".into()
                },
                true,
                TrayAction::Toggle,
            ),
            action("Cancel Dictation".into(), recording, TrayAction::Cancel),
            MenuEntry::Separator,
            MenuEntry::Submenu {
                label: format!("Model: {}", selected.name),
                entries: self.model_entries(download),
            },
        ]);
        if download.is_some() {
            menu.push(action(
                "Cancel Download".into(),
                true,
                TrayAction::CancelDownload,
            ));
        }
        let retry_label = match status.model_state {
            ModelState::Failed if self.downloaded.borrow().contains(&selected.id) => {
                Some("Retry Loading".to_string())
            }
            ModelState::Failed => Some("Retry Download".to_string()),
            ModelState::Missing => Some(format!("Download {}", selected.name)),
            _ => None,
        };
        if let Some(label) = retry_label {
            menu.push(action(label, true, TrayAction::RetryModel));
        }
        menu.extend([
            action(
                "Open Models Folder".into(),
                true,
                TrayAction::OpenModelsFolder,
            ),
            MenuEntry::Separator,
            action("Quit".into(), true, TrayAction::Quit),
        ]);

        let busy = status.state == State::Transcribing
            || download.is_some()
            || self.loading.get().is_some();
        TraySpec {
            icon: if recording {
                IconState::Attention
            } else if busy {
                IconState::Busy
            } else {
                IconState::Normal
            },
            title: format!("tyspeak — {headline}"),
            menu,
            click_action: Some(TrayAction::Toggle),
        }
    }

    fn model_entries(&self, download: Option<&Download>) -> Vec<MenuEntry<TrayAction>> {
        let downloaded = self.downloaded.borrow();
        MODELS
            .iter()
            .map(|model| {
                let mark = if self.selected.get() == model {
                    "●"
                } else {
                    "○"
                };
                let detail = match download {
                    Some(download) if download.model == model => {
                        format!("downloading {}", describe_download(download))
                    }
                    _ if downloaded.contains(&model.id) => model.size_label(),
                    _ => format!("{}, not downloaded", model.size_label()),
                };
                MenuEntry::Action {
                    label: format!("{mark} {} — {detail}", model.name),
                    enabled: true,
                    action: TrayAction::SelectModel(model.id),
                }
            })
            .collect()
    }

    // ---- Misc -----------------------------------------------------------------------------

    fn open_models_folder(&self) -> Result<()> {
        std::fs::create_dir_all(&self.model_dir)
            .with_context(|| format!("failed to create {}", self.model_dir.display()))?;
        Command::new("xdg-open")
            .arg(&self.model_dir)
            .spawn()
            .context("failed to launch xdg-open")?;
        Ok(())
    }

    fn quit(&self) -> Result<()> {
        self.cancel();
        process::exit(0);
    }

    fn notify(&self, summary: &str, body: &str) {
        if self.config.notifications.enabled {
            notify(summary, body);
        }
    }

    fn report(&self, result: Result<()>) {
        if let Err(err) = result {
            report_error(&err, self.config.notifications.enabled);
        }
    }
}

fn describe_download(download: &Download) -> String {
    DownloadStatus {
        model: String::new(),
        done_bytes: download.progress().done(),
        total_bytes: download.progress().total(),
    }
    .describe()
}

fn notify(summary: &str, body: &str) {
    ty_common::notify::notify(APP_ID, summary, body, NOTIFY_TIMEOUT);
}

fn report_error(err: &anyhow::Error, notify_user: bool) {
    eprintln!("error: {err:#}");
    if notify_user {
        notify("tyspeak", &format!("{err:#}"));
    }
}
