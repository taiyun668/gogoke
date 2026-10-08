//! Host-side interpretation of one authenticated USER time rule. This module
//! does not authorize a tool call, read a clock, or schedule a wakeup.
use chrono::{
    DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc,
};
use chrono_tz::Tz;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostSchedule {
    pub(crate) timezone: String,
    pub(crate) next_due_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScheduleError {
    EmptyRule,
    SourceMismatch,
    ConflictingRule,
    ConflictingTimezone,
    MissingTimezone,
    InvalidTimezone(String),
    UnsupportedRule,
    InvalidTime,
    InvalidDate,
    InvalidInterval,
    AmbiguousLocalTime,
    NonexistentLocalTime,
    TimeOutOfRange,
    DueNotAfterSource,
}

fn zone_alias(value: &str) -> Option<&'static str> {
    match value {
        "北京时间" | "中国标准时间" | "Asia/Shanghai" => Some("Asia/Shanghai"),
        "UTC" | "Etc/UTC" => Some("UTC"),
        _ => None,
    }
}

fn zone(value: &str) -> Result<(Tz, String), ScheduleError> {
    let canonical = zone_alias(value).unwrap_or(value);
    let parsed = canonical
        .parse::<Tz>()
        .map_err(|_| ScheduleError::InvalidTimezone(value.to_owned()))?;
    Ok((parsed, parsed.to_string()))
}

fn explicit_zones(text: &str) -> Result<Vec<String>, ScheduleError> {
    let mut found = Vec::new();
    for alias in ["北京时间", "中国标准时间"] {
        if text.contains(alias) {
            found.push(alias.to_owned());
        }
    }
    for word in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | '，' | '。' | ';' | '；' | '(' | ')' | '（' | '）' | '[' | ']' | '：' | ':'
            )
    }) {
        let candidate = word.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '.' | '!'));
        if (candidate.contains('/')
            && candidate
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic()))
            || candidate == "UTC"
        {
            zone(candidate)?;
            found.push(candidate.to_owned());
        } else if ["PST", "PDT", "EST", "EDT", "CST", "CDT", "IST", "GMT"].contains(&candidate) {
            return Err(ScheduleError::InvalidTimezone(candidate.to_owned()));
        }
    }
    Ok(found)
}

fn selected_zone(
    original: &str,
    proposed: &str,
    host: &str,
) -> Result<(Tz, String), ScheduleError> {
    let names = explicit_zones(original)?;
    let canonical_names: Result<Vec<_>, _> = names
        .iter()
        .map(|name| zone(name).map(|(_, name)| name))
        .collect();
    let canonical_names = canonical_names?;
    if canonical_names.windows(2).any(|pair| pair[0] != pair[1]) {
        return Err(ScheduleError::ConflictingTimezone);
    }
    if proposed == "HOST_DEFAULT" {
        if !names.is_empty() {
            return Err(ScheduleError::ConflictingTimezone);
        }
        if host.is_empty() {
            return Err(ScheduleError::MissingTimezone);
        }
        return zone(host);
    }
    if !original.contains(proposed) {
        return Err(ScheduleError::SourceMismatch);
    }
    let chosen = zone(proposed)?;
    if canonical_names.is_empty()
        || canonical_names
            .iter()
            .any(|name| name.as_str() != chosen.1.as_str())
    {
        return Err(ScheduleError::ConflictingTimezone);
    }
    Ok(chosen)
}

fn at_local(tz: Tz, date: NaiveDate, time: NaiveTime) -> Result<i64, ScheduleError> {
    let local = NaiveDateTime::new(date, time);
    match tz.from_local_datetime(&local) {
        chrono::LocalResult::Single(value) => Ok(value.timestamp_millis()),
        chrono::LocalResult::Ambiguous(_, _) => Err(ScheduleError::AmbiguousLocalTime),
        chrono::LocalResult::None => Err(ScheduleError::NonexistentLocalTime),
    }
}

fn time(value: &str) -> Result<NaiveTime, ScheduleError> {
    let mut pieces = value.split(':');
    let h = pieces.next().ok_or(ScheduleError::InvalidTime)?;
    let m = pieces.next().ok_or(ScheduleError::InvalidTime)?;
    if pieces.next().is_some()
        || h.len() > 2
        || m.len() != 2
        || !h.bytes().all(|b| b.is_ascii_digit())
        || !m.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(ScheduleError::InvalidTime);
    }
    NaiveTime::from_hms_opt(
        h.parse().map_err(|_| ScheduleError::InvalidTime)?,
        m.parse().map_err(|_| ScheduleError::InvalidTime)?,
        0,
    )
    .ok_or(ScheduleError::InvalidTime)
}

