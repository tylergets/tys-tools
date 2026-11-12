use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::config::Config;

#[derive(Clone, Debug, Serialize)]
pub struct Notification {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub icon: String,
    pub urgency: u8,
    pub timestamp: u64,
    pub actions: Vec<String>,
    #[serde(skip)]
    pub reply_placeholder: String,
    #[serde(skip)]
    pub reply_submit_text: String,
    #[serde(skip)]
    pub reply_submit_icon: String,
    pub progress: Option<i32>,
    #[serde(skip)]
    pub sender: String,
    #[serde(skip)]
    pub tag: String,
    #[serde(skip)]
    pub resident: bool,
    #[serde(skip)]
    pub transient: bool,
    #[serde(skip)]
    pub remaining: Option<u64>,
}

pub struct State {
    pub config: Config,
    pub active: Vec<Notification>,
    pub history: VecDeque<Notification>,
    pub paused: bool,
    pub snoozed_until: Option<SystemTime>,
    pub revision: u64,
    next_id: u32,
}

impl State {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            active: Vec::new(),
            history: VecDeque::new(),
            paused: false,
            snoozed_until: None,
            revision: 0,
            next_id: 1,
        }
    }

    pub fn notify(
        &mut self,
        mut notification: Notification,
        replaces: u32,
    ) -> Result<u32, &'static str> {
        let existing = self.active.iter().position(|n| {
            n.id == replaces
                || (!notification.tag.is_empty()
                    && n.app == notification.app
                    && n.tag == notification.tag)
        });
        if let Some(index) = existing {
            notification.id = self.active[index].id;
            self.active[index] = notification;
            self.revision += 1;
            return Ok(self.active[index].id);
        }
        if self.active.len() >= 512 {
            return Err("notification queue is full");
        }
        loop {
            notification.id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if !self
                .active
                .iter()
                .chain(self.history.iter())
                .any(|n| n.id == notification.id)
            {
                break;
            }
        }
        let id = notification.id;
        self.active.push(notification);
        self.revision += 1;
        Ok(id)
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.snoozed_until = None;
        self.revision += 1;
    }

    pub fn snooze(&mut self, duration: Duration) {
        self.paused = true;
        self.snoozed_until = SystemTime::now().checked_add(duration);
        self.revision += 1;
    }

    pub fn wake_if_due(&mut self, now: SystemTime) {
        if self.snoozed_until.is_some_and(|until| now >= until) {
            self.set_paused(false);
        }
    }

    pub fn snooze_remaining(&self) -> Option<Duration> {
        self.snoozed_until
            .map(|until| until.duration_since(SystemTime::now()).unwrap_or_default())
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
        self.revision += 1;
    }

    pub fn close(&mut self, id: u32) -> Option<Notification> {
        let index = self.active.iter().position(|n| n.id == id)?;
        let notification = self.active.remove(index);
        if !notification.transient && self.config.history_limit > 0 {
            self.history.push_front(notification.clone());
            self.history.truncate(self.config.history_limit);
        }
        self.revision += 1;
        Some(notification)
    }

    pub fn visible(&self) -> Vec<Notification> {
        if self.paused {
            return Vec::new();
        }
        let mut notifications: Vec<_> = self.active.iter().collect();
        notifications.sort_by_key(|n| std::cmp::Reverse(n.urgency));
        notifications.truncate(self.config.limit);
        notifications.into_iter().cloned().collect()
    }

    #[cfg(test)]
    fn tick(&mut self, elapsed_ms: u64) -> Vec<Notification> {
        let visible: Vec<_> = self.visible().iter().map(|n| n.id).collect();
        self.tick_visible(elapsed_ms, &visible)
    }

    /// Only notifications actually placed on the monitor consume their timeout.
    pub fn tick_visible(&mut self, elapsed_ms: u64, visible: &[u32]) -> Vec<Notification> {
        if self.paused {
            return Vec::new();
        }
        let mut expired = Vec::new();
        for n in &mut self.active {
            if visible.contains(&n.id) {
                if let Some(remaining) = &mut n.remaining {
                    *remaining = remaining.saturating_sub(elapsed_ms);
                    if *remaining == 0 {
                        expired.push(n.id);
                    }
                }
            }
        }
        expired
            .into_iter()
            .filter_map(|id| self.close(id))
            .collect()
    }

    pub fn replay(&mut self, id: u32) -> Result<u32, &'static str> {
        let mut n = self
            .history
            .iter()
            .find(|n| id == 0 || n.id == id)
            .cloned()
            .ok_or("notification not in history")?;
        // The original lifecycle has ended: never invoke stale actions or signal its client.
        n.sender.clear();
        n.actions.clear();
        n.tag.clear();
        n.transient = true;
        n.remaining = None;
        self.notify(n, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification() -> Notification {
        Notification {
            id: 0,
            app: "test".into(),
            summary: "hello".into(),
            body: String::new(),
            icon: String::new(),
            urgency: 1,
            timestamp: 0,
            actions: vec!["default".into(), "Open".into()],
            reply_placeholder: String::new(),
            reply_submit_text: String::new(),
            reply_submit_icon: String::new(),
            progress: None,
            sender: ":1.2".into(),
            tag: String::new(),
            resident: false,
            transient: false,
            remaining: Some(100),
        }
    }

    #[test]
    fn screen_overflow_does_not_expire_unshown_notifications() {
        let mut state = State::new(Config::default());
        let first = state.notify(notification(), 0).unwrap();
        let second = state.notify(notification(), 0).unwrap();
        assert_eq!(state.tick_visible(100, &[first])[0].id, first);
        assert_eq!(state.active[0].id, second);
        assert_eq!(state.active[0].remaining, Some(100));
        state.set_paused(true);
        assert!(state.tick_visible(100, &[second]).is_empty());
        assert_eq!(state.active[0].remaining, Some(100));
    }

    #[test]
    fn snooze_resumes_without_consuming_queued_timeouts() {
        let mut state = State::new(Config::default());
        let id = state.notify(notification(), 0).unwrap();
        state.snooze(Duration::from_secs(60));
        let deadline = state.snoozed_until.unwrap();
        assert!(state.visible().is_empty());
        assert!(state.tick(60000).is_empty());
        state.wake_if_due(deadline - Duration::from_secs(1));
        assert!(state.paused);
        state.wake_if_due(deadline);
        assert!(!state.paused);
        assert!(state.snoozed_until.is_none());
        assert_eq!(state.active[0].remaining, Some(100));
        assert_eq!(state.tick(100)[0].id, id);
    }

    #[test]
    fn changing_sleep_or_manually_pausing_cancels_old_deadline() {
        let mut state = State::new(Config::default());
        state.snooze(Duration::from_secs(30));
        let old_deadline = state.snoozed_until.unwrap();
        state.snooze(Duration::from_secs(60));
        state.wake_if_due(old_deadline);
        assert!(state.paused);
        state.set_paused(false);
        assert!(state.snoozed_until.is_none());
        state.snooze(Duration::from_secs(30));
        state.set_paused(true);
        state.wake_if_due(old_deadline + Duration::from_secs(120));
        assert!(state.paused);
        assert!(state.snoozed_until.is_none());
    }

    #[test]
    fn replacement_resets_expiry_without_history() {
        let mut state = State::new(Config::default());
        let id = state.notify(notification(), 0).unwrap();
        state.tick(90);
        assert_eq!(state.notify(notification(), id).unwrap(), id);
        assert!(state.tick(90).is_empty());
        assert_eq!(state.tick(10)[0].id, id);
        assert_eq!(state.history.len(), 1);
        assert!(state.close(id).is_none());
    }

    #[test]
    fn queued_and_paused_notifications_keep_their_timeout() {
        let mut state = State::new(Config {
            limit: 1,
            ..Config::default()
        });
        let first = state.notify(notification(), 0).unwrap();
        let second = state.notify(notification(), 0).unwrap();
        state.paused = true;
        assert!(state.tick(1000).is_empty());
        state.paused = false;
        assert_eq!(state.tick(100)[0].id, first);
        assert_eq!(state.active[0].id, second);
        assert_eq!(state.active[0].remaining, Some(100));
    }

    #[test]
    fn stack_tags_are_app_scoped_and_replay_has_no_stale_actions() {
        let mut state = State::new(Config::default());
        let mut n = notification();
        n.tag = "volume".into();
        let id = state.notify(n.clone(), 0).unwrap();
        assert_eq!(state.notify(n.clone(), 0).unwrap(), id);
        n.app = "other".into();
        assert_ne!(state.notify(n, 0).unwrap(), id);
        state.close(id);
        let replay = state.replay(id).unwrap();
        assert_ne!(replay, id);
        let n = state.active.iter().find(|n| n.id == replay).unwrap();
        assert!(n.sender.is_empty() && n.actions.is_empty());
    }
}
