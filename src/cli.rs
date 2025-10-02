use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use clap::Parser;
use nostr::Timestamp;
use std::str::FromStr;

#[derive(Parser, Debug, Clone)]
#[command(name = "nr2")]
#[command(about = "Nostr Relay Router - Routes events between relays with processing")]
pub struct Cli {
    /// Show current processing state
    #[arg(long)]
    pub show_state: bool,

    /// Use streaming collector for live events
    #[arg(long)]
    pub stream: bool,

    /// Use gap collector to fill missing time spans
    #[arg(long)]
    pub fetch_gaps: bool,

    /// Fetch events going back a specific amount of time (e.g., 5min, 4h, 3d, 1w, 2m, 1y)
    #[arg(long, value_name = "DURATION")]
    pub fetch_back: Option<String>,

    /// Fetch events starting from a specific date (yyyy-mm-dd)
    #[arg(long, value_name = "DATE")]
    pub fetch_from: Option<String>,

    /// Continuously fetch events backwards through time, stepping back by --step size
    #[arg(long)]
    pub fetch_continuous: bool,

    /// Step size for chunked fetching (e.g., 1h, 6h, 1d)
    #[arg(long, value_name = "DURATION")]
    pub step: Option<String>,

    /// Maximum number of events per fetch request
    #[arg(long, default_value = "500")]
    pub limit: usize,

    /// Wait duration between fetches (e.g., 5s, 1min, 1h)
    #[arg(long, value_name = "DURATION")]
    pub wait: Option<String>,
}

impl Cli {
    pub fn validate(&self) -> Result<()> {
        // Count mutually exclusive fetch modes
        let fetch_modes = [
            self.fetch_gaps,
            self.fetch_back.is_some(),
            self.fetch_from.is_some(),
            self.fetch_continuous,
        ]
        .iter()
        .filter(|&&x| x)
        .count();

        if fetch_modes > 1 {
            return Err(anyhow!(
                "Fetch options are mutually exclusive: choose only one of --fetch-gaps, --fetch-back, --fetch-from, or --fetch-continuous"
            ));
        }

        // Must have at least one mode
        if !self.show_state && !self.stream && fetch_modes == 0 {
            return Err(anyhow!(
                "Must specify at least one option: --show-state, --stream, or a fetch option"
            ));
        }

        // --step requires --fetch-continuous if no other fetch mode uses it
        if self.step.is_some() && !self.fetch_continuous &&
           !self.fetch_back.is_some() && !self.fetch_from.is_some() {
            return Err(anyhow!(
                "--step requires --fetch-continuous, --fetch-back, or --fetch-from"
            ));
        }

        Ok(())
    }

    pub fn get_fetch_mode(&self) -> FetchMode {
        if self.fetch_gaps {
            FetchMode::Gaps
        } else if let Some(duration) = &self.fetch_back {
            FetchMode::Back(duration.clone())
        } else if let Some(date) = &self.fetch_from {
            FetchMode::From(date.clone())
        } else if self.fetch_continuous {
            FetchMode::Continuous
        } else {
            FetchMode::None
        }
    }

    pub fn has_stream(&self) -> bool {
        self.stream
    }

    pub fn get_wait_seconds(&self) -> u64 {
        if let Some(wait_str) = &self.wait {
            parse_duration(wait_str)
                .map(|d| d.num_seconds().max(0) as u64)
                .unwrap_or(0)
        } else {
            0
        }
    }

    pub fn get_step(&self) -> Option<String> {
        self.step.clone()
    }
}

#[derive(Debug, Clone)]
pub enum FetchMode {
    None,
    Gaps,
    Back(String),  // Duration string
    From(String),  // Date string
    Continuous,    // Continuous fetching
}

/// Parse duration string like "5min", "4h", "3d", "1w", "2m", "1y"
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim().to_lowercase();

    // Extract number and unit
    let (num_str, unit) = s
        .char_indices()
        .find(|(_, c)| c.is_alphabetic())
        .ok_or_else(|| anyhow!("Invalid duration format: {}", s))
        .and_then(|(i, _)| {
            let num = &s[..i];
            let unit = &s[i..];
            Ok((num, unit))
        })?;

    let num: i64 = num_str
        .parse()
        .map_err(|_| anyhow!("Invalid number in duration: {}", num_str))?;

    let duration = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => Duration::seconds(num),
        "min" | "mins" | "minute" | "minutes" => Duration::minutes(num),
        "h" | "hr" | "hrs" | "hour" | "hours" => Duration::hours(num),
        "d" | "day" | "days" => Duration::days(num),
        "w" | "wk" | "week" | "weeks" => Duration::weeks(num),
        "m" | "mo" | "month" | "months" => Duration::days(num * 30), // Approximate
        "y" | "yr" | "year" | "years" => Duration::days(num * 365), // Approximate
        _ => return Err(anyhow!("Unknown time unit: {}", unit)),
    };

    Ok(duration)
}

/// Parse date string in yyyy-mm-dd format
pub fn parse_date(s: &str) -> Result<DateTime<Utc>> {
    NaiveDate::from_str(s)
        .map_err(|e| anyhow!("Invalid date format (expected yyyy-mm-dd): {}", e))
        .map(|date| date.and_hms_opt(0, 0, 0).unwrap())
        .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc))
}

/// Convert duration to seconds
pub fn duration_to_seconds(duration: &Duration) -> u64 {
    duration.num_seconds().max(0) as u64
}

/// Calculate timestamp from duration ago
pub fn timestamp_from_duration_ago(duration_str: &str) -> Result<Timestamp> {
    let duration = parse_duration(duration_str)?;
    let now = Timestamp::now();
    let seconds_ago = duration_to_seconds(&duration);
    Ok(Timestamp::from(now.as_u64().saturating_sub(seconds_ago)))
}

/// Calculate timestamp from date string
pub fn timestamp_from_date(date_str: &str) -> Result<Timestamp> {
    let dt = parse_date(date_str)?;
    Ok(Timestamp::from(dt.timestamp() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("5min").unwrap(), Duration::minutes(5));
        assert_eq!(parse_duration("2h").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("3d").unwrap(), Duration::days(3));
        assert_eq!(parse_duration("1w").unwrap(), Duration::weeks(1));
        assert_eq!(parse_duration("6m").unwrap(), Duration::days(6 * 30));
        assert_eq!(parse_duration("1y").unwrap(), Duration::days(365));
    }

    #[test]
    fn test_parse_date() {
        let dt = parse_date("2024-10-01").unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 10);
        assert_eq!(dt.day(), 1);
    }
}