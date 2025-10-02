pub mod config;
pub mod fetcher;
pub mod processor;
pub mod relay_router;
pub mod state;
pub mod timespan;

// Re-export commonly used types for convenience
pub use config::{Config, SubFilter, ProcessorType, SinkConfig};
pub use fetcher::{Fetcher, FetchResult};
pub use processor::{
    create_processor,
    GeohashFilterProcessor,
    PassthroughProcessor,
    WaitProcessor,
    Processor
};
pub use relay_router::{RelayRouter, RelayRouterBuilder};
pub use state::{ProcessingState, ProcessingStats};
pub use timespan::{TimeSpan, TimeSpanSet};

// Re-export nostr types that are commonly needed when implementing processors
pub use nostr::{Event, Timestamp};

// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");