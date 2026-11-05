use crate::clipboard::backend::ClipboardBackend;
use crate::clipboard::secrets::SecretFilter;
use crate::clipboard::ClipboardEntry;
use crate::core::active_window::ActiveWindowContext;

pub struct ClipboardPoller<B: ClipboardBackend> {
    backend: B,
    last_seen_value: Option<ClipboardEntry>,
    active_window_blacklist: Vec<String>,
    secret_filter: SecretFilter,
}

impl<B: ClipboardBackend> ClipboardPoller<B> {
    pub fn new(
        backend: B,
        active_window_blacklist: Vec<String>,
        secret_filter: SecretFilter,
    ) -> Self {
        Self {
            backend,
            last_seen_value: None,
            active_window_blacklist: normalized_blacklist(active_window_blacklist),
            secret_filter,
        }
    }

    pub fn poll_once(&mut self) -> Option<ClipboardEntry> {
        let value = self.backend.read_entry()?;
        if value.is_empty() {
            return None;
        }

        if self.last_seen_value.as_ref() == Some(&value) {
            return None;
        }

        self.last_seen_value = Some(value.clone());
        if let ClipboardEntry::Text { value: text, .. } = &value {
            if self.secret_filter.is_secret(text) {
                println!("clipboard event: secret skipped");
                return None;
            }
        }
        let active_window = self.backend.read_active_window();
        if should_skip_for_blacklisted_window(active_window.as_ref(), &self.active_window_blacklist)
        {
            return None;
        }
        Some(value.with_source_window(active_window))
    }
}

fn normalized_blacklist(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn should_skip_for_blacklisted_window(
    active_window: Option<&ActiveWindowContext>,
    blacklist: &[String],
) -> bool {
    if blacklist.is_empty() {
        return false;
    }

    let Some(active_window) = active_window else {
        return false;
    };

    let app_id = active_window
        .app_id
        .as_ref()
        .map(|value| value.trim().to_lowercase());
    let title = active_window.title.trim().to_lowercase();
    blacklist.iter().any(|blocked| {
        app_id.as_ref().is_some_and(|value| value == blocked) || title.contains(blocked)
    })
}

#[cfg(test)]
mod tests {
    use super::ClipboardPoller;
    use crate::clipboard::backend::ClipboardBackend;
    use crate::clipboard::secrets::SecretFilter;
    use crate::clipboard::ClipboardEntry;
    use crate::config::SecretsConfig;
    use crate::core::active_window::ActiveWindowContext;
    use std::cell::RefCell;

    struct MockBackend {
        entries: RefCell<Vec<Option<ClipboardEntry>>>,
        active_windows: RefCell<Vec<Option<ActiveWindowContext>>>,
    }

    impl MockBackend {
        fn new(
            entries: Vec<Option<ClipboardEntry>>,
            active_windows: Vec<Option<ActiveWindowContext>>,
        ) -> Self {
            Self {
                entries: RefCell::new(entries),
                active_windows: RefCell::new(active_windows),
            }
        }
    }

    impl ClipboardBackend for MockBackend {
        fn read_entry(&self) -> Option<ClipboardEntry> {
            self.entries.borrow_mut().remove(0)
        }

        fn read_active_window(&self) -> Option<ActiveWindowContext> {
            self.active_windows.borrow_mut().remove(0)
        }
    }

    fn text(value: &str) -> ClipboardEntry {
        ClipboardEntry::Text {
            value: value.to_string(),
            source_window: None,
        }
    }

    #[test]
    fn skips_entry_when_app_id_is_blacklisted() {
        let backend = MockBackend::new(
            vec![Some(text("secret"))],
            vec![Some(ActiveWindowContext {
                backend: "hyprctl".to_string(),
                title: "KeePassXC".to_string(),
                app_id: Some("keepassxc".to_string()),
                initial_app_id: None,
                initial_title: None,
                window_id: None,
                pid: None,
                workspace_id: None,
                workspace_name: None,
                is_xwayland: None,
            })],
        );
        let mut poller = ClipboardPoller::new(
            backend,
            vec!["KeePassXC".to_string()],
            SecretFilter::default(),
        );

        let entry = poller.poll_once();
        assert!(entry.is_none(), "blacklisted app id should be skipped");
    }

    #[test]
    fn skips_entry_when_title_contains_blacklisted_value() {
        let backend = MockBackend::new(
            vec![Some(text("token"))],
            vec![Some(ActiveWindowContext {
                backend: "xdotool".to_string(),
                title: "Slack | direct messages".to_string(),
                app_id: None,
                initial_app_id: None,
                initial_title: None,
                window_id: None,
                pid: None,
                workspace_id: None,
                workspace_name: None,
                is_xwayland: None,
            })],
        );
        let mut poller =
            ClipboardPoller::new(backend, vec!["slack".to_string()], SecretFilter::default());

        let entry = poller.poll_once();
        assert!(entry.is_none(), "blacklisted title should be skipped");
    }

    #[test]
    fn accepts_entry_when_window_not_blacklisted() {
        let backend = MockBackend::new(
            vec![Some(text("hello"))],
            vec![Some(ActiveWindowContext {
                backend: "hyprctl".to_string(),
                title: "Terminal".to_string(),
                app_id: Some("kitty".to_string()),
                initial_app_id: None,
                initial_title: None,
                window_id: None,
                pid: None,
                workspace_id: None,
                workspace_name: None,
                is_xwayland: None,
            })],
        );
        let mut poller =
            ClipboardPoller::new(backend, vec!["slack".to_string()], SecretFilter::default());

        let entry = poller.poll_once();
        assert!(entry.is_some(), "non-blacklisted window should be captured");
    }

    #[test]
    fn skips_secret_text() {
        let backend = MockBackend::new(
            vec![
                Some(text("ghp_0123456789abcdefghijABCDEFGHIJ012345")),
                Some(text("hello")),
            ],
            vec![None],
        );
        let filter = SecretFilter::from_config(&SecretsConfig::default()).unwrap();
        let mut poller = ClipboardPoller::new(backend, Vec::new(), filter);

        assert!(poller.poll_once().is_none(), "secret should be skipped");
        assert!(
            poller.poll_once().is_some(),
            "ordinary text should be captured"
        );
    }
}
