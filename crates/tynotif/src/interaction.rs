use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

use gtk::{gio, prelude::*};

use crate::{
    bus::{self, Shared},
    config::MouseAction,
    content,
    model::Notification,
};

#[derive(Clone)]
enum Target {
    Action(String),
    Url(String),
}

#[derive(Clone)]
struct Choice {
    label: String,
    notification: Notification,
    target: Target,
}

struct Pending {
    receiver: Receiver<Option<usize>>,
    choices: Vec<Choice>,
    continuation: (Option<u32>, Vec<MouseAction>),
}

#[derive(Default)]
pub struct Interaction {
    menu: Option<gtk::Menu>,
    pending: Option<Pending>,
}

pub fn default_action<'a>(n: &'a Notification, name: &str) -> Option<&'a str> {
    let mut keys = n
        .actions
        .chunks_exact(2)
        .map(|p| p[0].as_str())
        .filter(|k| *k != "inline-reply");
    let first = keys.next()?;
    if n.actions
        .chunks_exact(2)
        .any(|p| p[0] == name && p[0] != "inline-reply")
    {
        return Some(name_in_notification(n, name));
    }
    if keys.next().is_none() {
        Some(first)
    } else {
        None
    }
}

fn name_in_notification<'a>(n: &'a Notification, name: &str) -> &'a str {
    n.actions.chunks_exact(2).find(|p| p[0] == name).unwrap()[0].as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_action_prefers_named_action_then_only_action() {
        let mut n = crate::model::tests::notification();
        n.actions = vec![
            "open".into(),
            "Open".into(),
            "ignore".into(),
            "Ignore".into(),
        ];
        assert_eq!(default_action(&n, "default"), None);
        assert_eq!(default_action(&n, "open"), Some("open"));
        n.actions.truncate(2);
        assert_eq!(default_action(&n, "default"), Some("open"));
        n.actions = vec!["inline-reply".into(), "Reply".into()];
        assert_eq!(default_action(&n, "default"), None);
    }
}

fn choices(notifications: &[Notification]) -> Vec<Choice> {
    let mut out = Vec::new();
    for n in notifications {
        for action in n.actions.chunks_exact(2).filter(|p| p[0] != "inline-reply") {
            out.push(Choice {
                label: format!("{} — {}: {}", n.app, content::plain(&n.summary), action[1]),
                notification: n.clone(),
                target: Target::Action(action[0].clone()),
            });
        }
        for url in content::urls(n) {
            out.push(Choice {
                label: format!("{} — {}", n.app, url),
                notification: n.clone(),
                target: Target::Url(url),
            });
        }
    }
    out
}

fn select(choice: &Choice, state: &Shared, connection: &gio::DBusConnection) -> bool {
    let mut model = state.lock().unwrap();
    // A menu may outlive its source or a replacement. Never invoke a stale action.
    if !model
        .active
        .iter()
        .any(|n| n.id == choice.notification.id && n.generation == choice.notification.generation)
    {
        return false;
    }
    match &choice.target {
        Target::Action(key) => {
            if let Err(error) = bus::action(connection, &mut model, choice.notification.id, key) {
                eprintln!("{error}");
                return false;
            }
        }
        Target::Url(url) => {
            let argv = model.config.browser.clone();
            let url = url.clone();
            drop(model);
            std::thread::spawn(move || {
                match Command::new(&argv[0]).args(&argv[1..]).arg(url).status() {
                    Ok(status) if status.success() => {}
                    Ok(status) => eprintln!("URL opener exited with {status}"),
                    Err(error) => eprintln!("URL opener: {error}"),
                }
            });
        }
    }
    true
}

