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

    /// Show processing state in a formatted display
    pub async fn show_state_from_config(config_path: &std::path::Path) -> anyhow::Result<()> {
        use crate::config::Config;

        let config = if config_path.exists() {
            Config::load(&config_path).await?
        } else {
            Config::default()
        };

        let state_path = std::path::PathBuf::from(&config.state_file);

        if !state_path.exists() {
            println!("No state file found at: {}", config.state_file);
            return Ok(());
        }

        let state = ProcessingState::load(&state_path).await?;
        state.display();
        Ok(())
    }

    /// Display the state in a formatted way
    pub fn display(&self) {
        use chrono::{DateTime, Utc};

        // Helper function to format timestamp
        let format_timestamp = |ts: Timestamp| -> String {
            let secs = ts.as_u64() as i64;
            let dt = DateTime::<Utc>::from_timestamp(secs, 0)
                .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        };

        println!("╔══════════════════════════════════════════════════════════════╗");
        println!("║                EventFlow Processing State                    ║");
        println!("╠══════════════════════════════════════════════════════════════╣");
        println!("║ Total Events Processed: {:>36} ║", self.total_events_processed);
        println!("║ Total Sessions:         {:>36} ║", self.stats.sessions_count);
        println!("║ Total Gaps Filled:      {:>36} ║", self.stats.total_gaps_filled);
        println!("╠══════════════════════════════════════════════════════════════╣");

        // Current session info
        if let Some(start) = self.session_start {
            println!("║ Current Session:                                              ║");
            let started_line = format!("Started: {}", format_timestamp(start));
            println!("║   {:<60} ║", started_line);
            if let Some(last) = self.session_last_event {
                let last_line = format!("Last Event: {}", format_timestamp(last));
                println!("║   {:<60} ║", last_line);
                let duration = last.as_u64() - start.as_u64();
                let duration_line = format!("Duration: {}", format_duration(duration));
                println!("║   {:<60} ║", duration_line);
            }
            println!("╠══════════════════════════════════════════════════════════════╣");
        }

        // Coverage info
        println!("║ Coverage Information:                                        ║");
        let spans = self.collected_spans.spans();
        println!("║   Time Spans Collected: {:>36} ║", spans.len());

        if !spans.is_empty() {
            let earliest = self.collected_spans.earliest().unwrap();
            let latest = self.collected_spans.latest().unwrap();
            println!("║   Earliest: {}                              ║", format_timestamp(earliest));
            println!("║   Latest:   {}                              ║", format_timestamp(latest));

            let total_coverage = self.collected_spans.total_coverage_seconds();
            let coverage_str = format_duration(total_coverage);
            println!("║   Total Coverage: {:<42} ║", coverage_str);

            // Show individual spans
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║ Time Spans:                                                  ║");
            for (i, span) in spans.iter().enumerate() {
                if i < 10 {  // Show first 10 spans
                    let span_line = format!("{}. {} to {}",
                        i + 1,
                        format_timestamp(span.start),
                        format_timestamp(span.end)
                    );
                    println!("║   {:<58} ║", span_line);
                    let duration = span.end.as_u64() - span.start.as_u64();
                    let duration_line = format!("   Duration: {}", format_duration(duration));
                    println!("║   {:<58} ║", duration_line);
                }
            }
            if spans.len() > 10 {
                println!("║   ... and {} more spans                                      ║", spans.len() - 10);
            }
        }

        // Show gaps in the last 7 days
        let now = Timestamp::now();
        let seven_days_ago = Timestamp::from(now.as_u64().saturating_sub(7 * 24 * 60 * 60));
        let gaps = self.collected_spans.get_gaps(seven_days_ago, now);

        if !gaps.is_empty() {
            println!("╠══════════════════════════════════════════════════════════════╣");
            println!("║ Gaps in Last 7 Days:                                         ║");
            for (i, gap) in gaps.iter().enumerate() {
                if i < 5 {  // Show first 5 gaps
                    let gap_line = format!("{}. {} to {}",
                        i + 1,
                        format_timestamp(gap.start),
                        format_timestamp(gap.end)
                    );
                    println!("║   {:<58} ║", gap_line);
                    let duration = gap.end.as_u64() - gap.start.as_u64();
                    let missing_line = format!("   Missing: {}", format_duration(duration));
                    println!("║   {:<58} ║", missing_line);
                }
            }
            if gaps.len() > 5 {
                let more_line = format!("... and {} more gaps", gaps.len() - 5);
                println!("║   {:<60} ║", more_line);
            }
        }

        println!("╚══════════════════════════════════════════════════════════════╝");
    }
}

fn format_duration(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{:2}d {:2}h {:2}m {:2}s", days, hours, minutes, secs)
    } else if hours > 0 {
        format!("{:2}h {:2}m {:2}s", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{:2}m {:2}s", minutes, secs)
    } else {
        format!("{:2}s", secs)
    }
}

impl Default for ProcessingState {
    fn default() -> Self {
        Self::new()
    }
}