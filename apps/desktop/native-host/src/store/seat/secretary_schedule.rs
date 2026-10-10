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

fn chinese_zone_prefix(text: &str) -> Option<&str> {
    ["时间", "時間", "时区", "時區"].iter().find_map(|suffix| {
        let index = text.find(*suffix)?;
        let name = &text[..index];
        (!name.is_empty() && name.chars().all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)))
            .then_some(&text[..index + suffix.len()])
    })
}

// A standalone clause is an explicit zone declaration even when it follows
// the task instead of the time token. Unknown declarations cannot select the
// host timezone by omission.
fn clause_zone(clause: &str) -> Option<&str> {
    let clause = clause
        .trim()
        .trim_end_matches(|c: char| matches!(c, '.' | '!' | '！'));
    let clause = ["按照", "按", "使用", "用", "以"]
        .iter()
        .find_map(|prefix| clause.strip_prefix(*prefix))
        .unwrap_or(clause)
        .trim();
    if clause.is_empty() {
        return None;
    }
    let named_chinese_zone = chinese_zone_prefix(clause).is_some_and(|name| name == clause);
    let named_english_zone = clause
        .split_once(' ')
        .is_some_and(|(name, suffix)| !name.is_empty() && matches!(suffix, "time" | "timezone"));
    let abbreviation =
        (2..=5).contains(&clause.len()) && clause.bytes().all(|byte| byte.is_ascii_uppercase());
    (named_chinese_zone || named_english_zone || abbreviation).then_some(clause)
}

fn explicit_zones(text: &str) -> Result<Vec<String>, ScheduleError> {
    let mut found = Vec::new();
    // Explicit prepositions identify the original zone phrase before a
    // complete task clause can be mistaken for that phrase in diagnostics.
    for (index, _) in text.char_indices() {
        let tail = &text[index..];
        if let Some(after_preposition) = ["按照", "按", "使用", "用", "以"]
            .iter()
            .find_map(|prefix| tail.strip_prefix(*prefix))
        {
            if let Some(name) = chinese_zone_prefix(after_preposition) {
                zone(name)?;
                found.push(name.to_owned());
            }
        }
    }
    for clause in text.split(|c| matches!(c, ',' | '，' | ';' | '；' | '。')) {
        if let Some(name) = clause_zone(clause) {
            zone(name)?;
            found.push(name.to_owned());
        }
    }
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
        let iana_area = candidate.split_once('/').is_some_and(|(area, _)| {
            matches!(
                area,
                "Africa"
                    | "America"
                    | "Antarctica"
                    | "Arctic"
                    | "Asia"
                    | "Atlantic"
                    | "Australia"
                    | "Europe"
                    | "Indian"
                    | "Pacific"
                    | "Etc"
                    | "US"
                    | "Canada"
            )
        });
        if iana_area || candidate == "UTC" {
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

fn clock_prefix(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if !(1..=2).contains(&digits) || bytes.get(digits) != Some(&b':') {
        return None;
    }
    if bytes
        .get(digits + 1..digits + 3)?
        .iter()
        .all(u8::is_ascii_digit)
    {
        Some(digits + 3)
    } else {
        None
    }
}

fn after_clock(input: &str, prefix: &str) -> Option<usize> {
    let rest = input.strip_prefix(prefix)?;
    let spaces = rest.len() - rest.trim_start().len();
    Some(prefix.len() + spaces + clock_prefix(&rest[spaces..])?)
}

fn strip_ascii_prefix<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    input
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &input[prefix.len()..])
}

fn numeric_relative_prefix(input: &str) -> bool {
    let digits = input.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return false;
    }
    ["秒", "分钟", "小时", "天", "周"].iter().any(|unit| {
        input[digits..].strip_prefix(*unit).is_some_and(|tail| {
            ["后", "以后", "之后", "前", "以前", "之前"]
                .iter()
                .any(|suffix| tail.starts_with(*suffix))
        })
    })
}

