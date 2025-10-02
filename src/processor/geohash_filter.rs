use nostr::Event;
use super::Processor;

/// A processor that filters events based on geohash tags.
/// Only forwards events that have a 'g' tag, optionally filtered by allowed prefixes.
#[derive(Debug, Clone)]
pub struct GeohashFilterProcessor {
    allowed_prefixes: Vec<String>,
}

impl GeohashFilterProcessor {
    /// Create a new geohash filter processor
    ///
    /// # Arguments
    /// * `allowed_prefixes` - If empty, all events with 'g' tag pass through.
    ///                        If non-empty, only events with geohash starting with one of these prefixes pass.
    pub fn new(allowed_prefixes: Vec<String>) -> Self {
        Self { allowed_prefixes }
    }

    /// Check if a geohash value matches the allowed prefixes
    fn matches_prefix(&self, geohash_value: &str) -> bool {
        if self.allowed_prefixes.is_empty() {
            true
        } else {
            self.allowed_prefixes.iter().any(|prefix| {
                geohash_value.starts_with(prefix)
            })
        }
    }
}

impl Processor for GeohashFilterProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        let has_geohash = event.tags.iter().any(|tag| {
            // Check if this is a geohash tag
            if tag.kind().to_string() == "g" {
                if let Some(geohash_value) = tag.content() {
                    return self.matches_prefix(geohash_value);
                }
                // If tag is 'g' but has no content, pass it through if no prefixes are specified
                return self.allowed_prefixes.is_empty();
            }
            false
        });

        if has_geohash {
            vec![event.clone()]
        } else {
            vec![]
        }
    }
}