use nostr::Event;
use tracing::info;
use crate::processor::Processor;

/// A processor that prints event details for debugging purposes.
///
/// This processor prints key information about each event including:
/// - Event ID
/// - Author (public key)
/// - Event kind
/// - Creation timestamp
/// - Content preview (first 100 characters)
///
/// The event is passed through unchanged.
pub struct PrintProcessor {
    prefix: String,
}

impl PrintProcessor {
    pub fn new() -> Self {
        Self {
            prefix: "[PRINT]".to_string(),
        }
    }

    pub fn with_prefix(prefix: String) -> Self {
        Self { prefix }
    }
}

impl Default for PrintProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor for PrintProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        let content_preview = if event.content.len() > 100 {
            format!("{}...", &event.content[..100])
        } else {
            event.content.clone()
        };

        info!(
            "{} Event: {} | Author: {} | Kind: {} | Time: {} | Content: {}",
            self.prefix,
            event.id.to_hex()[..16].to_string() + "...",
            event.pubkey.to_hex()[..16].to_string() + "...",
            event.kind.as_u16(),
            event.created_at.as_u64(),
            content_preview.replace('\n', " ").replace('\r', "")
        );

        // Pass through the event unchanged
        vec![event.clone()]
    }

    fn name(&self) -> &str {
        "PrintProcessor"
    }
}