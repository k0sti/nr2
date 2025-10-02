# Using EventFlow as a Library

EventFlow can be used as a library in your own Rust projects to route and process Nostr events between relays.

## Adding to Your Project

Add eventflow to your `Cargo.toml`:

```toml
[dependencies]
nostr-eventflow = { path = "path/to/eventflow" }  # Or use git/crates.io when published
```

## Basic Usage

```rust
use eventflow::{Config, ProcessingState, RelayRouter};

#[tokio::main]
async fn main() -> Result<()> {
    // Create configuration
    let config = Config {
        sources: vec!["wss://relay.damus.io".to_string()],
        sinks: vec![/* ... */],
        state_file: "state.json".to_string(),
    };

    // Create or load state
    let state = ProcessingState::new();

    // Create router
    let router = RelayRouter::new(config, state).await?;

    // Connect and stream
    router.connect().await;
    router.stream_events().await?;

    Ok(())
}
```

## Custom Processors

You can create custom processors by implementing the `Processor` trait:

```rust
use eventflow::{Event, Processor};

struct MyCustomProcessor;

impl Processor for MyCustomProcessor {
    fn process(&self, event: &Event) -> Vec<Event> {
        // Your processing logic here
        // Return empty vec to filter out
        // Return vec with event(s) to forward
        vec![event.clone()]
    }
}
```

## Using the Builder Pattern

The `RelayRouterBuilder` provides a flexible way to configure the router:

```rust
use eventflow::RelayRouterBuilder;
use std::sync::Arc;

let router = RelayRouterBuilder::new(config)
    .with_state(state)
    .no_state(true)  // Don't persist state
    .add_processor(
        Arc::new(MyCustomProcessor),
        vec!["wss://my-relay.com".to_string()],
    )
    .build()
    .await?;
```

## Available Processors

EventFlow comes with several built-in processors:

### PassthroughProcessor
Forwards all events unchanged.

```rust
use eventflow::PassthroughProcessor;
```

### GeohashFilterProcessor
Filters events based on geohash tags.

```rust
use eventflow::GeohashFilterProcessor;

let processor = GeohashFilterProcessor::new(vec!["u0".to_string()]);
```

### Custom Processors

You can create processors for:
- **Content filtering**: Filter by event kind, content, tags
- **Content transformation**: Modify events before forwarding
- **Statistics collection**: Gather metrics about events
- **Rate limiting**: Control event flow
- **Deduplication**: Remove duplicate events
- **Aggregation**: Combine multiple events

## Examples

See the `examples/` directory for complete examples:

- `custom_processor.rs`: Demonstrates various custom processor patterns
- `simple_relay_router.rs`: Basic usage of the library

Run examples with:

```bash
cargo run --example custom_processor
cargo run --example simple_relay_router
```

## Advanced Usage

### Accessing Clients Directly

```rust
// Get access to the source client
let source = router.source_client();

// Get access to sink clients and processors
let sinks = router.sink_clients();
```

### Custom Fetch Operations

```rust
use eventflow::Timestamp;

// Fetch events from a specific time range
let start = Timestamp::from(1234567890);
let end = Timestamp::now();

router.fetch_range(start, end, 100, 1).await?;
```

### State Management

```rust
// Get current state
let state = router.get_state().await;
println!("Coverage: {}", state.get_coverage_stats());

// Save state manually
router.save_state().await?;
```

## Thread Safety

All processors must implement `Send + Sync` as they may be called from multiple threads. Use `Arc<Mutex<>>` or atomic types for shared state.

## Error Handling

Most operations return `Result<T, anyhow::Error>`. Handle errors appropriately:

```rust
if let Err(e) = router.stream_events().await {
    eprintln!("Streaming failed: {}", e);
}
```

## Performance Considerations

- Processors are called for every event, so keep processing logic efficient
- Use `Arc` to share processors between multiple sinks
- Consider batching operations when possible
- The router uses channels internally for efficient event flow

## License

MIT