//! Event processing pipeline for filtering and transforming Nostr events.
//!
//! This module provides the `Processor` trait and built-in implementations
//! for common processing patterns. Custom processors can be created by
//! implementing the `Processor` trait.

use nostr::Event;
use crate::config::ProcessorType;

// Re-export processor implementations
mod passthrough;
mod geohash_filter;
mod wait;

pub use passthrough::PassthroughProcessor;
pub use geohash_filter::GeohashFilterProcessor;
pub use wait::WaitProcessor;

/// Trait for processing Nostr events.
///
/// Processors can filter, transform, or route events. They must be thread-safe
/// (`Send + Sync`) as they may be called from multiple threads.
///
/// # Examples
///
/// ## Simple filter processor
/// ```rust
/// use nostr::Event;
/// use eventflow::Processor;
///
/// struct KindFilterProcessor {
///     allowed_kinds: Vec<u64>,
/// }
///
/// impl Processor for KindFilterProcessor {
///     fn process(&self, event: &Event) -> Vec<Event> {
///         if self.allowed_kinds.contains(&event.kind.as_u64()) {
///             vec![event.clone()]
///         } else {
///             vec![]
///         }
///     }
/// }
/// ```
///
/// ## Transform processor
/// ```rust
/// use nostr::Event;
/// use eventflow::Processor;
///
/// struct EventEnricherProcessor;
///
/// impl Processor for EventEnricherProcessor {
///     fn process(&self, event: &Event) -> Vec<Event> {
///         let mut enriched = event.clone();
///         // Add custom processing logic here
///         vec![enriched]
///     }
/// }
/// ```
///
/// ## One-to-many processor
/// ```rust
/// use nostr::Event;
/// use eventflow::Processor;
///
/// struct EventSplitterProcessor;
///
/// impl Processor for EventSplitterProcessor {
///     fn process(&self, event: &Event) -> Vec<Event> {
///         // Split event into multiple events based on some criteria
///         vec![event.clone(), event.clone()]
///     }
/// }
/// ```
pub trait Processor: Send + Sync {
    /// Process a single event and return zero or more events.
    ///
    /// # Arguments
    /// * `event` - The event to process
    ///
    /// # Returns
    /// * Empty vector to filter out the event
    /// * Single event vector to pass through (possibly transformed)
    /// * Multiple events for one-to-many transformations
    fn process(&self, event: &Event) -> Vec<Event>;

    /// Optional method to get processor name for debugging/logging.
    /// Default implementation returns the type name.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }

    /// Optional method to check if the processor is ready.
    /// Can be used for processors that need initialization or external resources.
    /// Default implementation always returns true.
    fn is_ready(&self) -> bool {
        true
    }

    /// Optional method for cleanup when the processor is being shut down.
    /// Default implementation does nothing.
    fn shutdown(&self) {
        // No-op by default
    }
}

/// Create a processor from a configuration type.
///
/// This is primarily used for built-in processors defined in the config.
/// For custom processors, create them directly and box them.
pub fn create_processor(processor_type: &ProcessorType) -> Box<dyn Processor> {
    match processor_type {
        ProcessorType::Passthrough => Box::new(PassthroughProcessor::new()),
        ProcessorType::WaitProcessor => Box::new(WaitProcessor::default_5s()),
        ProcessorType::GeohashFilter { allowed_prefixes } => {
            Box::new(GeohashFilterProcessor::new(allowed_prefixes.clone()))
        }
    }
}