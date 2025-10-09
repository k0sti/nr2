use anyhow::Result;
use nostr::prelude::*;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use crate::config::{Config, SubFilter};
use crate::fetcher::Fetcher;
use crate::processor::Processor;
use crate::state::ProcessingState;

/// Convert our SubFilter to Nostr SDK Filter
pub fn convert_filters_to_nostr(filters: &[SubFilter], since: Option<Timestamp>) -> Vec<Filter> {
    if filters.is_empty() {
        // No filters specified, return a basic filter with just the since timestamp
        if let Some(since) = since {
            return vec![Filter::new().since(since)];
        } else {
            return vec![Filter::new()];
        }
    }

    filters.iter().map(|sub_filter| {
        let mut filter = Filter::new();

        if let Some(since) = since {
            filter = filter.since(since);
        }

        if let Some(ref authors) = sub_filter.authors {
            let pubkeys: Vec<PublicKey> = authors.iter()
                .filter_map(|hex| PublicKey::from_hex(hex).ok())
                .collect();
            filter = filter.authors(pubkeys);
        }

        if let Some(ref kinds) = sub_filter.kinds {
            let kinds_converted: Vec<Kind> = kinds.iter()
                .map(|k| Kind::from(*k))
                .collect();
            filter = filter.kinds(kinds_converted);
        }

        // Handle tag filters
        for (tag_name, values) in &sub_filter.tags {
            let tag_key = if tag_name.starts_with('#') {
                &tag_name[1..]
            } else {
                tag_name
            };

            if tag_key.len() == 1 {
                let tag_char = tag_key.chars().next().unwrap();
                if let Ok(single_letter_tag) = SingleLetterTag::from_char(tag_char) {
                    filter = filter.custom_tags(single_letter_tag, values.clone());
                }
            }
        }

        filter
    }).collect()
}

/// Check if an event matches any of the provided filters
pub fn event_matches_filters(event: &Event, filters: &[SubFilter]) -> bool {
    // If no filters specified, everything passes
    if filters.is_empty() {
        return true;
    }

    // Check if event matches ANY filter (OR logic)
    for filter in filters {
        if matches_single_filter(event, filter) {
            return true;
        }
    }

    false
}

/// Check if an event matches a single filter (AND logic for all conditions)
pub fn matches_single_filter(event: &Event, filter: &SubFilter) -> bool {
    // Check authors
    if let Some(ref authors) = filter.authors {
        if !authors.contains(&event.pubkey.to_hex()) {
            return false;
        }
    }

    // Check kinds
    if let Some(ref kinds) = filter.kinds {
        if !kinds.contains(&event.kind.as_u16()) {
            return false;
        }
    }

    // Check tag filters
    for (tag_name, filter_values) in &filter.tags {
        // Handle both "#e" and "e" format
        let tag_key = if tag_name.starts_with('#') {
            &tag_name[1..]
        } else {
            tag_name
        };

        // Get all values for this tag from the event
        let event_tag_values: Vec<String> = event.tags
            .iter()
            .filter(|tag| tag.kind().to_string() == tag_key)
            .filter_map(|tag| tag.content().map(|s| s.to_string()))
            .collect();

        // At least one event tag value must match one filter value
        if event_tag_values.is_empty() ||
           !event_tag_values.iter().any(|ev| filter_values.contains(ev)) {
            return false;
        }
    }

    true
}

/// A builder for creating a RelayRouter with custom processors
pub struct RelayRouterBuilder {
    config: Config,
    state: Option<ProcessingState>,
    no_state: bool,
    custom_processors: Vec<(Arc<dyn Processor>, Vec<String>)>,
}

impl RelayRouterBuilder {
    /// Create a new builder with the given configuration
    pub fn new(config: Config) -> Self {
        Self {
            config,
            state: None,
            no_state: false,
            custom_processors: Vec::new(),
        }
    }

    /// Set the processing state
    pub fn with_state(mut self, state: ProcessingState) -> Self {
        self.state = Some(state);
        self
    }

    /// Disable state persistence
    pub fn no_state(mut self, no_state: bool) -> Self {
        self.no_state = no_state;
        self
    }

    /// Add a custom processor with its target relay URLs
    pub fn add_processor(mut self, processor: Arc<dyn Processor>, relay_urls: Vec<String>) -> Self {
        self.custom_processors.push((processor, relay_urls));
        self
    }

