use anyhow::Result;
use nostr::prelude::*;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::processor::{create_processor, Processor};

pub struct RelayManager {
    source_client: Client,
    sink_clients: Vec<(Arc<dyn Processor>, Client)>,
}

impl RelayManager {
    pub async fn new(config: Config) -> Result<Self> {
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
            source_client,
            sink_clients,
        })
    }

    pub async fn run(&self) -> Result<()> {
        info!("Connecting to source relays...");
        self.source_client.connect().await;

        info!("Connecting to sink relays...");
        for (_, client) in &self.sink_clients {
            client.connect().await;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let filter = Filter::new();

        info!("Subscribing to events from source relays...");
        self.source_client.subscribe(filter, None).await?;

        let (tx, mut rx) = mpsc::channel::<Event>(1000);

        let source_client = self.source_client.clone();
        tokio::spawn(async move {
            loop {
                match source_client.notifications().recv().await {
                    Ok(notification) => {
                        if let RelayPoolNotification::Event { event, .. } = notification {
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

        while let Some(event) = rx.recv().await {
            info!("Received event: {}", event.id);

            for (processor, client) in &self.sink_clients {
                let processed_events = processor.process(&event);

                if processed_events.is_empty() {
                    info!("Event {} filtered out by processor", event.id);
                } else {
                    for processed_event in processed_events {
                        info!("Forwarding event {} to sink relays", processed_event.id);

                        match client.send_event(&processed_event).await {
                            Ok(output) => {
                                info!("Event sent successfully: {:?}", output);
                            }
                            Err(e) => {
                                warn!("Failed to send event to sink relay: {}", e);
                            }
                        }
                    }
                }
            }
        }

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