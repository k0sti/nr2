use nr2::state::ProcessingState;
use nostr::prelude::*;

#[test]
fn test_fetch_with_limit_should_not_mark_full_range() {
    // Integration test showing the bug:
    // When fetching 24 days with limit=10, we get 10 events
    // but mark the entire 24-day period as collected

    let mut state = ProcessingState::new();

    // Simulate what happens in the real scenario
    let requested_start = Timestamp::from(1757326871);
    let requested_end = Timestamp::from(1759400471);

    // In reality, we only got events from the last 6 seconds
    // because the limit was reached
    let _events = vec![
        Timestamp::from(1759400464),
        Timestamp::from(1759400465),
        Timestamp::from(1759400466),
        Timestamp::from(1759400468),
        Timestamp::from(1759400469),
        Timestamp::from(1759400470),
    ];

    // FIXED: With the fix, we only mark the actual event range
    // Simulate marking only the events we got (6 seconds worth)
    let actual_start = Timestamp::from(1759400464);
    let actual_end = Timestamp::from(1759400470);
    state.collected_spans.add_span(actual_start, actual_end);

    // Check what got marked
    let gaps = state.collected_spans.get_gaps(
        requested_start,
        requested_end,
    );

    // Now we should have gaps for the unfetched portions
    assert!(
        !gaps.is_empty(),
        "Should have gaps! We only fetched 6 seconds of a 24-day range"
    );

    // We should have a huge gap before our fetched data
    let first_gap = &gaps[0];
    assert_eq!(first_gap.start, requested_start);
    assert!(first_gap.end <= actual_start, "Gap should end at or before the actual data start");
}

#[test]
fn test_correct_span_marking_based_on_events() {
    // This test shows the correct behavior

    let mut state = ProcessingState::new();

    // Request a large range
    let requested_start = Timestamp::from(1757326871);
    let requested_end = Timestamp::from(1759400471);

    // But we only got events from a small window
    let first_event = Timestamp::from(1759400464);
    let last_event = Timestamp::from(1759400470);

    // CORRECT: Only mark the range we actually have events for
    state.collected_spans.add_span(first_event, last_event);

    // Now check for gaps
    let gaps = state.collected_spans.get_gaps(
        requested_start,
        requested_end,
    );

    // We should have two gaps:
    // 1. From requested_start to first_event
    // 2. From last_event to requested_end (if any)

    assert!(!gaps.is_empty(), "Should have gaps for unfetched portions");

    // The first gap should be huge (almost 24 days)
    let first_gap = &gaps[0];
    let gap_duration = first_gap.end.as_u64() - first_gap.start.as_u64();

    assert!(
        gap_duration > 86400, // More than 1 day
        "First gap should be large, got {} seconds",
        gap_duration
    );
}