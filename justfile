# Show available commands
default:
    @just --list

# Run the relay router with streaming (default)
run:
    cargo run -- --stream

# Stream live events
stream:
    cargo run -- --stream

# Fill gaps in existing data
gaps:
    cargo run -- --fetch-gaps

# Fetch last 1 hour of events
fetch-1h:
    cargo run -- --fetch-back 1h

# Fetch last 1 day of events
fetch-1d:
    cargo run -- --fetch-back 1d

# Fetch from specific date
fetch-from date:
    cargo run -- --fetch-from {{date}}

# Fetch from specific date with chunking
fetch-from-chunked date step:
    cargo run -- --fetch-from {{date}} --step {{step}}

# Continuously fetch backwards through time with 1 hour steps
fetch-continuous:
    cargo run -- --fetch-continuous --step 1h

# Continuously fetch backwards with wait between fetches
fetch-continuous-slow:
    cargo run -- --fetch-continuous --step 1h --wait 5s

# Stream and fill gaps concurrently
stream-and-gaps:
    cargo run -- --stream --fetch-gaps

# Run with debug logging (suppresses nostr-sdk error spam)
debug:
    RUST_LOG="nr2=debug,nostr_relay_pool::relay::inner=warn,nostr_relay_pool=info" cargo run -- --stream

# Show current state in human-readable format
show-state:
    cargo run -- --show-state

# Listen to received events on local Nostr relay
listen:
    nak req ws://localhost:3330 --stream

# Test run with debug output (first 50 lines, suppresses nostr-sdk errors)
test:
    RUST_LOG="nr2=debug,nostr_relay_pool::relay::inner=warn,nostr_relay_pool=info" cargo run 2>&1 | head -50

# Run with all debug output including nostr-sdk errors
debug-verbose:
    RUST_LOG="nr2=debug,nostr_relay_pool=debug" cargo run

# Build the project
build:
    cargo build

# Check for compilation errors
check:
    cargo check
