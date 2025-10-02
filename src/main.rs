mod cli;
mod relay_manager;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::cli::Cli;
use crate::relay_manager::RelayManager;
use eventflow::config::Config;
use eventflow::state::ProcessingState;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Validate CLI arguments
    cli.validate()?;

    // Handle --show-state
    if cli.show_state {
        return show_state().await;
    }

    // Configure logging with EnvFilter
    // Default: show debug for eventflow, only warnings for nostr_relay_pool::relay::inner
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("eventflow=debug,nostr_relay_pool::relay::inner=warn,nostr_relay_pool=info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    info!("Starting EventFlow - Nostr Event Router");

    let config_path = PathBuf::from("config.toml");

    let config = if config_path.exists() {
        info!("Loading configuration from config.toml");
        Config::load(&config_path).await?
    } else {
        info!("Using default configuration");
        let default_config = Config::default();

        info!("Writing default configuration to config.toml");
        let toml_string = toml::to_string_pretty(&default_config)?;
        tokio::fs::write(&config_path, toml_string).await?;

        default_config
    };

    info!("Configuration loaded:");
    info!("  Source relays: {:?}", config.sources);
    info!("  Sinks: {} configured", config.sinks.len());
    info!("  State file: {}", config.state_file);

    // Load or create processing state
    let state = if cli.no_state {
        info!("Using in-memory state only (--no-state flag)");
        ProcessingState::new()
    } else {
        let state_path = PathBuf::from(&config.state_file);
        ProcessingState::load(&state_path).await?
    };

    let relay_manager = RelayManager::new_with_options(config.clone(), state, cli.no_state).await?;

    let shutdown = tokio::signal::ctrl_c();

    // Run based on selected modes
    let fetch_mode = cli.get_fetch_mode();
    let has_stream = cli.has_stream();
    let fetch_limit = cli.limit;
    let wait_seconds = cli.get_wait_seconds();
    let step = cli.get_step();
    let no_state = cli.no_state;

    tokio::select! {
        result = relay_manager.run_with_mode(has_stream, fetch_mode, fetch_limit, wait_seconds, step) => {
            if let Err(e) = result {
                error!("Relay manager error: {}", e);
            }
        }
        _ = shutdown => {
            info!("Received shutdown signal");
        }
    }

    info!("Shutting down...");

    // Save final state before disconnecting (unless --no-state)
    if !no_state {
        relay_manager.save_state().await?;
    }

    relay_manager.disconnect().await?;

    Ok(())
}

async fn show_state() -> Result<()> {
    use chrono::{DateTime, Utc};
    use nostr::Timestamp;

    let config_path = PathBuf::from("config.toml");
    let config = if config_path.exists() {
        Config::load(&config_path).await?
    } else {
        Config::default()
    };

    let state_path = PathBuf::from(&config.state_file);

    if !state_path.exists() {
        println!("No state file found at: {}", config.state_file);
        return Ok(());
    }

    let state = ProcessingState::load(&state_path).await?;

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
    println!("║ Total Events Processed: {:>36} ║", state.total_events_processed);
    println!("║ Total Sessions:         {:>36} ║", state.stats.sessions_count);
    println!("║ Total Gaps Filled:      {:>36} ║", state.stats.total_gaps_filled);
    println!("╠══════════════════════════════════════════════════════════════╣");

    // Current session info
    if let Some(start) = state.session_start {
        println!("║ Current Session:                                              ║");
        let started_line = format!("Started: {}", format_timestamp(start));
        println!("║   {:<60} ║", started_line);
        if let Some(last) = state.session_last_event {
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
    let spans = state.collected_spans.spans();
    println!("║   Time Spans Collected: {:>36} ║", spans.len());

    if !spans.is_empty() {
        let earliest = state.collected_spans.earliest().unwrap();
        let latest = state.collected_spans.latest().unwrap();
        println!("║   Earliest: {}                              ║", format_timestamp(earliest));
        println!("║   Latest:   {}                              ║", format_timestamp(latest));

        let total_coverage = state.collected_spans.total_coverage_seconds();
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
    let gaps = state.collected_spans.get_gaps(seven_days_ago, now);

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

    Ok(())
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
