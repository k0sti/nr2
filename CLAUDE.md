# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview
NR2 (Nostr Relay Router) is a Rust application for routing and processing Nostr events between relays with time-based state tracking and gap filling capabilities.

## Build & Development Commands

```bash
# Build the project
cargo build

# Build release version
cargo build --release

# Run tests
cargo test

# Check code without building
cargo check

# Format code
cargo fmt

# Lint code
cargo clippy
```

## Running the Application

### Core Modes
```bash
# Stream live events
cargo run -- --stream

# Show current processing state
cargo run -- --show-state

# Fetch historical data with gap filling
cargo run -- --fetch-gaps

# Fetch events from specific time ago
cargo run -- --fetch-back 2h --step 30min --limit 100

# Fetch continuously backwards through time
cargo run -- --fetch-continuous --step 30min --wait 2s

# Combine streaming with historical fetching
cargo run -- --stream --fetch-back 24h
```

### Environment Variables
```bash
# Control logging levels
RUST_LOG="nr2=info"  # Show info logs for nr2
RUST_LOG="nr2=debug,nostr_relay_pool::relay::inner=warn"  # Debug with reduced relay noise
```

## Architecture

### Module Structure
- `main.rs` - Entry point, CLI handling, orchestrates relay manager
- `cli.rs` - Command-line argument parsing and validation
- `relay_manager.rs` - Core logic for connecting to relays, fetching/streaming events
- `processor.rs` - Event processing pipeline (passthrough, geohash filtering)
- `config.rs` - Configuration loading (TOML format)
- `state.rs` - Persistent state tracking for processed time spans
- `timespan.rs` - Time span management with automatic merging of overlapping ranges

### Data Flow
1. **Sources**: Events are fetched from source relays (configured in config.toml)
2. **Processing**: Events pass through configured processors (Passthrough, GeohashFilter)
3. **Sinks**: Processed events are forwarded to sink relays
4. **State Tracking**: TimeSpanSet tracks which time ranges have been processed to avoid duplicates

### Key Concepts
- **Time Spans**: The system tracks processed time ranges to enable gap filling and prevent re-fetching
- **Sessions**: Stream mode creates sessions that track continuous event processing
- **Gap Filling**: Automatically identifies and fetches missing time periods in collected data
- **Processors**: Modular event filtering system (extensible via Processor trait)

## Configuration
The app uses `config.toml` for relay and processor configuration. See `config.example.toml` for reference.

## State Persistence
Processing state is saved to `nr2_state.json` (configurable) containing:
- Collected time spans
- Session information
- Processing statistics
- Total events processed