impl Interaction {
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.menu.as_ref().is_some_and(|m| m.is_visible())
    }

    pub fn poll(
        &mut self,
        state: &Shared,
        connection: &gio::DBusConnection,
        presented: &[u32],
        parent: Option<&gtk::Window>,
    ) {
        if let Some(pending) = &self.pending {
            match pending.receiver.try_recv() {
                Ok(index) => {
                    let pending = self.pending.take().unwrap();
                    if let Some(choice) = index.and_then(|i| pending.choices.get(i)) {
                        if select(choice, state, connection) {
                            state
                                .lock()
                                .unwrap()
                                .interactions
                                .push_front(pending.continuation);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.busy() {
            return;
        }
        if let Some(menu) = self.menu.take() {
            if menu.attach_widget().is_some() {
                menu.detach();
            }
        }
        // Bound each tick's work; UI and D-Bus remain responsive.
        for _ in 0..64 {
            let Some((id, actions)) = state.lock().unwrap().interactions.pop_front() else {
                break;
            };
            self.execute(state, connection, presented, parent, id, actions);
            if self.busy() {
                break;
            }
        }
    }

    fn execute(
        &mut self,
        state: &Shared,
        connection: &gio::DBusConnection,
        presented: &[u32],
        parent: Option<&gtk::Window>,
        id: Option<u32>,
        actions: Vec<MouseAction>,
    ) {
        let id = id.or_else(|| presented.first().copied());
        for (index, action) in actions.iter().enumerate() {
            let mut model = state.lock().unwrap();
            let n = id
                .and_then(|id| model.active.iter().find(|n| n.id == id))
                .cloned();
            let menu = match action {
                MouseAction::None => continue,
                MouseAction::CloseAll => {
                    bus::close_all(connection, &mut model);
                    continue;
                }
                MouseAction::CloseCurrent => {
                    if let Some(id) = id {
                        bus::close(connection, &mut model, id, 2);
                    }
                    continue;
                }
                MouseAction::DoAction => {
                    let Some(n) = n else {
                        continue;
                    };
                    if let Some(key) = default_action(&n, &model.config.action_name) {
                        if let Err(error) = bus::action(connection, &mut model, n.id, key) {
                            eprintln!("{error}");
                        }
                        continue;
                    }
                    let choices = choices(&[n]);
                    if choices.len() == 1 && matches!(choices[0].target, Target::Url(_)) {
                        drop(model);
                        select(&choices[0], state, connection);
                        continue;
                    }
                    choices
                }
                MouseAction::OpenUrl => {
                    let Some(n) = n else {
                        continue;
                    };
                    let urls = content::urls(&n);
                    if urls.len() == 1 {
                        let choice = Choice {
                            label: urls[0].clone(),
                            notification: n,
                            target: Target::Url(urls[0].clone()),
                        };
                        drop(model);
                        select(&choice, state, connection);
                        continue;
                    }
                    choices(&[n])
                        .into_iter()
                        .filter(|c| matches!(c.target, Target::Url(_)))
                        .collect()
                }
                MouseAction::Context => choices(&n.into_iter().collect::<Vec<_>>()),
                MouseAction::ContextAll => choices(
                    &presented
                        .iter()
                        .filter_map(|id| model.active.iter().find(|n| n.id == *id).cloned())
                        .collect::<Vec<_>>(),
                ),
            };
            let command = model.config.dmenu.clone();
            drop(model);
            if !menu.is_empty() {
                self.show_menu(
                    menu,
                    command,
                    (id, actions[index + 1..].to_vec()),
                    state,
                    connection,
                    parent,
                );
                return;
            }
        }
    }

    fn show_menu(
        &mut self,
        choices: Vec<Choice>,
        command: Vec<String>,
        continuation: (Option<u32>, Vec<MouseAction>),
        state: &Shared,
        connection: &gio::DBusConnection,
        parent: Option<&gtk::Window>,
    ) {
        if command.is_empty() {
            let menu = gtk::Menu::new();
            for choice in choices {
                let item = gtk::MenuItem::with_label(&choice.label);
                let state = state.clone();
                let connection = connection.clone();
                let continuation = continuation.clone();
                item.connect_activate(move |_| {
                    if select(&choice, &state, &connection) {
                        state
                            .lock()
                            .unwrap()
                            .interactions
                            .push_front(continuation.clone());
                    }
                });
                menu.append(&item);
            }
            menu.show_all();
            if let Some(parent) = parent {
                menu.set_attach_widget(Some(parent));
                menu.popup_at_widget(
                    parent,
                    gtk::gdk::Gravity::SouthEast,
                    gtk::gdk::Gravity::NorthEast,
                    None,
                );
            } else {
                menu.popup_easy(0, gtk::current_event_time());
            }
            self.menu = Some(menu);
        } else {
            let rows = choices
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{}: {}", i + 1, c.label.replace(['\n', '\r', '\t'], " ")))
                .collect::<Vec<_>>();
            let (tx, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let run = || -> anyhow::Result<Option<usize>> {
                    let mut child = Command::new(&command[0])
                        .args(&command[1..])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .spawn()?;
                    // Write on a separate thread to avoid a pipe deadlock with a verbose picker.
                    let mut input = child.stdin.take().unwrap();
                    let text = rows.join("\n") + "\n";
                    let writer = std::thread::spawn(move || input.write_all(text.as_bytes()));
                    let output = child.wait_with_output()?;
                    let _ = writer.join();
                    if !output.status.success() {
                        return Ok(None);
                    }
                    let selected = String::from_utf8(output.stdout)?;
                    Ok(rows
                        .iter()
                        .position(|row| row == selected.trim_end_matches(['\n', '\r'])))
                };
                let selected = match run() {
                    Ok(value) => value,
                    Err(error) => {
                        eprintln!("Notification menu: {error:#}");
                        None
                    }
                };
                let _ = tx.send(selected);
            });
            self.pending = Some(Pending {
                receiver,
                choices,
                continuation,
            });
        }
    }
}
