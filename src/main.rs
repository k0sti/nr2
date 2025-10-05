mod cli;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::cli::Cli;
use eventflow::config::Config;
use eventflow::state::ProcessingState;
use eventflow::relay_router::RelayRouter;
use nostr::Timestamp;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command-line arguments
    let cli = Cli::parse();

    // Validate CLI arguments
    cli.validate()?;

    // Handle --show-state
    if cli.show_state {
        let config_path = PathBuf::from("config.toml");
        return ProcessingState::show_state_from_config(&config_path).await;
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

    let relay_router = if cli.no_state {
        RelayRouter::new_with_options(config.clone(), state, true).await?
    } else {
        RelayRouter::new(config.clone(), state).await?
    };

    let shutdown = tokio::signal::ctrl_c();

    // Run based on selected modes
    let fetch_mode = cli.get_fetch_mode();
    let has_stream = cli.has_stream();
    let fetch_limit = cli.limit;
    let wait_seconds = cli.get_wait_seconds();
    let step = cli.get_step();
    let no_state = cli.no_state;

    relay_router.connect().await;

    tokio::select! {
        result = run_with_mode(&relay_router, has_stream, fetch_mode, fetch_limit, wait_seconds, step) => {
            if let Err(e) = result {
                error!("Relay router error: {}", e);
            }
        }
        _ = shutdown => {
            info!("Received shutdown signal");
        }
    }

    info!("Shutting down...");

    // Save final state before disconnecting (unless --no-state)
    if !no_state {
        relay_router.save_state().await?;
    }

    relay_router.disconnect().await?;

    Ok(())
}

async fn run_with_mode(
    relay_router: &RelayRouter,
    has_stream: bool,
    mode: cli::FetchMode,
    fetch_limit: usize,
    wait_seconds: u64,
    step: Option<String>,
) -> Result<()> {
    use crate::cli::{timestamp_from_duration_ago, timestamp_from_date, FetchMode};
    use std::time::Duration;
    use tokio::time;

    match mode {
        FetchMode::None => {
            if has_stream {
                relay_router.stream_events().await
            } else {
                info!("No fetch mode specified and streaming disabled. Exiting.");
                Ok(())
            }
        }
        FetchMode::Gaps => {
            loop {
                info!("Filling gaps with limit: {}", fetch_limit);
                relay_router.fill_gaps(fetch_limit).await?;

                if has_stream {
                    info!("Starting streaming after gap filling");
                    let router_clone = relay_router.clone();
                    let stream_handle = tokio::spawn(async move {
                        if let Err(e) = router_clone.stream_events().await {
                            error!("Streaming error: {}", e);
                        }
                    });

                    time::sleep(Duration::from_secs(wait_seconds)).await;
                    stream_handle.abort();
                } else {
                    break;
                }
            }
            Ok(())
        }
        FetchMode::Back(duration_str) => {
            let start_time = timestamp_from_duration_ago(&duration_str)?;
            let end_time = Timestamp::now();

            if let Some(step_str) = step {
                fetch_range_in_steps(relay_router, start_time, end_time, &step_str, fetch_limit, wait_seconds, has_stream).await
            } else {
                fetch_range_simple(relay_router, start_time, end_time, fetch_limit, wait_seconds, has_stream).await
            }
        }
        FetchMode::From(date_str) => {
            let start_time = timestamp_from_date(&date_str)?;
            let end_time = Timestamp::now();

            if let Some(step_str) = step {
                fetch_range_in_steps(relay_router, start_time, end_time, &step_str, fetch_limit, wait_seconds, has_stream).await
            } else {
                fetch_range_simple(relay_router, start_time, end_time, fetch_limit, wait_seconds, has_stream).await
            }
        }
    }
}

async fn fetch_range_simple(
    relay_router: &RelayRouter,
    start: Timestamp,
    end: Timestamp,
    limit: usize,
    wait_seconds: u64,
    has_stream: bool,
) -> Result<()> {
    use std::time::Duration;
    use tokio::time;

    loop {
        info!("Fetching range from {} to {} with limit {}", start.as_u64(), end.as_u64(), limit);
        relay_router.fetch_range(start, end, limit, wait_seconds).await?;

        if has_stream {
            info!("Starting streaming after range fetch");
            let router_clone = relay_router.clone();
            let stream_handle = tokio::spawn(async move {
                if let Err(e) = router_clone.stream_events().await {
                    error!("Streaming error: {}", e);
                }
            });

            time::sleep(Duration::from_secs(wait_seconds)).await;
            stream_handle.abort();
        } else {
            break;
        }
    }
    Ok(())
}

async fn fetch_range_in_steps(
    relay_router: &RelayRouter,
    start: Timestamp,
    end: Timestamp,
    step_str: &str,
    limit: usize,
    wait_seconds: u64,
    has_stream: bool,
) -> Result<()> {
    use crate::cli::{parse_duration, duration_to_seconds};
    use std::time::Duration;
    use tokio::time;

    let step_duration = parse_duration(step_str)?;
    let step_seconds = duration_to_seconds(&step_duration);

    let mut current_start = start;
    loop {
        let current_end = Timestamp::from(std::cmp::min(
            current_start.as_u64() + step_seconds,
            end.as_u64(),
        ));

        info!("Fetching step from {} to {} with limit {}",
              current_start.as_u64(), current_end.as_u64(), limit);

        relay_router.fetch_range(current_start, current_end, limit, wait_seconds).await?;

        if current_end >= end {
            break;
        }

        current_start = current_end;

        if has_stream {
            info!("Starting streaming after step fetch");
            let router_clone = relay_router.clone();
            let stream_handle = tokio::spawn(async move {
                if let Err(e) = router_clone.stream_events().await {
                    error!("Streaming error: {}", e);
                }
            });

            time::sleep(Duration::from_secs(wait_seconds)).await;
            stream_handle.abort();
        } else {
            time::sleep(Duration::from_secs(1)).await; // Brief pause between steps
        }
    }
    Ok(())
}

