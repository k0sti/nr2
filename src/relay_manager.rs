use anyhow::{anyhow, Result};
use nostr::prelude::*;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

use crate::cli::{FetchMode, parse_duration, timestamp_from_date, timestamp_from_duration_ago};
use nr2::config::Config;
use nr2::fetcher::Fetcher;
use nr2::processor::{create_processor, Processor};
use nr2::state::ProcessingState;

#[derive(Clone)]
pub struct RelayManager {
    config: Config,
    source_client: Client,
    sink_clients: Vec<(Arc<dyn Processor>, Client)>,
    state: Arc<Mutex<ProcessingState>>,
    no_state: bool,
}

impl RelayManager {
    pub async fn new(config: Config, state: ProcessingState) -> Result<Self> {
        Self::new_with_options(config, state, false).await
    }

    pub async fn new_with_options(config: Config, state: ProcessingState, no_state: bool) -> Result<Self> {
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
            no_state,
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
            no_state: self.no_state,
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
                // Note: This periodic save happens in a spawned task, doesn't have access to no_state flag
                // But it's okay as it's only for streaming mode checkpoint saves
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
            if !self.no_state {
                state.save(&self.config.state_file).await?;
            }
            info!("Final state: {}", state.get_coverage_stats());
        }

        Ok(())
    }

    pub async fn save_state(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        state.end_session();
        if !self.no_state {
            state.save(&self.config.state_file).await?;
        }
        info!("State saved: {}", state.get_coverage_stats());
        Ok(())
    }

    /// Fill gaps only within existing collected spans (one-time operation)
    async fn fill_gaps_only(&self, fetch_limit: usize, _wait_seconds: u64) -> Result<()> {
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
            let fetch_result = fetcher.fetch_range(earliest, latest, &mut *state, fetch_limit).await?;

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
    /// Will continue fetching iteratively until the range is covered or max iterations reached
    async fn fetch_range(&self, start: Timestamp, end: Timestamp, limit: usize, wait_seconds: u64) -> Result<()> {
        let fetcher = Fetcher::new(
            self.source_client.clone(),
            self.sink_clients.clone(),
        );

        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 1000;

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                warn!("[FETCH] Hit maximum iteration limit ({}), stopping", MAX_ITERATIONS);
                break;
            }

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
            // If we got exactly 'limit' events or more, continue fetching
            if result.events_count as usize >= limit {
                info!("[FETCH] Hit limit, continuing to fetch remaining gaps");

                // Wait between fetches if configured
                if wait_seconds > 0 {
                    tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
                }
            } else {
                // Got less than limit, we're probably done
                info!("[FETCH] Fetched {} events (less than limit), stopping", result.events_count);
                break;
            }
        }

        Ok(())
    }

    /// Fetch events in steps (for large time ranges)
    async fn fetch_range_in_steps(&self, start: Timestamp, end: Timestamp, step_seconds: u64, limit: usize, wait_seconds: u64) -> Result<()> {
        let fetcher = Fetcher::new(
            self.source_client.clone(),
            self.sink_clients.clone(),
        );

        let mut current_start = start;
        let mut total_fetched = false;

        while current_start < end {
            let current_end = Timestamp::from(
                (current_start.as_u64() + step_seconds).min(end.as_u64())
            );

            info!("[FETCH] Processing step: {} to {}",
                current_start.as_u64(),
                current_end.as_u64()
            );

            let _result = {
                let mut state = self.state.lock().await;
                let fetch_result = fetcher.fetch_range(current_start, current_end, &mut *state, limit).await?;

                // Save state after each step if there was fetching
                if fetch_result.fetched && !self.no_state {
                    state.save(&self.config.state_file).await?;
                }
                if fetch_result.fetched {
                    total_fetched = true;
                }
                fetch_result
            };

            current_start = current_end;

            // Delay between steps if configured and not at end
            if current_start < end && wait_seconds > 0 {
                tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
            }
        }

        if !total_fetched {
            info!("[FETCH] Range already fully processed");
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