// Only the beginning of the authenticated USER command can designate a rule.
// This grammar selects complete numeric and calendar tokens before examining
// the model's proposal; a substring such as 1分钟后 inside 11分钟后 is never a rule.
fn rule_prefix(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 {
        for unit in ["秒后", "分钟后", "小时后"] {
            if input[digits..].starts_with(unit) {
                return Some(digits + unit.len());
            }
        }
    }
    if let Some(rest) = input.strip_prefix('每') {
        let n = rest.bytes().take_while(u8::is_ascii_digit).count();
        if n > 0 {
            for unit in ["秒", "分钟", "小时"] {
                if rest[n..].starts_with(unit) {
                    return Some("每".len() + n + unit.len());
                }
            }
        }
    }
    for prefix in ["每天", "明天", "daily at ", "tomorrow at "] {
        if let Some(end) = after_clock(input, prefix) {
            return Some(end);
        }
    }
    if input.starts_with("每天") || input.starts_with("明天") {
        return input.split_whitespace().next().map(str::len);
    }
    for prefix in ["每周", "每星期", "weekly on "] {
        if let Some(rest) = input.strip_prefix(prefix) {
            let day = if prefix == "weekly on " {
                rest.split_once(" at ")?.0
            } else {
                let first = rest.chars().next()?;
                if weekday(&first.to_string()).is_some() {
                    &rest[..first.len_utf8()]
                } else {
                    rest.split_once(char::is_whitespace)?.0
                }
            };
            if weekday(day).is_some() {
                let tail = &rest[day.len()..];
                let tail = if prefix == "weekly on " {
                    tail.strip_prefix(" at ")?
                } else {
                    tail
                };
                let spaces = tail.len() - tail.trim_start().len();
                return Some(
                    prefix.len()
                        + day.len()
                        + if prefix == "weekly on " { 4 } else { 0 }
                        + spaces
                        + clock_prefix(&tail[spaces..])?,
                );
            }
        }
    }
    for prefix in ["in ", "every "] {
        if let Some(rest) = input.strip_prefix(prefix) {
            let n = rest.bytes().take_while(u8::is_ascii_digit).count();
            if n > 0 && rest[n..].starts_with(' ') {
                let unit = rest[n + 1..]
                    .split(|c: char| !c.is_ascii_alphabetic())
                    .next()?;
                if matches!(
                    unit,
                    "second" | "seconds" | "minute" | "minutes" | "hour" | "hours"
                ) {
                    return Some(prefix.len() + n + 1 + unit.len());
                }
            }
        }
    }
    // RFC3339 is one token. The date forms then consume one complete clock.
    let token = input.split_whitespace().next()?;
    if DateTime::parse_from_rfc3339(token).is_ok() {
        return Some(token.len());
    }
    let date_end = if input.len() >= 10 && input.as_bytes().get(4) == Some(&b'-') {
        10
    } else {
        input.find('日')? + '日'.len_utf8()
    };
    let tail = input.get(date_end..)?;
    let spaces = tail.len() - tail.trim_start().len();
    Some(date_end + spaces + clock_prefix(&tail[spaces..])?)
}

