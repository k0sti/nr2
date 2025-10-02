pub mod config;
pub mod fetcher;
pub mod processor;
pub mod state;
pub mod timespan;

pub use config::{Config, ProcessorType, SinkConfig};
pub use processor::{create_processor, Processor};
pub use state::ProcessingState;
pub use timespan::{TimeSpan, TimeSpanSet};