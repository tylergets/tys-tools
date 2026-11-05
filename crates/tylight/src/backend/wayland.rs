//! wlroots compositors (Sway, Hyprland, river, niri, labwc, ...):
//! `wlr-gamma-control-unstable-v1`. The compositor restores the original ramps by itself
//! when tylight disconnects.

use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;

use anyhow::{bail, Context, Result};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1;
use wayland_protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_v1::{
    self, ZwlrGammaControlV1,
};

use super::Backend;
use crate::color::{ramps, Setting};

struct Output {
    /// Registry name, used to match `global_remove`.
    global: u32,
    wl: WlOutput,
    control: Option<ZwlrGammaControlV1>,
    size: Option<usize>,
    failed: bool,
}

#[derive(Default)]
struct State {
    manager: Option<ZwlrGammaControlManagerV1>,
    outputs: Vec<Output>,
    /// An output became ready (or went away) since the last poll.
    changed: bool,
}

pub struct WaylandBackend {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
}

impl WaylandBackend {
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("failed to connect to Wayland")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());

        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        if state.manager.is_none() {
            bail!(
                "this compositor doesn't support wlr-gamma-control \
                 (GNOME and KDE have their own night light instead)"
            );
        }
        let mut backend = Self { conn, queue, state };
        backend.ensure_controls();
        backend.queue.roundtrip(&mut backend.state)?;
        Ok(backend)
    }

    fn ensure_controls(&mut self) {
        let qh = self.queue.handle();
        let Some(manager) = &self.state.manager else {
            return;
        };
        for output in &mut self.state.outputs {
            if output.control.is_none() && !output.failed {
                output.control = Some(manager.get_gamma_control(&output.wl, &qh, output.global));
            }
        }
    }
}

impl Backend for WaylandBackend {
    fn name(&self) -> &'static str {
        "wayland"
    }

    fn apply(&mut self, setting: Setting, gamma: f64) -> Result<()> {
        for output in &self.state.outputs {
            let (Some(control), Some(size)) = (&output.control, output.size) else {
                continue;
            };
            let file = ramp_file(size, setting, gamma)?;
            control.set_gamma(file.as_fd());
        }
        self.conn.flush()?;
        Ok(())
    }

    fn poll(&mut self) -> Result<bool> {
        self.queue
            .roundtrip(&mut self.state)
            .context("lost the Wayland connection")?;
        self.ensure_controls();
        self.queue.roundtrip(&mut self.state)?;
        Ok(std::mem::take(&mut self.state.changed))
    }

    fn restore(&mut self) -> Result<()> {
        for output in &mut self.state.outputs {
            if let Some(control) = output.control.take() {
                control.destroy();
            }
        }
        self.conn.flush()?;
        Ok(())
    }
}

/// The three ramps in an unlinked temp file, for `set_gamma`.
fn ramp_file(size: usize, setting: Setting, gamma: f64) -> Result<File> {
    let path =
        ty_common::runtime_dir().join(format!("tylight-gamma-{}-{}", std::process::id(), size));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let _ = fs::remove_file(&path);
    let bytes: Vec<u8> = ramps(size, setting, gamma)
        .iter()
        .flatten()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    file.write_all(&bytes)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name, interface, ..
            } => {
                if interface == ZwlrGammaControlManagerV1::interface().name {
                    state.manager = Some(registry.bind(name, 1, qh, ()));
                } else if interface == WlOutput::interface().name {
                    state.outputs.push(Output {
                        global: name,
                        wl: registry.bind(name, 1, qh, ()),
                        control: None,
                        size: None,
                        failed: false,
                    });
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.outputs.iter().position(|o| o.global == name) {
                    let output = state.outputs.remove(index);
                    if let Some(control) = output.control {
                        control.destroy();
                    }
                    if output.wl.version() >= 3 {
                        output.wl.release();
                    }
                    state.changed = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlOutput,
        _: <WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrGammaControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrGammaControlManagerV1,
        _: <ZwlrGammaControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrGammaControlV1, u32> for State {
    fn event(
        state: &mut Self,
        control: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.iter_mut().find(|o| o.global == *global) else {
            return;
        };
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                output.size = Some(size as usize);
                state.changed = true;
            }
            zwlr_gamma_control_v1::Event::Failed => {
                eprintln!(
                    "warning: gamma control failed for an output \
                     (is gammastep, wlsunset or hyprsunset running?)"
                );
                control.destroy();
                output.control = None;
                output.size = None;
                output.failed = true;
            }
            _ => {}
        }
    }
}
