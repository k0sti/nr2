# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview
EventFlow (Nostr EventFlow) is a Rust library and CLI application for routing and processing Nostr events between relays with time-based state tracking and gap filling capabilities. It can be used both as a standalone binary (`flow`) and as a library (`eventflow`).

## Build & Development Commands

```bash
# Build the project
cargo build
just build

# Run tests (including integration tests)
cargo test
cargo test --lib           # Run unit tests only
cargo test --test '*'       # Run integration tests only

# Linting and formatting
cargo fmt                   # Format code
cargo clippy               # Lint code
cargo check                # Check compilation without building

# Run examples
cargo run --example custom_processor
cargo run --example simple_relay_router
```

## Running the Application

### Using Just Commands (Preferred)
```bash
just run                    # Stream live events (default)
just stream                 # Stream live events
just show-state            # Show current processing state
just gaps                  # Fill gaps in existing data
just fetch-1h              # Fetch last hour of events
just fetch-1d              # Fetch last day of events
just fetch-continuous      # Continuously fetch backwards through time
just debug                 # Run with debug logging (suppresses nostr-sdk noise)
just test                  # Test run with debug output (first 50 lines)
```

### Direct CLI Usage
```bash
# Stream live events
cargo run -- --stream
# or if installed: flow --stream

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
# Control logging levels (note: binary name is 'nostr-eventflow' in logs)
RUST_LOG="nr2=info"                                              # Info logging
RUST_LOG="nr2=debug,nostr_relay_pool::relay::inner=warn"        # Debug with reduced relay noise
RUST_LOG="nr2=trace"                                            # Trace logging for detailed debugging
```

## Architecture

### Dual Nature: Library and Binary
- **Library** (`src/lib.rs`): Exposes public API for use in other projects
- **Binary** (`src/main.rs`): CLI tool using the library

### Core Modules
- `relay_router.rs` - Main router managing event flow between relays with builder pattern support
- `relay_manager.rs` - Low-level relay connection management, fetching, and streaming
- `fetcher.rs` - Event fetching logic with time range support
- `processor.rs` - Event processing trait and built-in processors (Passthrough, GeohashFilter)
- `state.rs` - Persistent state tracking for processed time spans
- `timespan.rs` - Time span management with automatic merging of overlapping ranges
- `config.rs` - TOML configuration parsing for sources, sinks, and processors
- `cli.rs` - Command-line argument parsing and validation

### Data Flow
1. **Sources**: Events fetched/streamed from configured source relays
2. **Processing**: Events pass through processors (implementing `Processor` trait)
3. **Routing**: Processed events are routed to specific sink relays based on configuration
4. **State Tracking**: TimeSpanSet tracks processed ranges to prevent re-processing

### Key Concepts
- **Time Spans**: Contiguous time ranges that have been processed, automatically merged when overlapping
- **Sessions**: Stream mode creates sessions tracking continuous event processing
- **Gap Detection**: Identifies missing time periods between processed spans
- **Processors**: Modular system for filtering/transforming events (must be `Send + Sync`)
- **Builder Pattern**: `RelayRouterBuilder` for flexible router configuration

## Configuration
Uses `config.toml` in the working directory. Structure:
```toml
sources = ["wss://relay1.com", "wss://relay2.com"]

[[sinks]]
processor = { type = "Passthrough" }  # or "GeohashFilter" with allowed_prefixes
relays = ["wss://sink1.com"]
```

## State Persistence
Processing state saved to `eventflow_state.json` (default) or `--state-file` path:
- Time spans collected
- Session information
- Processing statistics
- Can be disabled with `--no-state` flag

## Library Usage
When using as a library, the main entry points are:
- `RelayRouter` - Main router class
- `RelayRouterBuilder` - Builder for configuring routers
- `Processor` trait - Implement for custom processors
- Public types re-exported in `lib.rs` for convenience