use nostr::Event;
use crate::config::ProcessorType;

pub trait Processor: Send + Sync {
    fn process(&self, event: &Event) -> Vec<Event>;
}

pub struct PassthroughProcessor;

impl Processor for PassthroughProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        vec![event.clone()]
    }
}

pub struct GeohashFilterProcessor {
    allowed_prefixes: Vec<String>,
}

impl GeohashFilterProcessor {
    pub fn new(allowed_prefixes: Vec<String>) -> Self {
        Self { allowed_prefixes }
    }
}

impl Processor for GeohashFilterProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        let has_geohash = event.tags.iter().any(|tag| {
            // Check if this is a geohash tag using as_standardized method
            if tag.kind().to_string() == "g" {
                if self.allowed_prefixes.is_empty() {
                    return true;
                }
                // Get the geohash value from the tag content
                if let Some(geohash_value) = tag.content() {
                    return self.allowed_prefixes.iter().any(|prefix| {
                        geohash_value.starts_with(prefix)
                    });
                }
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

pub fn create_processor(processor_type: &ProcessorType) -> Box<dyn Processor> {
    match processor_type {
        ProcessorType::Passthrough => Box::new(PassthroughProcessor),
        ProcessorType::GeohashFilter { allowed_prefixes } => {
            Box::new(GeohashFilterProcessor::new(allowed_prefixes.clone()))
        }
    }
}