fn interval(input: &str, prefix: &str, suffix: &str) -> Option<Result<i64, ScheduleError>> {
    let body = input.strip_prefix(prefix)?.strip_suffix(suffix)?.trim();
    if !body.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let (number, multiplier) = if let Some(n) = body.strip_suffix("秒") {
        (n, 1_000)
    } else if let Some(n) = body.strip_suffix("分钟") {
        (n, 60_000)
    } else if let Some(n) = body.strip_suffix("小时") {
        (n, 3_600_000)
    } else {
        return Some(Err(ScheduleError::UnsupportedRule));
    };
    Some(
        number
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .and_then(|n| n.checked_mul(multiplier))
            .ok_or(ScheduleError::InvalidInterval),
    )
}

fn english_interval(input: &str, prefix: &str) -> Option<Result<i64, ScheduleError>> {
    let rest = input.strip_prefix(prefix)?.trim();
    let mut words = rest.split_whitespace();
    let n = words.next()?;
    let unit = words.next()?;
    if words.next().is_some() {
        return Some(Err(ScheduleError::ConflictingRule));
    }
    let scale = match unit {
        "second" | "seconds" => 1_000,
        "minute" | "minutes" => 60_000,
        "hour" | "hours" => 3_600_000,
        _ => return Some(Err(ScheduleError::UnsupportedRule)),
    };
    Some(
        n.parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .and_then(|n| n.checked_mul(scale))
            .ok_or(ScheduleError::InvalidInterval),
    )
}

fn has_clock(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(5).any(|part| {
        part[0].is_ascii_digit()
            && part[1].is_ascii_digit()
            && part[2] == b':'
            && part[3].is_ascii_digit()
            && part[4].is_ascii_digit()
    })
}

fn weekday(value: &str) -> Option<chrono::Weekday> {
    use chrono::Weekday::*;
    Some(match value {
        "一" | "1" | "Monday" | "monday" => Mon,
        "二" | "2" | "Tuesday" | "tuesday" => Tue,
        "三" | "3" | "Wednesday" | "wednesday" => Wed,
        "四" | "4" | "Thursday" | "thursday" => Thu,
        "五" | "5" | "Friday" | "friday" => Fri,
        "六" | "6" | "Saturday" | "saturday" => Sat,
        "日" | "天" | "7" | "Sunday" | "sunday" => Sun,
        _ => return None,
    })
}

fn calendar(
    rule: &str,
    tz: Tz,
    source: DateTime<Utc>,
    explicit_zone: bool,
) -> Result<i64, ScheduleError> {
    let local = source.with_timezone(&tz);
    let lower = rule.to_ascii_lowercase();
    let daily = rule
        .strip_prefix("每天")
        .or_else(|| lower.strip_prefix("daily at "));
    if let Some(clock) = daily {
        let clock = time(clock.trim())?;
        let today = at_local(tz, local.date_naive(), clock)?;
        if today > source.timestamp_millis() {
            return Ok(today);
        }
        let tomorrow = local
            .date_naive()
            .succ_opt()
            .ok_or(ScheduleError::TimeOutOfRange)?;
        return at_local(tz, tomorrow, clock);
    }
    let tomorrow = rule
        .strip_prefix("明天")
        .or_else(|| lower.strip_prefix("tomorrow at "));
    if let Some(clock) = tomorrow {
        let date = local
            .date_naive()
            .succ_opt()
            .ok_or(ScheduleError::TimeOutOfRange)?;
        return at_local(tz, date, time(clock.trim())?);
    }
    let weekly = rule
        .strip_prefix("每周")
        .or_else(|| rule.strip_prefix("每星期"));
    let weekly_english = lower.strip_prefix("weekly on ");
    if let Some(rest) = weekly.or(weekly_english) {
        let rest = rest.trim();
        let (day, clock) = if weekly_english.is_some() {
            rest.split_once(" at ").ok_or(ScheduleError::InvalidTime)?
        } else {
            let first = rest.chars().next().ok_or(ScheduleError::InvalidTime)?;
            if weekday(&first.to_string()).is_some() {
                (&rest[..first.len_utf8()], rest[first.len_utf8()..].trim())
            } else {
                rest.split_once(char::is_whitespace)
                    .ok_or(ScheduleError::InvalidTime)?
            }
        };
        let day = weekday(day).ok_or(ScheduleError::UnsupportedRule)?;
        let clock = time(clock.trim())?;
        let delta = (7 + day.num_days_from_monday() as i64
            - local.weekday().num_days_from_monday() as i64)
            % 7;
        let date = local
            .date_naive()
            .checked_add_signed(Duration::days(delta))
            .ok_or(ScheduleError::TimeOutOfRange)?;
        let due = at_local(tz, date, clock)?;
        if due > source.timestamp_millis() {
            return Ok(due);
        }
        return at_local(
            tz,
            date.checked_add_signed(Duration::days(7))
                .ok_or(ScheduleError::TimeOutOfRange)?,
            clock,
        );
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(rule) {
        if explicit_zone
            && parsed.offset().local_minus_utc()
                != tz
                    .offset_from_utc_datetime(&parsed.naive_utc())
                    .fix()
                    .local_minus_utc()
        {
            return Err(ScheduleError::ConflictingTimezone);
        }
        return Ok(parsed.timestamp_millis());
    }
    let (date, clock) = rule
        .split_once(' ')
        .or_else(|| rule.split_once('T'))
        .ok_or(ScheduleError::UnsupportedRule)?;
    if date.contains('年') {
        let (year, rest) = date.split_once('年').ok_or(ScheduleError::InvalidDate)?;
        let (month, day) = rest.split_once('月').ok_or(ScheduleError::InvalidDate)?;
        let day = day.strip_suffix('日').ok_or(ScheduleError::InvalidDate)?;
        let date = NaiveDate::from_ymd_opt(
            year.parse().map_err(|_| ScheduleError::InvalidDate)?,
            month.parse().map_err(|_| ScheduleError::InvalidDate)?,
            day.parse().map_err(|_| ScheduleError::InvalidDate)?,
        )
        .ok_or(ScheduleError::InvalidDate)?;
        return at_local(tz, date, time(clock.trim())?);
    }
    if date.len() != 10
        || !date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return Err(ScheduleError::UnsupportedRule);
    }
    let date =
        NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| ScheduleError::InvalidDate)?;
    at_local(tz, date, time(clock.trim())?)
}

