# Show available commands
default:
    @just --list

help:
    cargo run -- --help

# Run the relay router with streaming (default)
run:
    cargo run -- --stream

# Fill gaps in existing data
gaps:
    cargo run -- --fetch-gaps

# Listen to received events on local Nostr relay
listen:
    nak req ws://localhost:3330 --stream
