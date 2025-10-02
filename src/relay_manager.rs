use anyhow::{anyhow, Result};
use nostr::prelude::*;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use crate::cli::{FetchMode, parse_duration, timestamp_from_date, timestamp_from_duration_ago};
use crate::config::Config;
use crate::processor::{create_processor, Processor};
use crate::state::ProcessingState;

#[derive(Clone)]
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

    #[allow(dead_code)]
    pub async fn run(&self) -> Result<()> {
        // Default behavior: run stream only
        self.run_with_mode(true, FetchMode::None, 500, 0, None).await
    }

    pub async fn run_with_mode(&self, has_stream: bool, mode: FetchMode, fetch_limit: usize, wait_seconds: u64, step: Option<String>) -> Result<()> {
        info!("Connecting to relays...");
        self.connect_relays().await;

        // Handle different combinations
        match (has_stream, mode) {
            // Stream only
            (true, FetchMode::None) => {
                info!("Running in stream mode only");
                self.stream_events().await
            }
            // Fetch mode only
            (false, fetch_mode) => {
                self.run_fetch_mode(fetch_mode, fetch_limit, wait_seconds, step).await
            }
            // Both stream and fetch mode - run concurrently
            (true, fetch_mode) => {
                info!("Running stream and fetch modes concurrently");

                let stream_handle = tokio::spawn({
                    let self_clone = self.clone();
                    async move {
                        if let Err(e) = self_clone.stream_events().await {
                            error!("[STREAM] Error: {}", e);
                        }
                    }
                });

                let _fetch_handle = tokio::spawn({
                    let self_clone = self.clone();
                    let mode_clone = fetch_mode.clone();
                    let step_clone = step.clone();
                    async move {
                        // Give stream a moment to start
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        if let Err(e) = self_clone.run_fetch_mode(mode_clone, fetch_limit, wait_seconds, step_clone).await {
                            error!("[FETCH] Error: {}", e);
                        }
                    }
                });

                // When stream is active, only wait for stream to end (fetch can complete)
                stream_handle.await.map_err(|e| anyhow!("Stream task failed: {}", e))?;
                info!("Stream ended");

                Ok(())
            }
        }
    }

    async fn run_fetch_mode(&self, mode: FetchMode, fetch_limit: usize, wait_seconds: u64, step: Option<String>) -> Result<()> {
        match mode {
            FetchMode::Gaps => {
                info!("Running gap-fill mode");
                self.fill_gaps_only(fetch_limit, wait_seconds).await
            }
            FetchMode::Back(duration) => {
                info!("Fetching events from {} ago", duration);
                self.fetch_back(&duration, step, fetch_limit, wait_seconds).await
            }
            FetchMode::From(date) => {
                info!("Fetching events from {}", date);
                self.fetch_from(&date, step, fetch_limit, wait_seconds).await
            }
            FetchMode::Continuous => {
                if let Some(step_duration) = step {
                    info!("Fetching events continuously backwards with {} steps", step_duration);
                    self.fetch_continuous(&step_duration, fetch_limit, wait_seconds).await
                } else {
                    return Err(anyhow!("--fetch-continuous requires --step"));
                }
            }
            FetchMode::None => {
                warn!("No fetch mode specified");
                Ok(())
            }
        }
    }

    #[allow(dead_code)]
    fn clone_for_task(&self) -> Self {
        Self {
            config: self.config.clone(),
            source_client: self.source_client.clone(),
            sink_clients: self.sink_clients.clone(),
            state: self.state.clone(),
        }
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

        info!("[STREAM] Starting live stream from {}", start_time);
        let subscription = self.source_client.subscribe(filter, None).await?;
        info!("[STREAM] Subscription created: {:?}", subscription);

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
                    warn!("[STREAM] Failed to save state: {}", e);
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

    /// Fill gaps only within existing collected spans (one-time operation)
    async fn fill_gaps_only(&self, _fetch_limit: usize, wait_seconds: u64) -> Result<()> {
        info!("[FETCH] Starting gap-filling mode");

        // Get current gaps within existing data
        let gaps = {
            let state = self.state.lock().await;

            // Only look for gaps if we have existing data
            if state.collected_spans.span_count() == 0 {
                info!("[FETCH] No existing data to find gaps in");
                Vec::new()
            } else if let (Some(earliest), Some(latest)) =
                (state.collected_spans.earliest(), state.collected_spans.latest()) {
                // Find gaps within our collected range
                state.collected_spans.get_gaps(earliest, latest)
            } else {
                Vec::new()
            }
        };

        if gaps.is_empty() {
            info!("[FETCH] No gaps found, exiting");
            return Ok(());
        }

            info!("[FETCH] Found {} gaps to fill", gaps.len());

            // Process each gap
            for gap in gaps {
                info!(
                    "[FETCH] Filling gap from {} to {} ({} seconds)",
                    gap.start,
                    gap.end,
                    gap.end.as_u64() - gap.start.as_u64()
                );

                // Create filter for this gap
                let filter = Filter::new()
                    .since(gap.start)
                    .until(gap.end)
                    .limit(500);  // Reasonable limit per gap query

                match self.source_client.fetch_events(filter, Duration::from_secs(10)).await {
                    Ok(events) => {
                        info!("[FETCH] Retrieved {} events for gap", events.len());

                        if !events.is_empty() {
                            // Track the span we're filling
                            {
                                let mut state = self.state.lock().await;
                                state.collected_spans.add_span(gap.start, gap.end);
                                state.total_events_processed += events.len() as u64;
                            }

                            // Process and forward events
                            for event in events {
                                debug!("[FETCH] Processing event: {} at {}", event.id, event.created_at);

                                for (processor, client) in &self.sink_clients {
                                    let processed_events = processor.process(&event);

                                    if processed_events.is_empty() {
                                        debug!("[FETCH] Event {} filtered out by processor", event.id);
                                    } else {
                                        for processed_event in processed_events {
                                            debug!("[FETCH] Forwarding event {} to sink relays", processed_event.id);

                                            match client.send_event(&processed_event).await {
                                                Ok(_) => {
                                                    debug!("[FETCH] Event sent successfully");
                                                }
                                                Err(e) => {
                                                    warn!("[FETCH] Failed to send event to sink relay: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Save state after processing gap
                            {
                                let mut state = self.state.lock().await;
                                if let Err(e) = state.save(&self.config.state_file).await {
                                    warn!("[FETCH] Failed to save state: {}", e);
                                }
                                info!("[FETCH] State updated: {}", state.get_coverage_stats());
                            }
                        } else {
                            // Even if no events, mark the gap as collected
                            let mut state = self.state.lock().await;
                            state.collected_spans.add_span(gap.start, gap.end);
                            info!("[FETCH] No events in gap, marking as collected");
                        }
                    }
                    Err(e) => {
                        error!("[FETCH] Failed to fetch events for gap: {}", e);
                    }
                }

                // Delay between gap queries if configured
                if wait_seconds > 0 {
                    tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
                }
            }

        info!("[FETCH] Completed gap-filling");
        Ok(())
    }

    /// Fetch events going back a specific duration
    async fn fetch_back(&self, duration_str: &str, fetch_step: Option<String>, limit: usize, wait_seconds: u64) -> Result<()> {
        let start_time = timestamp_from_duration_ago(duration_str)?;
        let end_time = Timestamp::now();

        info!("[FETCH] Fetching events from {} to {}", start_time.as_u64(), end_time.as_u64());

        if let Some(step_str) = fetch_step {
            // Fetch in steps
            let step_duration = parse_duration(&step_str)?;
            let step_seconds = step_duration.num_seconds().max(60) as u64;
            self.fetch_range_in_steps(start_time, end_time, step_seconds, limit, wait_seconds).await
        } else {
            // Fetch entire range
            self.fetch_range(start_time, end_time, limit, wait_seconds).await
        }
    }

    /// Fetch events starting from a specific date
    async fn fetch_from(&self, date_str: &str, fetch_step: Option<String>, limit: usize, wait_seconds: u64) -> Result<()> {
        let start_time = timestamp_from_date(date_str)?;
        let end_time = Timestamp::now();

        info!("[FETCH] Fetching events from {} to now", date_str);

        if let Some(step_str) = fetch_step {
            // Fetch in steps
            let step_duration = parse_duration(&step_str)?;
            let step_seconds = step_duration.num_seconds().max(60) as u64;
            self.fetch_range_in_steps(start_time, end_time, step_seconds, limit, wait_seconds).await
        } else {
            // Fetch entire range
            self.fetch_range(start_time, end_time, limit, wait_seconds).await
        }
    }

    /// Fetch events in a specific time range, considering already processed spans
    async fn fetch_range(&self, start: Timestamp, end: Timestamp, limit: usize, wait_seconds: u64) -> Result<()> {
        // Get gaps in the requested range
        let gaps = {
            let state = self.state.lock().await;
            state.collected_spans.get_gaps(start, end)
        };

        if gaps.is_empty() {
            info!("[FETCH] Range already fully processed");
            return Ok(());
        }

        info!("[FETCH] Found {} gaps to fetch", gaps.len());

        for gap in gaps {
            info!("[FETCH] Fetching gap from {} to {} ({} seconds)",
                gap.start.as_u64(),
                gap.end.as_u64(),
                gap.end.as_u64() - gap.start.as_u64()
            );

            let filter = Filter::new()
                .since(gap.start)
                .until(gap.end)
                .limit(limit);

            match self.source_client.fetch_events(filter, Duration::from_secs(30)).await {
                Ok(events) => {
                    let event_count = events.len();
                    info!("[FETCH] Retrieved {} events", event_count);

                    // Process and forward events
                    for event in events {
                        debug!("[FETCH] Processing event: {} at {}", event.id, event.created_at);

                        for (processor, client) in &self.sink_clients {
                            let processed_events = processor.process(&event);

                            if !processed_events.is_empty() {
                                for processed_event in processed_events {
                                    if let Err(e) = client.send_event(&processed_event).await {
                                        warn!("[FETCH] Failed to send event: {}", e);
                                    }
                                }
                            }
                        }
                    }

                    // Update state
                    {
                        let mut state = self.state.lock().await;
                        state.collected_spans.add_span(gap.start, gap.end);
                        state.total_events_processed += event_count as u64;
                        state.save(&self.config.state_file).await?;
                        info!("[FETCH] State updated: {}", state.get_coverage_stats());
                    }
                }
                Err(e) => {
                    error!("[FETCH] Failed to fetch events: {}", e);
                }
            }

            // Delay between fetches if configured
            if wait_seconds > 0 {
                tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
            }
        }

        Ok(())
    }

    /// Fetch events in steps (for large time ranges)
    async fn fetch_range_in_steps(&self, start: Timestamp, end: Timestamp, step_seconds: u64, limit: usize, wait_seconds: u64) -> Result<()> {
        let mut current_start = start;

        info!("[FETCH] Fetching in steps of {} seconds", step_seconds);

        while current_start < end {
            let current_end = Timestamp::from(
                (current_start.as_u64() + step_seconds).min(end.as_u64())
            );

            info!("[FETCH] Processing step: {} to {}",
                current_start.as_u64(),
                current_end.as_u64()
            );

            self.fetch_range(current_start, current_end, limit, wait_seconds).await?;

            current_start = current_end;

            // Delay between steps if configured
            if current_start < end && wait_seconds > 0 {
                tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
            }
        }

        Ok(())
    }

    /// Continuously fetch events backwards in time, stepping back by the given duration
    async fn fetch_continuous(&self, step_duration_str: &str, limit: usize, wait_seconds: u64) -> Result<()> {
        let step_duration = parse_duration(step_duration_str)?;
        let step_seconds = step_duration.num_seconds().max(60) as u64;

        info!("[FETCH-CONTINUOUS] Starting continuous backwards fetch with {} second steps", step_seconds);

        let mut current_end = Timestamp::now();

        loop {
            let current_start = Timestamp::from(current_end.as_u64().saturating_sub(step_seconds));

            info!("[FETCH-CONTINUOUS] Fetching from {} to {}",
                current_start.as_u64(),
                current_end.as_u64()
            );

            // Fetch this time range (fetch_range automatically skips already-fetched spans)
            self.fetch_range(current_start, current_end, limit, wait_seconds).await?;

            // Move backwards for the next iteration
            current_end = current_start;

            // Stop if we've gone too far back (e.g., before Nostr existed)
            if current_end.as_u64() < 1609459200 { // Jan 1, 2021
                info!("[FETCH-CONTINUOUS] Reached early limit, stopping");
                break;
            }

            // Wait before next fetch if configured
            if wait_seconds > 0 {
                info!("[FETCH-CONTINUOUS] Waiting {} seconds before next fetch", wait_seconds);
                tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
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