fn command_rule(original: &str) -> Result<&str, ScheduleError> {
    let text = original.trim();
    if text.starts_with("不要") && !text.starts_with("不要忘了")
        || text.starts_with("取消")
        || text.starts_with("别提醒")
    {
        return Err(ScheduleError::ConflictingRule);
    }
    let english = strip_ascii_prefix(text, "please ").unwrap_or(text);
    let (command, positive_idiom) = if let Some(rest) = strip_ascii_prefix(english, "remind me ")
        .or_else(|| strip_ascii_prefix(english, "notify me "))
        .or_else(|| strip_ascii_prefix(english, "don't forget to "))
    {
        (rest.trim_start(), true)
    } else {
        ["请在", "在", "请帮我", "帮我", "请", "不要忘了", "别忘了"]
            .iter()
            .find_map(|prefix| {
                text.strip_prefix(prefix)
                    .map(|rest| (rest.trim_start(), matches!(*prefix, "不要忘了" | "别忘了")))
            })
            .unwrap_or((english, false))
    };
    let mut end = rule_prefix(command).ok_or(ScheduleError::UnsupportedRule)?;
    let tail = &command[end..];
    if tail
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == ':')
    {
        return Err(ScheduleError::ConflictingRule);
    }
    let spaces = tail.len() - tail.trim_start().len();
    let following = &tail[spaces..];
    let zone_len = ["北京时间", "中国标准时间", "UTC"]
        .iter()
        .find(|name| following.starts_with(**name))
        .map(|name| name.len())
        .or_else(|| {
            // The task starts here; a later explicit zone declaration is
            // checked against the complete USER text by explicit_zones.
            if following.starts_with("提醒")
                || following.starts_with("通知")
                || following.starts_with("叫我")
                || strip_ascii_prefix(following, "remind me").is_some()
                || strip_ascii_prefix(following, "notify me").is_some()
            {
                return None;
            }
            let word = following
                .split(|c: char| c.is_whitespace() || matches!(c, ',' | '，' | '。' | '；' | ';'))
                .next()?;
            if word.contains('/') && word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                Some(
                    word.bytes()
                        .take_while(|b| {
                            b.is_ascii_alphanumeric() || matches!(*b, b'/' | b'_' | b'-' | b'+')
                        })
                        .count(),
                )
            } else if let Some(name) = chinese_zone_prefix(word) {
                Some(name.len())
            } else if word.len() >= 2
                && word.len() <= 5
                && word.bytes().all(|b| b.is_ascii_uppercase())
            {
                Some(word.len())
            } else if following[word.len()..].starts_with(" time") {
                Some(word.len() + " time".len())
            } else {
                None
            }
        });
    if let Some(length) = zone_len {
        zone(&following[..length])?;
        end += spaces + length;
    }
    if command[end..]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || matches!(c, ':' | '+' | '/'))
    {
        return Err(ScheduleError::ConflictingRule);
    }
    let rest = command[end..]
        .trim()
        .trim_matches(|c: char| matches!(c, ',' | '，' | '。' | '.'));
    let positive = rest.starts_with("提醒")
        || rest.starts_with("通知")
        || rest.starts_with("叫我")
        || strip_ascii_prefix(rest, "remind me").is_some()
        || strip_ascii_prefix(rest, "notify me").is_some();
    if !positive && !positive_idiom {
        return Err(ScheduleError::ConflictingRule);
    }
    if rest.chars().any(|c| matches!(c, '?' | '？'))
        || rest.contains("不要")
        || rest.contains("取消")
        || rest.contains("别提醒")
        || rest.contains("是什么意思")
        || rest.contains("的含义")
        || rest.contains(" not ")
        || rest.contains(" instead of ")
    {
        return Err(ScheduleError::ConflictingRule);
    }
    // A second supported rule anywhere in the task makes the command ambiguous.
    for (i, _) in rest.char_indices() {
        if rule_prefix(&rest[i..]).is_some() || numeric_relative_prefix(&rest[i..]) {
            return Err(ScheduleError::ConflictingRule);
        }
    }
    Ok(&command[..end])
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
    // Literal-source uniqueness remains necessary, but is not authorization:
    // the complete USER grammar below also determines the entire time rule.
    if original_user.match_indices(exact_span).count() != 1 {
        return Err(ScheduleError::SourceMismatch);
    }
    let authorized_span = command_rule(original_user)?;
    if exact_span != authorized_span {
        return Err(ScheduleError::SourceMismatch);
    }
    let (tz, timezone) = selected_zone(original_user, proposed_zone, host_timezone)?;
    if source_input_ms <= 0 {
        return Err(ScheduleError::TimeOutOfRange);
    }
    let source = DateTime::<Utc>::from_timestamp_millis(source_input_ms)
        .ok_or(ScheduleError::TimeOutOfRange)?;
    let mut rule = authorized_span
        .trim()
        .trim_matches(|c: char| matches!(c, '，' | ',' | '。' | '.'));
    if proposed_zone != "HOST_DEFAULT" {
        rule = rule.trim_end_matches(proposed_zone).trim();
    }
    if rule.is_empty() {
        return Err(ScheduleError::EmptyRule);
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

/// Advance an already authenticated routine after its exact prior occurrence.
/// The stored canonical zone and original USER rule are the only schedule
/// inputs. Missed intervals are skipped; there is never a burst of catch-up
/// occurrences or a clock allowance around `now_ms`.
pub(crate) fn next_due_after(
    original_user: &str,
    exact_rule: &str,
    canonical_timezone: &str,
    prior_due_ms: i64,
    now_ms: i64,
) -> Result<Option<i64>, ScheduleError> {
    if prior_due_ms <= 0 || now_ms <= 0 {
        return Err(ScheduleError::TimeOutOfRange);
    }
    if original_user.match_indices(exact_rule).count() != 1
        || command_rule(original_user)? != exact_rule
    {
        return Err(ScheduleError::SourceMismatch);
    }
    let (tz, canonical) = zone(canonical_timezone)?;
    if canonical != canonical_timezone {
        return Err(ScheduleError::ConflictingTimezone);
    }
    let explicit = explicit_zones(original_user)?;
    if explicit.iter().any(|name| zone(name).map(|(_, value)| value != canonical).unwrap_or(true)) {
        return Err(ScheduleError::ConflictingTimezone);
    }
    let normalized = exact_rule.trim().trim_matches(|c: char| matches!(c, '，' | ',' | '。' | '.'));
    let end = rule_prefix(normalized).ok_or(ScheduleError::UnsupportedRule)?;
    let rule = &normalized[..end];
    let period = interval(rule, "每", "")
        .or_else(|| english_interval(rule, "every "));
    if let Some(period) = period {
        let period = period?;
        let elapsed = now_ms.max(prior_due_ms).checked_sub(prior_due_ms)
            .ok_or(ScheduleError::TimeOutOfRange)?;
        let steps = elapsed.checked_div(period).and_then(|n| n.checked_add(1))
            .ok_or(ScheduleError::TimeOutOfRange)?;
        return prior_due_ms.checked_add(steps.checked_mul(period)
            .ok_or(ScheduleError::TimeOutOfRange)?)
            .map(Some).ok_or(ScheduleError::TimeOutOfRange);
    }
    let lower = rule.to_ascii_lowercase();
    if rule.starts_with("每天") || rule.starts_with("每周")
        || rule.starts_with("每星期") || lower.starts_with("daily at ")
        || lower.starts_with("weekly on ")
    {
        let source = DateTime::<Utc>::from_timestamp_millis(now_ms.max(prior_due_ms))
            .ok_or(ScheduleError::TimeOutOfRange)?;
        let due = calendar(rule, tz, source, false)?;
        if due <= now_ms || due <= prior_due_ms {
            return Err(ScheduleError::DueNotAfterSource);
        }
        return Ok(Some(due));
    }
    // Absolute dates, tomorrow and relative "in/后" rules each authorize one
    // occurrence only. The already reserved occurrence consumes that rule.
    Ok(None)
}

#[cfg(test)]
#[path = "secretary_schedule_tests.rs"]
mod tests;
