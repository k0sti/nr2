mod config;
mod processor;
mod relay_manager;
mod state;
mod timespan;

use anyhow::Result;
use std::path::PathBuf;
use tracing::{error, info};
use tracing_subscriber;

use crate::config::Config;
use crate::relay_manager::RelayManager;
use crate::state::ProcessingState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    info!("Starting NR2 - Nostr Relay Router");

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
    let state_path = PathBuf::from(&config.state_file);
    let state = ProcessingState::load(&state_path).await?;

    let relay_manager = RelayManager::new(config.clone(), state).await?;

    let shutdown = tokio::signal::ctrl_c();

    tokio::select! {
        result = relay_manager.run() => {
            if let Err(e) = result {
                error!("Relay manager error: {}", e);
            }
        }
        _ = shutdown => {
            info!("Received shutdown signal");
        }
    }

    info!("Shutting down...");

    // Save final state before disconnecting
    relay_manager.save_state().await?;

    relay_manager.disconnect().await?;

    Ok(())
}