    /// Build the RelayRouter
    pub async fn build(self) -> Result<RelayRouter> {
        let state = self.state.unwrap_or_else(ProcessingState::new);
        RelayRouter::new_with_custom_processors(
            self.config,
            state,
            self.no_state,
            self.custom_processors,
        ).await
    }
}

/// Main relay routing component that can be used as a library
#[derive(Clone)]
pub struct RelayRouter {
    config: Config,
    source_client: Client,
    sink_clients: Vec<(Arc<dyn Processor>, Client)>,
    state: Arc<Mutex<ProcessingState>>,
    no_state: bool,
}

impl RelayRouter {
    /// Create a new RelayRouter with the default configuration
    pub async fn new(config: Config, state: ProcessingState) -> Result<Self> {
        Self::new_with_options(config, state, false).await
    }

    /// Create a new RelayRouter with options
    pub async fn new_with_options(
        config: Config,
        state: ProcessingState,
        no_state: bool,
    ) -> Result<Self> {
        Self::new_with_custom_processors(config, state, no_state, Vec::new()).await
    }

    /// Create a new RelayRouter with custom processors
    pub async fn new_with_custom_processors(
        config: Config,
        state: ProcessingState,
        no_state: bool,
        custom_processors: Vec<(Arc<dyn Processor>, Vec<String>)>,
    ) -> Result<Self> {
        let source_client = Client::new(Keys::generate());

        for relay_url in &config.sources {
            info!("Adding source relay: {}", relay_url);
            source_client.add_relay(relay_url).await?;
        }

        let mut sink_clients = Vec::new();

        // Add processors from config
        for sink_config in &config.sinks {
            let processor = crate::processor::create_processor(&sink_config.processor);
            let client = Client::new(Keys::generate());

            for relay_url in &sink_config.relays {
                info!("Adding sink relay: {}", relay_url);
                client.add_relay(relay_url).await?;
            }

            sink_clients.push((Arc::from(processor), client));
        }

        // Add custom processors
        for (processor, relay_urls) in custom_processors {
            let client = Client::new(Keys::generate());

            for relay_url in relay_urls {
                info!("Adding custom sink relay: {}", relay_url);
                client.add_relay(&relay_url).await?;
            }

            sink_clients.push((processor, client));
        }

        Ok(Self {
            config,
            source_client,
            sink_clients,
            state: Arc::new(Mutex::new(state)),
            no_state,
        })
    }

    /// Create a builder for more flexible configuration
    pub fn builder(config: Config) -> RelayRouterBuilder {
        RelayRouterBuilder::new(config)
    }

