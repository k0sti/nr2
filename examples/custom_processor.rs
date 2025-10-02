use anyhow::Result;
use eventflow::{
    Config, Event, ProcessingState, Processor, RelayRouter, RelayRouterBuilder, SubFilter, Timestamp,
    WaitProcessor,
};
use std::sync::Arc;
use tracing::info;

/// Example custom processor that filters events by kind
struct KindFilterProcessor {
    allowed_kinds: Vec<u16>,
}

impl KindFilterProcessor {
    fn new(allowed_kinds: Vec<u16>) -> Self {
        Self { allowed_kinds }
    }
}

impl Processor for KindFilterProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        if self.allowed_kinds.contains(&event.kind.as_u16()) {
            vec![event.clone()]
        } else {
            vec![]
        }
    }
}

/// Example custom processor that modifies event content
struct ContentTransformProcessor {
    prefix: String,
}

impl ContentTransformProcessor {
    fn new(prefix: String) -> Self {
        Self { prefix }
    }
}

impl Processor for ContentTransformProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        // Clone the event and modify its content
        // Note: In a real implementation, you'd need to re-sign the event
        let modified_event = event.clone();
        // For demonstration only - actual modification would require re-signing
        vec![modified_event]
    }
}

/// Example of chaining multiple processors
struct ChainedProcessor {
    processors: Vec<Box<dyn Processor>>,
}

impl ChainedProcessor {
    fn new() -> Self {
        Self {
            processors: Vec::new(),
        }
    }

    fn add_processor(mut self, processor: Box<dyn Processor>) -> Self {
        self.processors.push(processor);
        self
    }
}

impl Processor for ChainedProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        let mut current_events = vec![event.clone()];

        for processor in &self.processors {
            let mut next_events = Vec::new();
            for event in current_events {
                next_events.extend(processor.process(&event));
            }
            current_events = next_events;

            // If any processor filters out all events, stop processing
            if current_events.is_empty() {
                break;
            }
        }

        current_events
    }
}

/// Example of a processor that collects statistics
struct StatsCollectorProcessor {
    event_count: std::sync::atomic::AtomicU64,
    kinds_seen: Arc<std::sync::Mutex<std::collections::HashSet<u16>>>,
}

impl StatsCollectorProcessor {
    fn new() -> Self {
        Self {
            event_count: std::sync::atomic::AtomicU64::new(0),
            kinds_seen: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    fn get_stats(&self) -> (u64, usize) {
        let count = self.event_count.load(std::sync::atomic::Ordering::Relaxed);
        let kinds_count = self.kinds_seen.lock().unwrap().len();
        (count, kinds_count)
    }
}

impl Processor for StatsCollectorProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        self.event_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.kinds_seen.lock().unwrap().insert(event.kind.as_u16());

        // Pass through the event unchanged
        vec![event.clone()]
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Load configuration
    let config = Config::default(); // Or load from file

    // Create processing state
    let state = ProcessingState::new();

    // Example 1: Using a simple custom processor
    {
        info!("Example 1: Simple custom processor");

        // Create a processor that only allows kind 1 (text notes)
        let kind_filter = Arc::new(KindFilterProcessor::new(vec![1]));

        let _router = RelayRouterBuilder::new(config.clone())
            .with_state(state.clone())
            .no_state(true)  // Don't persist state for this example
            .add_processor(
                kind_filter,
                vec!["wss://relay.damus.io".to_string()],
            )
            .build()
            .await?;

        // Use the router...
    }

    // Example 2: Using a chained processor
    {
        info!("Example 2: Chained processor");

        let chained = ChainedProcessor::new()
            .add_processor(Box::new(KindFilterProcessor::new(vec![1, 3, 7])))
            .add_processor(Box::new(ContentTransformProcessor::new("PREFIX:".to_string())));

        let _router = RelayRouterBuilder::new(config.clone())
            .with_state(state.clone())
            .no_state(true)
            .add_processor(
                Arc::new(chained),
                vec!["wss://relay.nostr.band".to_string()],
            )
            .build()
            .await?;

        // Use the router...
    }

    // Example 3: Using a stats collector
    {
        info!("Example 3: Stats collector processor");

        let stats_processor = Arc::new(StatsCollectorProcessor::new());
        let stats_clone = stats_processor.clone();

        let router = RelayRouterBuilder::new(config.clone())
            .with_state(state.clone())
            .no_state(true)
            .add_processor(
                stats_processor,
                vec!["wss://nos.lol".to_string()],
            )
            .build()
            .await?;

        // Connect and stream for a while
        router.connect().await;

        // Simulate streaming (in real use, you'd call router.stream_events())
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        // Get statistics
        let (count, kinds) = stats_clone.get_stats();
        info!("Processed {} events with {} unique kinds", count, kinds);

        router.disconnect().await?;
    }

    // Example 4: Using the WaitProcessor to test slow processing
    {
        info!("Example 4: Testing slow processor with WaitProcessor");

        // Create a WaitProcessor with 2 second delay
        let wait_processor = Arc::new(WaitProcessor::new(2000));

        info!("Creating router with WaitProcessor (2 second delay per event)");
        let _router = RelayRouterBuilder::new(config.clone())
            .with_state(state.clone())
            .no_state(true)
            .add_processor(
                wait_processor,
                vec!["wss://relay.example.com".to_string()],
            )
            .build()
            .await?;

        // In real usage, this would slow down event processing significantly
        info!("Router created with slow processor - each event will be delayed by 2 seconds");
    }

    // Example 5: Using global filters with custom processor
    {
        info!("Example 5: Using global source filters");

        // Create a config with filters at source level
        let mut filtered_config = config.clone();
        filtered_config.filters = Some(vec![
            SubFilter {
                kinds: Some(vec![1, 30023]), // Text notes and long-form content
                ..Default::default()
            }
        ]);

        let text_processor = Arc::new(KindFilterProcessor::new(vec![1]));

        let _router = RelayRouterBuilder::new(filtered_config)
            .with_state(state.clone())
            .no_state(true)
            .add_processor(
                text_processor,
                vec!["wss://relay.example.com".to_string()],
            )
            .build()
            .await?;

        info!("Router created with source filters - only matching events are fetched from sources");
    }

    // Example 6: Direct use as a library with custom fetch range
    {
        info!("Example 6: Custom fetch range");

        let router = RelayRouter::new(config.clone(), state.clone()).await?;

        router.connect().await;

        // Fetch events from the last hour
        let end = Timestamp::now();
        let start = Timestamp::from(end.as_u64() - 3600);

        router.fetch_range(start, end, 100, 1).await?;

        // Get the state to see what was collected
        let final_state = router.get_state().await;
        info!("Collected spans: {}", final_state.get_coverage_stats());

        router.disconnect().await?;
    }

    Ok(())
}