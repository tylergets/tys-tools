//! Tray icon on its own GTK thread (the main thread runs the rdev key listener). The menu
//! lists the configured snippets; clicking one copies it to the clipboard.

use std::collections::HashMap;

use std::process;

use gtk::glib::ControlFlow;
use ty_common::icons::Tool;
use ty_common::tray::{self, drain_actions, Applet, IconState, MenuEntry, TraySpec};

use crate::config::{MenuSnippet, NotificationConfig, APP_NAME};
use crate::core::expansion::render_template_macros;
use crate::platform::notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    /// Copy `snippets[index]`.
    Copy(usize),
    Quit,
}

pub use ty_common::tray::TrayThread as AppIndicator;

pub fn start(
    snippets: Vec<MenuSnippet>,
    globals: HashMap<String, String>,
    notifications: NotificationConfig,
) -> Option<AppIndicator> {
    ty_common::tray::spawn_gtk_thread(move || {
        let (action_tx, action_rx) = tray::channel();
        let applet = Applet::new(
            APP_NAME,
            &Tool::Key.tray_icons(),
            tray_spec(&snippets),
            action_tx,
        )?;
        drain_actions(action_rx, move |action| {
            match action {
                TrayAction::Copy(index) => {
                    if let Some(snippet) = snippets.get(index) {
                        copy_snippet(snippet, &globals, notifications.on_snippet_copy);
                    }
                }
                TrayAction::Quit => process::exit(0),
            }
            ControlFlow::Continue
        });
        Ok(applet)
    })
}

fn tray_spec(snippets: &[MenuSnippet]) -> TraySpec<TrayAction> {
    let mut menu = vec![MenuEntry::Label("Running".to_string())];
    if !snippets.is_empty() {
        menu.push(MenuEntry::Separator);
        menu.extend(
            snippets
                .iter()
                .enumerate()
                .map(|(index, snippet)| MenuEntry::Action {
                    label: snippet.title.clone(),
                    enabled: true,
                    action: TrayAction::Copy(index),
                }),
        );
        menu.push(MenuEntry::Separator);
    }
    menu.push(MenuEntry::Action {
        label: "Quit".to_string(),
        enabled: true,
        action: TrayAction::Quit,
    });

    TraySpec {
        icon: IconState::Normal,
        title: APP_NAME.to_string(),
        menu,
        click_action: None,
    }
}

fn copy_snippet(snippet: &MenuSnippet, globals: &HashMap<String, String>, notify_on_copy: bool) {
    let text = render_template_macros(&snippet.content, globals).unwrap_or_else(|err| {
        eprintln!("failed to render snippet template macros: {err}");
        snippet.content.clone()
    });
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard.set_text(&text);
    clipboard.store();

    if notify_on_copy {
        notify("Copied Snippet", &snippet.title);
    }
}

#[cfg(test)]
mod tests {
    use super::{tray_spec, TrayAction};
    use crate::config::MenuSnippet;
    use ty_common::tray::MenuEntry;

    fn snippet(title: &str) -> MenuSnippet {
        MenuSnippet {
            title: title.to_string(),
            content: "x".to_string(),
        }
    }

    #[test]
    fn menu_without_snippets_is_status_and_quit() {
        let menu = tray_spec(&[]).menu;
        assert_eq!(menu.len(), 2);
        assert_eq!(menu[0], MenuEntry::Label("Running".to_string()));
        assert!(
            matches!(&menu[1], MenuEntry::Action { label, action: TrayAction::Quit, .. } if label == "Quit")
        );
    }

    #[test]
    fn snippets_sit_between_separators() {
        let menu = tray_spec(&[snippet("Email"), snippet("Phone")]).menu;
        assert_eq!(menu.len(), 6);
        assert_eq!(menu[1], MenuEntry::Separator);
        assert!(
            matches!(&menu[2], MenuEntry::Action { label, action: TrayAction::Copy(0), .. } if label == "Email")
        );
        assert!(
            matches!(&menu[3], MenuEntry::Action { label, action: TrayAction::Copy(1), .. } if label == "Phone")
        );
        assert_eq!(menu[4], MenuEntry::Separator);
    }
}
