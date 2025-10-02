use nostr::Event;
use std::thread;
use std::time::Duration;
use super::Processor;
use tracing::info;

/// A processor that introduces a delay before passing events through.
/// Useful for testing slow processor behavior and bottlenecks.
#[derive(Debug, Clone)]
pub struct WaitProcessor {
    delay_ms: u64,
}

impl WaitProcessor {
    /// Create a new wait processor with specified delay in milliseconds
    pub fn new(delay_ms: u64) -> Self {
        Self { delay_ms }
    }

    /// Create a wait processor with default 5 second delay
    pub fn default_5s() -> Self {
        Self::new(5000)
    }
}

impl Default for WaitProcessor {
    fn default() -> Self {
        Self::default_5s()
    }
}

impl Processor for WaitProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        info!("Waiting...");

        // Sleep for the configured duration
        thread::sleep(Duration::from_millis(self.delay_ms));

        // Then pass the event through unchanged
        vec![event.clone()]
    }
}