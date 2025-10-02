use eventflow::state::ProcessingState;
use eventflow::timespan::TimeSpan;
use nostr::prelude::*;

#[test]
fn test_should_continue_fetching_after_hitting_limit() {
    // When fetching 24 days with limit=10, we should:
    // 1. Fetch first 10 events (e.g., last 4 seconds)
    // 2. Mark only those 4 seconds as collected
    // 3. Continue fetching the remaining gap (24 days minus 4 seconds)

    let mut state = ProcessingState::new();

    // Request 24 days
    let requested_start = Timestamp::from(1757327684);
    let requested_end = Timestamp::from(1759401284);

    // First fetch: got 10 events from the last 4 seconds
    let first_fetch_start = Timestamp::from(1759401278);
    let first_fetch_end = Timestamp::from(1759401282);
    state.collected_spans.add_span(first_fetch_start, first_fetch_end);

    // Check remaining gaps
    let gaps = state.collected_spans.get_gaps(requested_start, requested_end);

    // We should still have a HUGE gap to fetch
    assert!(!gaps.is_empty(), "Should have remaining gaps to fetch");

    // The remaining gap should be almost 24 days
    let remaining_gap = &gaps[0];
    let gap_duration = remaining_gap.end.as_u64() - remaining_gap.start.as_u64();

    assert!(
        gap_duration > 2073000, // More than 23.9 days
        "Remaining gap should be almost 24 days, got {} seconds",
        gap_duration
    );

    // The gap should go from requested_start to just before first_fetch_start
    assert_eq!(remaining_gap.start, requested_start);
    assert_eq!(remaining_gap.end, first_fetch_start);
}

#[test]
fn test_iterative_fetch_simulation() {
    // Simulate what SHOULD happen with iterative fetching

    let mut state = ProcessingState::new();

    // Request 24 days
    let requested_start = Timestamp::from(1757327684);
    let requested_end = Timestamp::from(1759401284);
    let total_duration = requested_end.as_u64() - requested_start.as_u64();

    // Simulate multiple fetches (each getting 10 events covering ~5 seconds)
    let mut iterations = 0;
    let mut current_coverage = 0u64;

    loop {
        let gaps = state.collected_spans.get_gaps(requested_start, requested_end);

        if gaps.is_empty() {
            break; // All fetched
        }

        iterations += 1;

        // Simulate fetching the first gap
        let gap = &gaps[0];

        // Simulate: we fetch and get events from the end of the gap (most recent)
        // covering about 5 seconds
        let fetch_duration = 5u64.min(gap.end.as_u64() - gap.start.as_u64());
        let fetch_start = gap.end.as_u64() - fetch_duration;
        let fetch_end = gap.end.as_u64();

        state.collected_spans.add_span(
            Timestamp::from(fetch_start),
            Timestamp::from(fetch_end),
        );

        current_coverage += fetch_duration;

        // Safety check to prevent infinite loop in test
        if iterations > 100000 {
            panic!("Too many iterations - something is wrong");
        }

        // For testing, simulate only a few iterations
        if iterations >= 3 {
            break;
        }
    }

    // After 3 iterations, we should have fetched about 15 seconds
    let total_coverage = state.collected_spans.total_coverage_seconds();
    assert!(
        total_coverage >= 15 && total_coverage <= 20,
        "Should have fetched about 15 seconds, got {}",
        total_coverage
    );

    // We should still have gaps
    let remaining_gaps = state.collected_spans.get_gaps(requested_start, requested_end);
    assert!(
        !remaining_gaps.is_empty(),
        "Should still have gaps after 3 iterations"
    );

    println!(
        "After {} iterations: covered {} of {} seconds ({:.2}%)",
        iterations,
        total_coverage,
        total_duration,
        (total_coverage as f64 / total_duration as f64) * 100.0
    );
}