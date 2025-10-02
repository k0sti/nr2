use nostr::Event;
use super::Processor;

/// A processor that passes through all events unchanged.
/// This is the simplest possible processor implementation.
#[derive(Debug, Clone)]
pub struct PassthroughProcessor;

impl PassthroughProcessor {
    /// Create a new passthrough processor
    pub fn new() -> Self {
        Self
    }
}

impl Default for PassthroughProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor for PassthroughProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        vec![event.clone()]
    }
}