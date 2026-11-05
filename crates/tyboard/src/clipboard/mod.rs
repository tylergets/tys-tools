pub mod backend;
pub mod poller;
pub mod state;
pub mod storage;

pub use state::{ClipboardEntry, SharedClipboardState, DEFAULT_HISTORY_LIMIT};
