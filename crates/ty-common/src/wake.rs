//! A "something changed" signal from any thread to the GTK thread, so it can sleep instead
//! of polling shared state. Wakes are coalesced: several before the GTK thread gets to run
//! count as one. Handled with [`crate::tray::on_wake`].

/// Cheap to clone and call from any thread. Does nothing once the [`Wakeups`] are dropped.
#[derive(Debug, Clone)]
pub struct Waker(async_channel::Sender<()>);

/// The receiving end of a [`Waker`].
#[derive(Debug)]
pub struct Wakeups(pub(crate) async_channel::Receiver<()>);

pub fn pair() -> (Waker, Wakeups) {
    let (tx, rx) = async_channel::bounded(1);
    (Waker(tx), Wakeups(rx))
}

impl Waker {
    pub fn wake(&self) {
        // Full means a wake is already pending; closed means nobody listens.
        let _ = self.0.try_send(());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wakes_coalesce() {
        let (waker, wakeups) = super::pair();
        waker.wake();
        waker.clone().wake();
        assert!(wakeups.0.try_recv().is_ok());
        assert!(wakeups.0.try_recv().is_err());
        drop(wakeups);
        waker.wake();
    }
}