    /// Connect to all configured relays
    pub async fn connect(&self) {
        info!("Connecting to source relays...");
        self.source_client.connect().await;

        info!("Connecting to sink relays...");
        for (_, client) in &self.sink_clients {
            client.connect().await;
        }

        // Give relays time to connect
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    /// Disconnect from all relays
    pub async fn disconnect(&self) -> Result<()> {
        info!("Disconnecting from source relays...");
        self.source_client.disconnect().await;

        info!("Disconnecting from sink relays...");
        for (_, client) in &self.sink_clients {
            client.disconnect().await;
        }

        Ok(())
    }

    /// Stream events from current time onwards
    pub async fn stream_events(&self) -> Result<()> {
        let start_time = Timestamp::now();

        // Start session
        {
            let mut state = self.state.lock().await;
            state.start_session(start_time);
        }

        // Create filters for subscription based on config
        let filters = if let Some(ref sub_filters) = self.config.filters {
            convert_filters_to_nostr(sub_filters, Some(start_time))
        } else {
            vec![Filter::new().since(start_time)]
        };

        info!("[STREAM] Starting live stream from {} with {} filter(s)", start_time, filters.len());

        // Subscribe with multiple filters (OR logic between them)
        let mut subscriptions = Vec::new();
        for filter in filters {
            let sub = self.source_client.subscribe(filter, None).await?;
            subscriptions.push(sub);
        }
        info!("[STREAM] Created {} subscription(s)", subscriptions.len());

        // Setup periodic state saving
        let state_clone = self.state.clone();
        let state_file = self.config.state_file.clone();
        let no_state = self.no_state;
        let save_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let mut state = state_clone.lock().await;
                state.checkpoint_session();
                if !no_state {
                    if let Err(e) = state.save(&state_file).await {
                        warn!("[STREAM] Failed to save state: {}", e);
                    }
                }
                info!("[STREAM] State checkpoint: {}", state.get_coverage_stats());
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
                                error!("[STREAM] Failed to send event to processor: {}", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        error!("[STREAM] Error receiving notification: {}", e);
                        break;
                    }
                }
            }
        });

        // Process received events
        while let Some(event) = rx.recv().await {
            debug!("[STREAM] Processing event: {} at {}", event.id, event.created_at);

            for (processor, client) in &self.sink_clients {
                let processed_events = processor.process(&event);

                if processed_events.is_empty() {
                    debug!("[STREAM] Event {} filtered out by processor", event.id);
                } else {
                    for processed_event in processed_events {
                        debug!("[STREAM] Forwarding event {} to sink relays", processed_event.id);

                        match client.send_event(&processed_event).await {
                            Ok(_) => {
                                debug!("[STREAM] Event sent successfully");
                            }
                            Err(e) => {
                                warn!("[STREAM] Failed to send event to sink relay: {}", e);
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
            if !self.no_state {
                state.save(&self.config.state_file).await?;
            }
            info!("Final state: {}", state.get_coverage_stats());
        }

        Ok(())
    }

    /// Fetch events in a specific time range
    pub async fn fetch_range(
        &self,
        start: Timestamp,
        end: Timestamp,
        limit: usize,
        wait_seconds: u64,
    ) -> Result<()> {
        let fetcher = Fetcher::new(
            self.source_client.clone(),
            self.sink_clients.clone(),
        );

        let mut iterations = 0;

        loop {
            iterations += 1;

            let result = {
                let mut state = self.state.lock().await;
                let fetch_result = fetcher.fetch_range(start, end, &mut *state, limit).await?;

                // Save state after each successful fetch
                if fetch_result.fetched && !self.no_state {
                    state.save(&self.config.state_file).await?;
                }
                fetch_result
            };

            if !result.fetched {
                info!("[FETCH] Range fully processed after {} iterations", iterations - 1);
                break;
            }

            // Check if we hit the limit (likely have more to fetch)
            if result.events_count as usize >= limit {
                info!("[FETCH] Hit limit, continuing to fetch remaining gaps");

                // Wait between fetches if configured
                if wait_seconds > 0 {
                    tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
                }
            } else {
                info!("[FETCH] Fetched {} events (less than limit), stopping", result.events_count);
                break;
            }
        }

        Ok(())
    }

    /// Fill gaps in existing collected spans
    pub async fn fill_gaps(&self, limit: usize) -> Result<()> {
        info!("[FETCH] Starting gap-filling mode");

        // Get the range to check for gaps
        let (earliest, latest) = {
            let state = self.state.lock().await;

            // Only look for gaps if we have existing data
            if state.collected_spans.span_count() == 0 {
                info!("[FETCH] No existing data to find gaps in");
                return Ok(());
            }

            match (state.collected_spans.earliest(), state.collected_spans.latest()) {
                (Some(e), Some(l)) => (e, l),
                _ => {
                    info!("[FETCH] No valid range for gap filling");
                    return Ok(());
                }
            }
        };

        // Use the fetcher to fill gaps in the range
        let fetcher = Fetcher::new(
            self.source_client.clone(),
            self.sink_clients.clone(),
        );

        let result = {
            let mut state = self.state.lock().await;
            let fetch_result = fetcher.fetch_range(earliest, latest, &mut *state, limit).await?;

            // Save state after fetching
            if fetch_result.fetched && !self.no_state {
                state.save(&self.config.state_file).await?;
            }
            if fetch_result.fetched {
                info!("[FETCH] Filled {} gaps with {} events",
                      fetch_result.spans_processed.len(),
                      fetch_result.events_count);
            }
            fetch_result
        };

        if !result.fetched {
            info!("[FETCH] No gaps found, exiting");
        } else {
            info!("[FETCH] Completed gap-filling");
        }

        Ok(())
    }

    /// Get a copy of the current processing state
    pub async fn get_state(&self) -> ProcessingState {
        let state = self.state.lock().await;
        state.clone()
    }

    /// Save the current state to disk
    pub async fn save_state(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.end_session();
        if !self.no_state {
            state.save(&self.config.state_file).await?;
        }
        info!("State saved: {}", state.get_coverage_stats());
        Ok(())
    }

    /// Get access to the source client for custom operations
    pub fn source_client(&self) -> &Client {
        &self.source_client
    }

    /// Get access to sink clients and their processors for custom operations
    pub fn sink_clients(&self) -> &[(Arc<dyn Processor>, Client)] {
        &self.sink_clients
    }
}