use anyhow::Result;
use nostr::prelude::*;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::processor::Processor;
use crate::state::ProcessingState;
use crate::timespan::TimeSpan;

pub struct FetchResult {
    pub fetched: bool,
    pub events_count: u64,
    pub spans_processed: Vec<TimeSpan>,
}

pub struct Fetcher {
    source_client: Client,
    sink_clients: Vec<(Arc<dyn Processor>, Client)>,
}

impl Fetcher {
    pub fn new(source_client: Client, sink_clients: Vec<(Arc<dyn Processor>, Client)>) -> Self {
        Self {
            source_client,
            sink_clients,
        }
    }

    /// Fetch events in a specific time range, considering already processed spans.
    /// Returns false if the entire range is already collected, true if any fetching was done.
    pub async fn fetch_range(
        &self,
        start: Timestamp,
        end: Timestamp,
        state: &mut ProcessingState,
        limit: usize,
    ) -> Result<FetchResult> {
        // Get gaps in the requested range
        let gaps = state.collected_spans.get_gaps(start, end);

        if gaps.is_empty() {
            info!("[FETCH] Range already fully processed");
            return Ok(FetchResult {
                fetched: false,
                events_count: 0,
                spans_processed: vec![],
            });
        }

        info!("[FETCH] Found {} gaps to fetch", gaps.len());

        let mut total_events = 0u64;
        let mut spans_processed = Vec::new();

        for gap in gaps.iter() {
            info!(
                "[FETCH] Fetching gap from {} to {} ({} seconds)",
                gap.start.as_u64(),
                gap.end.as_u64(),
                gap.end.as_u64() - gap.start.as_u64()
            );

            let mut filter = Filter::new()
                .since(gap.start)
                .until(gap.end);

            // Only add limit if it's greater than 0
            if limit > 0 {
                filter = filter.limit(limit);
            }

            match self.source_client.fetch_events(filter, Duration::from_secs(30)).await {
                Ok(events) => {
                    let event_count = events.len();
                    info!("[FETCH] Retrieved {} events", event_count);

                    if event_count > 0 {
                        // Find the actual time range of events we received
                        let first_event = events.first().unwrap();
                        let mut min_timestamp = first_event.created_at;
                        let mut max_timestamp = first_event.created_at;

                        // Process and forward events
                        for event in events.iter() {
                            debug!("[FETCH] Processing event: {} at {}", event.id, event.created_at);

                            // Track the actual time range
                            if event.created_at < min_timestamp {
                                min_timestamp = event.created_at;
                            }
                            if event.created_at > max_timestamp {
                                max_timestamp = event.created_at;
                            }

                            for (processor, client) in &self.sink_clients {
                                let processed_events = processor.process(event);

                                if !processed_events.is_empty() {
                                    for processed_event in processed_events {
                                        if let Err(e) = client.send_event(&processed_event).await {
                                            warn!("[FETCH] Failed to send event: {}", e);
                                        }
                                    }
                                }
                            }
                        }

                        // Always mark only the actual range we received, not the requested range
                        // This is more conservative but handles relay limits correctly
                        info!("[FETCH] Got {} events, marking actual range: {} to {}",
                              event_count, min_timestamp, max_timestamp);
                        state.collected_spans.add_span(min_timestamp, max_timestamp);
                        spans_processed.push(TimeSpan::new(min_timestamp, max_timestamp));

                        state.total_events_processed += event_count as u64;
                        total_events += event_count as u64;
                    } else {
                        // No events in this gap, but we checked it
                        info!("[FETCH] No events in gap, marking as checked");
                        state.collected_spans.add_span(gap.start, gap.end);
                        spans_processed.push(gap.clone());
                    }

                    info!("[FETCH] State updated: {}", state.get_coverage_stats());
                }
                Err(e) => {
                    warn!("[FETCH] Failed to fetch events for gap: {}", e);
                    // Continue with next gap even if one fails
                }
            }

        }

        Ok(FetchResult {
            fetched: true,
            events_count: total_events,
            spans_processed,
        })
    }

    /// Fetch multiple gaps with delays between them
    pub async fn fetch_gaps(
        &self,
        gaps: Vec<TimeSpan>,
        state: &mut ProcessingState,
        limit: usize,
        wait_seconds: u64,
    ) -> Result<FetchResult> {
        if gaps.is_empty() {
            return Ok(FetchResult {
                fetched: false,
                events_count: 0,
                spans_processed: vec![],
            });
        }

        info!("[FETCH] Processing {} gaps", gaps.len());

        let mut total_events = 0u64;
        let mut spans_processed = Vec::new();

        for (idx, gap) in gaps.iter().enumerate() {
            let result = self.fetch_range(gap.start, gap.end, state, limit).await?;

            if result.fetched {
                total_events += result.events_count;
                spans_processed.extend(result.spans_processed);
            }

            // Delay between gaps if configured and not the last gap
            if wait_seconds > 0 && idx < gaps.len() - 1 {
                tokio::time::sleep(Duration::from_secs(wait_seconds)).await;
            }
        }

        Ok(FetchResult {
            fetched: !spans_processed.is_empty(),
            events_count: total_events,
            spans_processed,
        })
    }

    /// Fetch events in steps (for large time ranges)
    pub async fn fetch_range_in_steps(
        &self,
        start: Timestamp,
        end: Timestamp,
        step_seconds: u64,
        state: &mut ProcessingState,
        limit: usize,
    ) -> Result<FetchResult> {
        let mut current_start = start;
        let mut total_result = FetchResult {
            fetched: false,
            events_count: 0,
            spans_processed: vec![],
        };

        info!("[FETCH] Fetching in steps of {} seconds", step_seconds);

        while current_start < end {
            let current_end = Timestamp::from(
                (current_start.as_u64() + step_seconds).min(end.as_u64())
            );

            info!(
                "[FETCH] Processing step: {} to {}",
                current_start.as_u64(),
                current_end.as_u64()
            );

            let result = self.fetch_range(current_start, current_end, state, limit).await?;

            if result.fetched {
                total_result.fetched = true;
                total_result.events_count += result.events_count;
                total_result.spans_processed.extend(result.spans_processed);
            }

            current_start = current_end;
            // Note: Delays between steps should be handled by the caller
        }

        Ok(total_result)
    }
}