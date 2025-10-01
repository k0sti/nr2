# Show available commands
default:
    @just --list

# Run the relay router
run:
    cargo run

# Run with debug logging
debug:
    RUST_LOG=nr2=debug,nostr_relay_pool=info cargo run

# Show current state file
show-state:
    @test -f nr2_state.json && cat nr2_state.json | jq . || echo "No state file found"

# Listen to received events on local Nostr relay
listen:
    nak req ws://localhost:3330 --stream
