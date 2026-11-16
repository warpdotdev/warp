use chrono::{DateTime, Utc};

/// Parse an RFC 3339 timestamp into a UTC `DateTime`.
pub(crate) fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| format!("invalid RFC 3339 timestamp '{s}': {e}"))
}
