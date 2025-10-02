use anyhow::Result;
use nostr::Timestamp;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{debug, info};

use crate::timespan::TimeSpanSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingState {
    /// Tracks all time spans that have been processed
    pub collected_spans: TimeSpanSet,

    /// Current session's start time (when streaming started)
    pub session_start: Option<Timestamp>,

    /// Last event timestamp in current session
    pub session_last_event: Option<Timestamp>,

    /// Total number of events processed across all sessions
    pub total_events_processed: u64,

    /// Statistics for monitoring
    pub stats: ProcessingStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingStats {
    pub sessions_count: u32,
    pub total_gaps_filled: u32,
    pub last_save_timestamp: Option<Timestamp>,
}

impl ProcessingState {
    pub fn new() -> Self {
        Self {
            collected_spans: TimeSpanSet::new(),
            session_start: None,
            session_last_event: None,
            total_events_processed: 0,
            stats: ProcessingStats {
                sessions_count: 0,
                total_gaps_filled: 0,
                last_save_timestamp: None,
            },
        }
    }

    /// Load state from file
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            let content = tokio::fs::read_to_string(path).await?;
            let state: ProcessingState = serde_json::from_str(&content)?;

            info!("Loaded state: {} events processed, {} spans collected, {} sessions",
                  state.total_events_processed,
                  state.collected_spans.span_count(),
                  state.stats.sessions_count);

            if let Some(earliest) = state.collected_spans.earliest() {
                if let Some(latest) = state.collected_spans.latest() {
                    info!("Coverage from {} to {}", earliest, latest);
                }
            }

            Ok(state)
        } else {
            info!("No existing state file found, starting fresh");
            Ok(Self::new())
        }
    }

    /// Save state to file
    pub async fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.stats.last_save_timestamp = Some(Timestamp::now());
        let json = serde_json::to_string_pretty(&self)?;
        tokio::fs::write(path, json).await?;
        debug!("State saved: {} events, {} spans",
               self.total_events_processed,
               self.collected_spans.span_count());
        Ok(())
    }

    /// Start a new streaming session
    pub fn start_session(&mut self, start_time: Timestamp) {
        self.session_start = Some(start_time);
        self.session_last_event = None;
        self.stats.sessions_count += 1;
        info!("Started session #{} at {}", self.stats.sessions_count, start_time);
    }

    /// Update state with a processed event
    pub fn process_event(&mut self, timestamp: Timestamp) {
        self.session_last_event = Some(timestamp);
        self.total_events_processed += 1;

        // For streaming, we don't update collected_spans here
        // It will be updated when session ends or periodically
    }

    /// End current session and update collected spans
    pub fn end_session(&mut self) {
        if let (Some(start), Some(end)) = (self.session_start, self.session_last_event) {
            info!("Ending session: {} to {} ({} events)",
                  start, end, self.total_events_processed);
            self.collected_spans.add_span(start, end);
        } else if let Some(start) = self.session_start {
            // Session started but no events received
            info!("Ending session with no events from {}", start);
        }

        self.session_start = None;
        self.session_last_event = None;
    }

    /// Save current session progress (for periodic saves without ending session)
    pub fn checkpoint_session(&mut self) {
        if let (Some(start), Some(end)) = (self.session_start, self.session_last_event) {
            debug!("Checkpointing session: {} to {}", start, end);
            self.collected_spans.add_span(start, end);
        }
    }

    /// Get gaps in collected time spans
    #[allow(dead_code)]
    pub fn get_gaps(&self, from: Timestamp, to: Timestamp) -> Vec<(Timestamp, Timestamp)> {
        self.collected_spans.get_gaps(from, to)
            .into_iter()
            .map(|span| (span.start, span.end))
            .collect()
    }

    /// Check if a timestamp has already been collected
    #[allow(dead_code)]
    pub fn is_collected(&self, timestamp: Timestamp) -> bool {
        self.collected_spans.contains(timestamp)
    }

    /// Get coverage statistics
    pub fn get_coverage_stats(&self) -> String {
        format!(
            "Total events: {}, Spans: {}, Coverage: {} seconds, Sessions: {}",
            self.total_events_processed,
            self.collected_spans.span_count(),
            self.collected_spans.total_coverage_seconds(),
            self.stats.sessions_count
        )
    }

    /// Mark a gap as filled
    #[allow(dead_code)]
    pub fn mark_gap_filled(&mut self, start: Timestamp, end: Timestamp) {
        self.collected_spans.add_span(start, end);
        self.stats.total_gaps_filled += 1;
        info!("Filled gap: {} to {}", start, end);
    }
}

impl Default for ProcessingState {
    fn default() -> Self {
        Self::new()
    }
}