/// The caller separately compares `next_due_ms` with trusted current time.
/// Relative and periodic first occurrences are anchored to the stored USER
/// input timestamp, never to a tool invocation or model supplied timestamp.
pub(crate) fn resolve_user_schedule(
    original_user: &str,
    exact_span: &str,
    proposed_zone: &str,
    source_input_ms: i64,
    host_timezone: &str,
) -> Result<HostSchedule, ScheduleError> {
    if exact_span.trim().is_empty() {
        return Err(ScheduleError::EmptyRule);
    }
    let Some(position) = original_user.find(exact_span) else {
        return Err(ScheduleError::SourceMismatch);
    };
    if original_user[position + exact_span.len()..].contains(exact_span) {
        return Err(ScheduleError::SourceMismatch);
    }
    let prefix: String = original_user[..position]
        .chars()
        .rev()
        .take(16)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let prefix_lower = prefix.to_ascii_lowercase();
    if ["不要", "别", "不是", "取消", "不按"]
        .iter()
        .any(|word| prefix.contains(word))
        || ["not ", "don't ", "do not ", "instead of "]
            .iter()
            .any(|word| prefix_lower.contains(word))
    {
        return Err(ScheduleError::ConflictingRule);
    }
    let (tz, timezone) = selected_zone(original_user, proposed_zone, host_timezone)?;
    if source_input_ms <= 0 {
        return Err(ScheduleError::TimeOutOfRange);
    }
    let source = DateTime::<Utc>::from_timestamp_millis(source_input_ms)
        .ok_or(ScheduleError::TimeOutOfRange)?;
    let mut rule = exact_span
        .trim()
        .trim_matches(|c: char| matches!(c, '，' | ',' | '。' | '.'));
    if proposed_zone != "HOST_DEFAULT" {
        rule = rule.trim_end_matches(proposed_zone).trim();
    }
    if rule.is_empty() {
        return Err(ScheduleError::EmptyRule);
    }
    let outside = format!(
        "{} {}",
        &original_user[..position],
        &original_user[position + exact_span.len()..]
    );
    if [
        "每天",
        "每周",
        "每星期",
        "明天",
        "daily at ",
        "weekly on ",
        "tomorrow at ",
        "every ",
        "in ",
    ]
    .iter()
    .any(|token| outside.contains(token))
        || outside.contains("分钟后")
        || outside.contains("小时后")
        || outside.contains("秒后")
        || has_clock(&outside)
    {
        return Err(ScheduleError::ConflictingRule);
    }
    let due = if let Some(result) = interval(rule, "", "后")
        .or_else(|| interval(rule, "每", ""))
        .or_else(|| english_interval(rule, "in "))
        .or_else(|| english_interval(rule, "every "))
    {
        source_input_ms
            .checked_add(result?)
            .ok_or(ScheduleError::TimeOutOfRange)?
    } else {
        calendar(rule, tz, source, proposed_zone != "HOST_DEFAULT")?
    };
    if due <= source_input_ms {
        return Err(ScheduleError::DueNotAfterSource);
    }
    Ok(HostSchedule {
        timezone,
        next_due_ms: due,
    })
}

#[cfg(test)]
#[path = "secretary_schedule_tests.rs"]
mod tests;
