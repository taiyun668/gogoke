use super::*;

fn stamp(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp_millis()
}

fn resolve(
    body: &str,
    span: &str,
    proposed: &str,
    source: &str,
    host: &str,
) -> Result<HostSchedule, ScheduleError> {
    resolve_user_schedule(body, span, proposed, stamp(source), host)
}

#[test]
fn relative_time_uses_original_user_input_not_tool_time() {
    let row = resolve(
        "请在10分钟后提醒我",
        "10分钟后",
        "HOST_DEFAULT",
        "2026-10-07T12:00:00Z",
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(row.next_due_ms, stamp("2026-10-07T12:10:00Z"));
    assert_eq!(row.timezone, "America/Los_Angeles");
    let row = resolve(
        "in 2 hours remind me",
        "in 2 hours",
        "HOST_DEFAULT",
        "2026-10-07T12:00:00Z",
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(row.next_due_ms, stamp("2026-10-07T14:00:00Z"));
}

#[test]
fn model_cannot_supply_a_different_or_duplicate_rule() {
    assert_eq!(
        resolve(
            "请在11分钟后提醒我",
            "1分钟后",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::SourceMismatch)
    );
    assert_eq!(
        resolve(
            "请在11分钟后提醒我",
            "11分钟后",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        )
        .unwrap()
        .next_due_ms,
        stamp("2026-10-07T12:11:00Z")
    );
    assert!(resolve(
        "请在10分钟后提醒我读“计划”",
        "10分钟后",
        "HOST_DEFAULT",
        "2026-10-07T12:00:00Z",
        "Asia/Shanghai"
    )
    .is_ok());
    assert!(resolve(
        "请解释‘10分钟后提醒我’的含义",
        "10分钟后",
        "HOST_DEFAULT",
        "2026-10-07T12:00:00Z",
        "Asia/Shanghai"
    )
    .is_err());
    assert_eq!(
        resolve(
            "10分钟后提醒我是什么意思",
            "10分钟后",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::ConflictingRule)
    );
    assert_eq!(
        resolve(
            "明天 09:00",
            "明天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::ConflictingRule)
    );
    assert_eq!(
        resolve(
            "明天 09:00 提醒",
            "每天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::SourceMismatch)
    );
    assert_eq!(
        resolve(
            "每天 09:00 和每天 09:00",
            "每天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::SourceMismatch)
    );
    assert_eq!(
        resolve(
            "每天 09:00 或每周一 10:00",
            "每天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::ConflictingRule)
    );
    assert_eq!(
        resolve(
            "每2小时提醒，明天 09:00 也提醒",
            "每2小时",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::ConflictingRule)
    );
    assert_eq!(
        resolve(
            "不要每天 09:00 提醒",
            "每天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::ConflictingRule)
    );
    assert_eq!(
        resolve(
            "每天 09:00 提醒我 check tests in main",
            "每天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        )
        .unwrap()
        .next_due_ms,
        stamp("2026-10-08T01:00:00Z")
    );
    assert_eq!(
        resolve(
            "每天 09:00 and in 2 hours",
            "每天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::ConflictingRule)
    );
    assert_eq!(
        resolve(
            "不要忘了每天 09:00 提醒",
            "每天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        )
        .unwrap()
        .next_due_ms,
        stamp("2026-10-08T01:00:00Z")
    );
    assert_eq!(
        resolve(
            "不要把旧任务改成这样。请每天 09:00 提醒",
            "每天 09:00",
            "HOST_DEFAULT",
            "2026-10-07T12:00:00Z",
            "Asia/Shanghai"
        ),
        Err(ScheduleError::ConflictingRule)
    );
}

#[test]
fn timezone_must_come_from_user_or_real_host() {
    let source = "2026-10-07T12:00:00Z";
    assert_eq!(
        resolve(
            "明天 09:00 北京时间 提醒我",
            "明天 09:00 北京时间",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::ConflictingTimezone)
    );
    assert_eq!(
        resolve(
            "明天 09:00 日本时间提醒我",
            "明天 09:00",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::InvalidTimezone("日本时间".to_owned()))
    );
    assert_eq!(
        resolve(
            "明天 09:00 JST 提醒我",
            "明天 09:00",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::InvalidTimezone("JST".to_owned()))
    );
    assert_eq!(
        resolve(
            "明天 09:00 提醒我",
            "明天 09:00",
            "HOST_DEFAULT",
            source,
            ""
        ),
        Err(ScheduleError::MissingTimezone)
    );
    assert_eq!(
        resolve(
            "明天 09:00 北京时间 提醒我",
            "明天 09:00 北京时间",
            "Asia/Shanghai",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::SourceMismatch)
    );
    let row = resolve(
        "明天 09:00 北京时间 提醒我",
        "明天 09:00 北京时间",
        "北京时间",
        source,
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(row.timezone, "Asia/Shanghai");
    assert_eq!(row.next_due_ms, stamp("2026-10-08T01:00:00Z"));
    assert_eq!(
        resolve(
            "明天 09:00 CST 提醒我",
            "明天 09:00 CST",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::InvalidTimezone("CST".to_owned()))
    );
    assert_eq!(
        resolve(
            "明天 09:00 Mars/Base 提醒我",
            "明天 09:00 Mars/Base",
            "Mars/Base",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::InvalidTimezone("Mars/Base".to_owned()))
    );
    assert!(resolve(
        "每天 09:00 提醒我检查 docs/PLAN.md",
        "每天 09:00",
        "HOST_DEFAULT",
        source,
        "America/Los_Angeles"
    )
    .is_ok());
    assert_eq!(
        resolve(
            "明天 09:00 Europe/Invalid 提醒我",
            "明天 09:00 Europe/Invalid",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::InvalidTimezone("Europe/Invalid".to_owned()))
    );
}

#[test]
fn daily_weekly_and_one_shot_use_calendar() {
    let source = "2026-10-07T12:00:00Z";
    let daily = resolve(
        "每天 09:00 提醒我",
        "每天 09:00",
        "HOST_DEFAULT",
        source,
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(daily.next_due_ms, stamp("2026-10-07T16:00:00Z"));
    let weekly = resolve(
        "weekly on Monday at 09:00 提醒我",
        "weekly on Monday at 09:00",
        "HOST_DEFAULT",
        source,
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(weekly.next_due_ms, stamp("2026-10-12T16:00:00Z"));
    let weekly_zh = resolve(
        "每周一09:00 提醒我",
        "每周一09:00",
        "HOST_DEFAULT",
        source,
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(weekly_zh.next_due_ms, weekly.next_due_ms);
    let once = resolve(
        "2026-10-08 09:00 提醒我",
        "2026-10-08 09:00",
        "HOST_DEFAULT",
        source,
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(once.next_due_ms, stamp("2026-10-08T16:00:00Z"));
    let once_zh = resolve(
        "2026年10月8日 09:00 提醒我",
        "2026年10月8日 09:00",
        "HOST_DEFAULT",
        source,
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(once_zh.next_due_ms, once.next_due_ms);
    let rfc = resolve(
        "2026-10-08T09:00:00+08:00 提醒我",
        "2026-10-08T09:00:00+08:00",
        "HOST_DEFAULT",
        source,
        "America/Los_Angeles",
    )
    .unwrap();
    assert_eq!(rfc.next_due_ms, stamp("2026-10-08T01:00:00Z"));
    assert_eq!(
        resolve(
            "2026-10-08T09:00:00+08:00 America/Los_Angeles 提醒我",
            "2026-10-08T09:00:00+08:00 America/Los_Angeles",
            "America/Los_Angeles",
            source,
            "Asia/Shanghai"
        ),
        Err(ScheduleError::ConflictingTimezone)
    );
}

#[test]
fn invalid_or_ambiguous_clock_never_rolls_forward() {
    let source = "2026-01-01T00:00:00Z";
    assert_eq!(
        resolve(
            "2026-03-08 02:30 提醒我",
            "2026-03-08 02:30",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::NonexistentLocalTime)
    );
    assert_eq!(
        resolve(
            "2026-11-01 01:30 提醒我",
            "2026-11-01 01:30",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::AmbiguousLocalTime)
    );
    assert_eq!(
        resolve(
            "每天 25:00 提醒我",
            "每天 25:00",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::InvalidTime)
    );
    assert_eq!(
        resolve(
            "2026-02-30 09:00 提醒我",
            "2026-02-30 09:00",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::InvalidDate)
    );
    assert_eq!(
        resolve(
            "每天早上 提醒我",
            "每天早上",
            "HOST_DEFAULT",
            source,
            "America/Los_Angeles"
        ),
        Err(ScheduleError::InvalidTime)
    );
}
