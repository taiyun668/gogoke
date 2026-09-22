use std::{fmt, str::FromStr};

use chrono::{DateTime, FixedOffset};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// A cross-TypeScript-safe unsigned counter. It is always encoded as a
/// canonical base-10 JSON string and never as a JSON number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalU64(u64);

impl DecimalU64 {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for DecimalU64 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for DecimalU64 {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "0" {
            return Ok(Self::ZERO);
        }
        if value.is_empty()
            || value.starts_with('0')
            || !value.as_bytes().iter().all(u8::is_ascii_digit)
        {
            return Err("NON_CANONICAL_UNSIGNED_DECIMAL");
        }
        value
            .parse::<u64>()
            .map(Self)
            .map_err(|_| "UNSIGNED_DECIMAL_OVERFLOW")
    }
}

impl Serialize for DecimalU64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DecimalU64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

/// Canonical lower-case hyphenated UUID used as an opaque product identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpaqueId(String);

impl OpaqueId {
    pub fn parse(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        let parsed = Uuid::parse_str(&value).map_err(|_| "INVALID_OPAQUE_ID")?;
        if parsed.hyphenated().to_string() != value {
            return Err("NON_CANONICAL_OPAQUE_ID");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for OpaqueId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for OpaqueId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// RFC3339 timestamp whose offset is UTC. The original valid spelling is
/// retained; timeouts and leases must still use a monotonic clock elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTimestamp(String);

impl UtcTimestamp {
    pub fn parse(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if !is_canonical_utc_timestamp(&value) {
            return Err("INVALID_RFC3339_TIMESTAMP");
        }
        let parsed: DateTime<FixedOffset> =
            DateTime::parse_from_rfc3339(&value).map_err(|_| "INVALID_RFC3339_TIMESTAMP")?;
        if parsed.offset().local_minus_utc() != 0 {
            return Err("TIMESTAMP_NOT_UTC");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_canonical_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    let timestamp_end = if bytes.ends_with(b"Z") {
        bytes.len().checked_sub(1)
    } else if bytes.ends_with(b"+00:00") {
        bytes.len().checked_sub(6)
    } else {
        None
    };
    let Some(timestamp_end) = timestamp_end else {
        return false;
    };
    if timestamp_end < 19
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return false;
    }
    for index in [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !bytes.get(index).is_some_and(u8::is_ascii_digit) {
            return false;
        }
    }
    if timestamp_end > 19
        && (bytes.get(19) != Some(&b'.')
            || timestamp_end == 20
            || !bytes[20..timestamp_end].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    if timestamp_end != 19 && timestamp_end <= 20 {
        return false;
    }

    let parse_pair = |start: usize| -> u32 {
        u32::from(bytes[start] - b'0') * 10 + u32::from(bytes[start + 1] - b'0')
    };
    let year = u32::from(bytes[0] - b'0') * 1000
        + u32::from(bytes[1] - b'0') * 100
        + u32::from(bytes[2] - b'0') * 10
        + u32::from(bytes[3] - b'0');
    let month = parse_pair(5);
    let day = parse_pair(8);
    let hour = parse_pair(11);
    let minute = parse_pair(14);
    let second = parse_pair(17);
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = [
        31,
        if leap_year { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=12).contains(&month)
        && day >= 1
        && day <= days_in_month[(month - 1) as usize]
        && hour <= 23
        && minute <= 59
        && second <= 59
}

impl Serialize for UtcTimestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for UtcTimestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}
