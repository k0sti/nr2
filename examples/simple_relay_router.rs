use anyhow::Result;
use eventflow::{Config, ProcessingState, ProcessorType, RelayRouter, SinkConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("eventflow=info")
        .init();

    // Create configuration programmatically
    let mut config = Config {
        sources: vec![
            "wss://relay.damus.io".to_string(),
            "wss://nos.lol".to_string(),
        ],
        sinks: vec![],
        state_file: "example_state.json".to_string(),
    };

    // Add a passthrough sink that forwards all events
    config.sinks.push(SinkConfig {
        relays: vec!["wss://relay.nostr.band".to_string()],
        processor: ProcessorType::Passthrough,
    });

    // Add a geohash filter sink
    config.sinks.push(SinkConfig {
        relays: vec!["wss://nostr.wine".to_string()],
        processor: ProcessorType::GeohashFilter {
            allowed_prefixes: vec!["u0".to_string(), "u1".to_string()],
        },
    });

    // Load or create processing state
    let state_path = PathBuf::from(&config.state_file);
    let state = ProcessingState::load(&state_path).await.unwrap_or_else(|_| {
        println!("Creating new state");
        ProcessingState::new()
    });

    // Create the relay router
    let router = RelayRouter::new(config, state).await?;

    // Connect to all relays
    router.connect().await;

    // Stream events for 30 seconds
    println!("Streaming events for 30 seconds...");

    let stream_handle = tokio::spawn({
        let router_clone = router.clone();
        async move {
            if let Err(e) = router_clone.stream_events().await {
                eprintln!("Stream error: {}", e);
            }
        }
    });

    // Wait for 30 seconds
    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

    // Cancel streaming
    stream_handle.abort();

    // Save state and disconnect
    router.save_state().await?;
    router.disconnect().await?;

    println!("Done!");

    Ok(())
}