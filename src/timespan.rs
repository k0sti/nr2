use nostr::Timestamp;
use serde::{Deserialize, Serialize};

/// A collection of time spans that tracks which time periods have been processed.
/// Automatically merges overlapping or adjacent spans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSpanSet {
    /// Sorted list of non-overlapping time ranges
    spans: Vec<TimeSpan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeSpan {
    pub start: Timestamp,
    pub end: Timestamp,
}

impl TimeSpan {
    pub fn new(start: Timestamp, end: Timestamp) -> Self {
        assert!(start <= end, "TimeSpan start must be <= end");
        Self { start, end }
    }

    /// Check if two spans overlap or are adjacent (can be merged)
    pub fn can_merge_with(&self, other: &TimeSpan) -> bool {
        // Spans can merge if they overlap or are exactly adjacent
        !(self.end < other.start || other.end < self.start)
    }

    /// Merge two overlapping or adjacent spans
    pub fn merge(&self, other: &TimeSpan) -> TimeSpan {
        TimeSpan {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Check if a timestamp is within this span
    #[allow(dead_code)]
    pub fn contains(&self, timestamp: Timestamp) -> bool {
        timestamp >= self.start && timestamp <= self.end
    }
}

impl TimeSpanSet {
    pub fn new() -> Self {
        Self { spans: Vec::new() }
    }

    /// Add a new time span, merging with existing spans if they overlap
    pub fn add_span(&mut self, start: Timestamp, end: Timestamp) {
        let new_span = TimeSpan::new(start, end);

        if self.spans.is_empty() {
            self.spans.push(new_span);
            return;
        }

        let mut result = Vec::new();
        let mut merged = new_span.clone();
        let mut merged_any = false;

        for span in &self.spans {
            if span.can_merge_with(&merged) {
                merged = span.merge(&merged);
                merged_any = true;
            } else if span.end < merged.start {
                // This span comes before the merged span
                result.push(span.clone());
            } else {
                // This span comes after the merged span
                if !merged_any {
                    result.push(merged.clone());
                    merged_any = true;
                }
                result.push(span.clone());
            }
        }

        if !merged_any || result.is_empty() || result.last().unwrap().end < merged.start {
            result.push(merged);
        }

        self.spans = result;
        self.sort_and_merge();
    }

    /// Sort spans and merge any that became adjacent
    fn sort_and_merge(&mut self) {
        if self.spans.is_empty() {
            return;
        }

        self.spans.sort_by_key(|s| s.start);

        let mut merged = Vec::new();
        let mut current = self.spans[0].clone();

        for span in &self.spans[1..] {
            if current.can_merge_with(span) {
                current = current.merge(span);
            } else {
                merged.push(current);
                current = span.clone();
            }
        }
        merged.push(current);

        self.spans = merged;
    }

    /// Check if a timestamp has been processed
    #[allow(dead_code)]
    pub fn contains(&self, timestamp: Timestamp) -> bool {
        self.spans.iter().any(|span| span.contains(timestamp))
    }

    /// Get gaps (uncollected time spans) between a start and end time
    pub fn get_gaps(&self, start: Timestamp, end: Timestamp) -> Vec<TimeSpan> {
        let mut gaps = Vec::new();
        let mut current_start = start;

        for span in &self.spans {
            if span.start > end {
                break;
            }

            if span.end < start {
                continue;
            }

            if current_start < span.start {
                gaps.push(TimeSpan::new(
                    current_start,
                    span.start.min(end),
                ));
            }

            current_start = span.end.max(current_start);
        }

        if current_start < end {
            gaps.push(TimeSpan::new(current_start, end));
        }

        gaps
    }

    /// Get the earliest timestamp that has been processed
    pub fn earliest(&self) -> Option<Timestamp> {
        self.spans.first().map(|s| s.start)
    }

    /// Get the latest timestamp that has been processed
    pub fn latest(&self) -> Option<Timestamp> {
        self.spans.last().map(|s| s.end)
    }

    /// Get total number of spans (for diagnostics)
    pub fn span_count(&self) -> usize {
        self.spans.len()
    }

    /// Get all spans (for diagnostics/display)
    pub fn spans(&self) -> &[TimeSpan] {
        &self.spans
    }

    /// Calculate total coverage duration
    pub fn total_coverage_seconds(&self) -> u64 {
        self.spans
            .iter()
            .map(|s| s.end.as_u64().saturating_sub(s.start.as_u64()))
            .sum()
    }
}

impl Default for TimeSpanSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_adjacent_spans() {
        let mut set = TimeSpanSet::new();
        set.add_span(Timestamp::from(100), Timestamp::from(200));
        set.add_span(Timestamp::from(200), Timestamp::from(300));

        assert_eq!(set.span_count(), 1);
        assert_eq!(set.spans()[0].start, Timestamp::from(100));
        assert_eq!(set.spans()[0].end, Timestamp::from(300));
    }

    #[test]
    fn test_merge_overlapping_spans() {
        let mut set = TimeSpanSet::new();
        set.add_span(Timestamp::from(100), Timestamp::from(250));
        set.add_span(Timestamp::from(200), Timestamp::from(300));

        assert_eq!(set.span_count(), 1);
        assert_eq!(set.spans()[0].start, Timestamp::from(100));
        assert_eq!(set.spans()[0].end, Timestamp::from(300));
    }

    #[test]
    fn test_get_gaps() {
        let mut set = TimeSpanSet::new();
        set.add_span(Timestamp::from(100), Timestamp::from(200));
        set.add_span(Timestamp::from(300), Timestamp::from(400));

        let gaps = set.get_gaps(Timestamp::from(0), Timestamp::from(500));
        assert_eq!(gaps.len(), 3);
        assert_eq!(gaps[0].start, Timestamp::from(0));
        assert_eq!(gaps[0].end, Timestamp::from(100));
        assert_eq!(gaps[1].start, Timestamp::from(200));
        assert_eq!(gaps[1].end, Timestamp::from(300));
        assert_eq!(gaps[2].start, Timestamp::from(400));
        assert_eq!(gaps[2].end, Timestamp::from(500));
    }
}