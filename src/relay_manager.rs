use anyhow::Result;
use nostr::prelude::*;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::processor::{create_processor, Processor};
use crate::state::ProcessingState;

pub struct RelayManager {
    config: Config,
    source_client: Client,
    sink_clients: Vec<(Arc<dyn Processor>, Client)>,
    state: Arc<Mutex<ProcessingState>>,
}

impl RelayManager {
    pub async fn new(config: Config, state: ProcessingState) -> Result<Self> {
        let source_client = Client::new(Keys::generate());

        for relay_url in &config.sources {
            info!("Adding source relay: {}", relay_url);
            source_client.add_relay(relay_url).await?;
        }

        let mut sink_clients = Vec::new();
        for sink_config in &config.sinks {
            let processor = create_processor(&sink_config.processor);
            let client = Client::new(Keys::generate());

            for relay_url in &sink_config.relays {
                info!("Adding sink relay: {}", relay_url);
                client.add_relay(relay_url).await?;
            }

            sink_clients.push((Arc::from(processor), client));
        }

        Ok(Self {
            config,
            source_client,
            sink_clients,
            state: Arc::new(Mutex::new(state)),
        })
    }

    pub async fn run(&self) -> Result<()> {
        info!("Connecting to relays...");
        self.connect_relays().await;

        // Start streaming from current time
        self.stream_events().await
    }

    async fn connect_relays(&self) {
        info!("Connecting to source relays...");
        self.source_client.connect().await;

        info!("Connecting to sink relays...");
        for (_, client) in &self.sink_clients {
            client.connect().await;
        }

        // Give relays time to connect
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    /// Stream events from current time onwards
    async fn stream_events(&self) -> Result<()> {
        let start_time = Timestamp::now();

        // Start session
        {
            let mut state = self.state.lock().await;
            state.start_session(start_time);
        }

        // Create filter for streaming from now
        let filter = Filter::new().since(start_time);

        info!("Starting live stream from {}", start_time);
        let subscription = self.source_client.subscribe(filter, None).await?;
        info!("Subscription created: {:?}", subscription);

        // Setup periodic state saving
        let state_clone = self.state.clone();
        let state_file = self.config.state_file.clone();
        let save_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let mut state = state_clone.lock().await;
                state.checkpoint_session();
                if let Err(e) = state.save(&state_file).await {
                    warn!("Failed to save state: {}", e);
                }
                info!("State checkpoint: {}", state.get_coverage_stats());
            }
        });

        // Process events
        let (tx, mut rx) = mpsc::channel::<Event>(1000);

        let source_client = self.source_client.clone();
        let state_clone = self.state.clone();

        tokio::spawn(async move {
            loop {
                match source_client.notifications().recv().await {
                    Ok(notification) => {
                        if let RelayPoolNotification::Event { event, .. } = notification {
                            // Update state
                            {
                                let mut state = state_clone.lock().await;
                                state.process_event(event.created_at);
                            }

                            if let Err(e) = tx.send(*event).await {
                                error!("Failed to send event to processor: {}", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error receiving notification: {}", e);
                        break;
                    }
                }
            }
        });

        // Process received events
        while let Some(event) = rx.recv().await {
            debug!("Processing event: {} at {}", event.id, event.created_at);

            for (processor, client) in &self.sink_clients {
                let processed_events = processor.process(&event);

                if processed_events.is_empty() {
                    debug!("Event {} filtered out by processor", event.id);
                } else {
                    for processed_event in processed_events {
                        debug!("Forwarding event {} to sink relays", processed_event.id);

                        match client.send_event(&processed_event).await {
                            Ok(_) => {
                                debug!("Event sent successfully");
                            }
                            Err(e) => {
                                warn!("Failed to send event to sink relay: {}", e);
                            }
                        }
                    }
                }
            }
        }

        save_handle.abort();

        // End session and save final state
        {
            let mut state = self.state.lock().await;
            state.end_session();
            state.save(&self.config.state_file).await?;
            info!("Final state: {}", state.get_coverage_stats());
        }

        Ok(())
    }

    pub async fn save_state(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.end_session();
        state.save(&self.config.state_file).await?;
        info!("State saved: {}", state.get_coverage_stats());
        Ok(())
    }

    pub async fn disconnect(&self) -> Result<()> {
        info!("Disconnecting from source relays...");
        self.source_client.disconnect().await;

        info!("Disconnecting from sink relays...");
        for (_, client) in &self.sink_clients {
            client.disconnect().await;
        }

        Ok(())
    }
}