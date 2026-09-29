//! B2 CommitOrchestration vertical slice: project / thread / message on the
//! same-open connection. Not SQL-over-IPC and not the nine-projector port.

use super::atomic::{exec, AtomicError, Statement};
use super::digest::content_hash;
use super::same_open::{SameOpenError, VerifiedDatabaseConnection};
use crate::process::ProcessCustodyError;

#[derive(Debug)]
pub enum OrchestrationError {
    Invalid(&'static str),
    OperationConflict,
    StreamConflict,
    AccessDenied,
    ProjectorRejected(&'static str),
    Fault(FaultPoint),
    CommitUnknown,
    Atomic(AtomicError),
    Process(ProcessCustodyError),
}

impl std::fmt::Display for OrchestrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for OrchestrationError {}

impl From<ProcessCustodyError> for OrchestrationError {
    fn from(error: ProcessCustodyError) -> Self { Self::Process(error) }
}

impl From<AtomicError> for OrchestrationError {
    fn from(error: AtomicError) -> Self {
        match error {
            AtomicError::SameOpen(SameOpenError::SqliteExec { .. }) => Self::CommitUnknown,
            AtomicError::CommitUnknown => Self::CommitUnknown,
            other => Self::Atomic(other),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultPoint {
    AfterIntent,
    AfterEvent(usize),
    AfterProjector(usize),
    AfterCursor,
    AfterReceipt,
    BeforeCommit,
    RevokeBeforeCommit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventProposal {
    ProjectCreated {
        event_id: String,
        project_id: String,
        title: String,
        workspace_root: String,
        occurred_at: String,
    },
    ThreadCreated {
        event_id: String,
        thread_id: String,
        project_id: String,
        title: String,
        model: String,
        occurred_at: String,
    },
    MessageSent {
        event_id: String,
        message_id: String,
        thread_id: String,
        role: String,
        text: String,
        occurred_at: String,
    },
    ActivityAppended {
        event_id: String,
        activity_id: String,
        thread_id: String,
        tone: String,
        kind: String,
        summary: String,
        occurred_at: String,
    },
    SessionSet {
        event_id: String,
        thread_id: String,
        status: String,
        occurred_at: String,
    },
    TurnRequested {
        event_id: String,
        thread_id: String,
        turn_id: String,
        occurred_at: String,
    },
    ApprovalRequested {
        event_id: String,
        thread_id: String,
        request_id: String,
        occurred_at: String,
    },
    ProposedPlanUpserted {
        event_id: String,
        thread_id: String,
        plan_id: String,
        markdown: String,
        occurred_at: String,
    },
    CheckpointCompleted {
        event_id: String,
        thread_id: String,
        occurred_at: String,
    },
}

impl EventProposal {
    fn event_id(&self) -> &str {
        match self {
            Self::ProjectCreated { event_id, .. }
            | Self::ThreadCreated { event_id, .. }
            | Self::MessageSent { event_id, .. }
            | Self::ActivityAppended { event_id, .. }
            | Self::SessionSet { event_id, .. }
            | Self::TurnRequested { event_id, .. }
            | Self::ApprovalRequested { event_id, .. }
            | Self::ProposedPlanUpserted { event_id, .. }
            | Self::CheckpointCompleted { event_id, .. } => event_id,
        }
    }

    fn event_type(&self) -> &'static str {
        match self {
            Self::ProjectCreated { .. } => "project.created",
            Self::ThreadCreated { .. } => "thread.created",
            Self::MessageSent { .. } => "thread.message-sent",
            Self::ActivityAppended { .. } => "thread.activity-appended",
            Self::SessionSet { .. } => "thread.session-set",
            Self::TurnRequested { .. } => "thread.turn-requested",
            Self::ApprovalRequested { .. } => "thread.approval-response-requested",
            Self::ProposedPlanUpserted { .. } => "thread.proposed-plan-upserted",
            Self::CheckpointCompleted { .. } => "thread.turn-diff-completed",
        }
    }

    fn aggregate_kind(&self) -> &'static str {
        match self {
            Self::ProjectCreated { .. } => "project",
            Self::ThreadCreated { .. }
            | Self::MessageSent { .. }
            | Self::ActivityAppended { .. }
            | Self::SessionSet { .. }
            | Self::TurnRequested { .. }
            | Self::ApprovalRequested { .. }
            | Self::ProposedPlanUpserted { .. }
            | Self::CheckpointCompleted { .. } => "thread",
        }
    }

    fn stream_id(&self) -> &str {
        match self {
            Self::ProjectCreated { project_id, .. } => project_id,
            Self::ThreadCreated { thread_id, .. }
            | Self::MessageSent { thread_id, .. }
            | Self::ActivityAppended { thread_id, .. }
            | Self::SessionSet { thread_id, .. }
            | Self::TurnRequested { thread_id, .. }
            | Self::ApprovalRequested { thread_id, .. }
            | Self::ProposedPlanUpserted { thread_id, .. }
            | Self::CheckpointCompleted { thread_id, .. } => thread_id,
        }
    }

    fn occurred_at(&self) -> &str {
        match self {
            Self::ProjectCreated { occurred_at, .. }
            | Self::ThreadCreated { occurred_at, .. }
            | Self::MessageSent { occurred_at, .. }
            | Self::ActivityAppended { occurred_at, .. }
            | Self::SessionSet { occurred_at, .. }
            | Self::TurnRequested { occurred_at, .. }
            | Self::ApprovalRequested { occurred_at, .. }
            | Self::ProposedPlanUpserted { occurred_at, .. }
            | Self::CheckpointCompleted { occurred_at, .. } => occurred_at,
        }
    }

    fn payload_json(&self) -> String {
        match self {
            Self::ProjectCreated {
                project_id,
                title,
                workspace_root,
                occurred_at,
                ..
            } => format!(
                "{{\"createdAt\":\"{occurred_at}\",\"projectId\":\"{}\",\"scripts\":[],\"title\":\"{}\",\"updatedAt\":\"{occurred_at}\",\"workspaceRoot\":\"{}\"}}",
                escape(project_id),
                escape(title),
                escape(workspace_root),
            ),
            Self::ThreadCreated {
                thread_id,
                project_id,
                title,
                model,
                occurred_at,
                ..
            } => format!(
                "{{\"createdAt\":\"{occurred_at}\",\"model\":\"{}\",\"projectId\":\"{}\",\"threadId\":\"{}\",\"title\":\"{}\",\"updatedAt\":\"{occurred_at}\"}}",
                escape(model),
                escape(project_id),
                escape(thread_id),
                escape(title),
            ),
            Self::MessageSent {
                message_id,
                thread_id,
                role,
                text,
                occurred_at,
                ..
            } => format!(
                "{{\"createdAt\":\"{occurred_at}\",\"messageId\":\"{}\",\"role\":\"{}\",\"text\":\"{}\",\"threadId\":\"{}\",\"updatedAt\":\"{occurred_at}\"}}",
                escape(message_id),
                escape(role),
                escape(text),
                escape(thread_id),
            ),
            Self::ActivityAppended {
                activity_id,
                thread_id,
                tone,
                kind,
                summary,
                occurred_at,
                ..
            } => format!(
                "{{\"activityId\":\"{}\",\"kind\":\"{}\",\"occurredAt\":\"{occurred_at}\",\"summary\":\"{}\",\"threadId\":\"{}\",\"tone\":\"{}\"}}",
                escape(activity_id),
                escape(kind),
                escape(summary),
                escape(thread_id),
                escape(tone),
            ),
            Self::SessionSet {
                thread_id,
                status,
                occurred_at,
                ..
            } => format!(
                "{{\"occurredAt\":\"{occurred_at}\",\"status\":\"{}\",\"threadId\":\"{}\"}}",
                escape(status),
                escape(thread_id),
            ),
            Self::TurnRequested {
                thread_id,
                turn_id,
                occurred_at,
                ..
            } => format!(
                "{{\"occurredAt\":\"{occurred_at}\",\"threadId\":\"{}\",\"turnId\":\"{}\"}}",
                escape(thread_id),
                escape(turn_id),
            ),
            Self::ApprovalRequested {
                thread_id,
                request_id,
                occurred_at,
                ..
            } => format!(
                "{{\"occurredAt\":\"{occurred_at}\",\"requestId\":\"{}\",\"threadId\":\"{}\"}}",
                escape(request_id),
                escape(thread_id),
            ),
            Self::ProposedPlanUpserted {
                thread_id,
                plan_id,
                markdown,
                occurred_at,
                ..
            } => format!(
                "{{\"markdown\":\"{}\",\"occurredAt\":\"{occurred_at}\",\"planId\":\"{}\",\"threadId\":\"{}\"}}",
                escape(markdown),
                escape(plan_id),
                escape(thread_id),
            ),
            Self::CheckpointCompleted {
                thread_id,
                occurred_at,
                ..
            } => format!(
                "{{\"occurredAt\":\"{occurred_at}\",\"threadId\":\"{}\"}}",
                escape(thread_id),
            ),
        }
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn require_id(value: &str, path: &'static str) -> Result<(), OrchestrationError> {
    if value.is_empty() || value.trim() != value || value.contains('\0') {
        return Err(OrchestrationError::Invalid(path));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrchestrationCommand {
    pub command_id: String,
    pub command_type: String,
    pub events: Vec<EventProposal>,
    pub expected_stream_version: Option<i64>,
    pub access_admitted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrchestrationReceipt {
    pub disposition: &'static str,
    pub command_id: String,
    pub result_sequence: i64,
    pub fingerprint: String,
}

fn fingerprint(command: &OrchestrationCommand) -> String {
    let mut material = format!(
        "{{\"commandId\":\"{}\",\"commandType\":\"{}\",\"events\":[",
        escape(&command.command_id),
        escape(&command.command_type)
    );
    for (index, event) in command.events.iter().enumerate() {
        if index > 0 {
            material.push(',');
        }
        material.push_str(&format!(
            "{{\"eventId\":\"{}\",\"payload\":{}}}",
            escape(event.event_id()),
            event.payload_json()
        ));
    }
    material.push_str("]}");
    content_hash(material.as_bytes())
}

pub fn apply_orchestration_slice_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), OrchestrationError> {
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS orchestration_intents (
            command_id TEXT PRIMARY KEY,
            command_type TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            canonical_json TEXT NOT NULL
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS orchestration_events (
            sequence INTEGER PRIMARY KEY,
            event_id TEXT NOT NULL UNIQUE,
            aggregate_kind TEXT NOT NULL,
            stream_id TEXT NOT NULL,
            stream_version INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            command_id TEXT,
            causation_event_id TEXT,
            correlation_id TEXT,
            actor_kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            metadata_json TEXT NOT NULL,
            UNIQUE (aggregate_kind, stream_id, stream_version)
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS orchestration_command_receipts (
            command_id TEXT PRIMARY KEY,
            aggregate_kind TEXT NOT NULL,
            aggregate_id TEXT NOT NULL,
            accepted_at TEXT NOT NULL,
            result_sequence INTEGER NOT NULL,
            status TEXT NOT NULL,
            error TEXT
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_projects (
            project_id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            workspace_root TEXT NOT NULL,
            default_model TEXT,
            scripts_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted_at TEXT
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_threads (
            thread_id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            title TEXT NOT NULL,
            model TEXT NOT NULL,
            branch TEXT,
            worktree_path TEXT,
            latest_turn_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted_at TEXT
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_thread_messages (
            message_id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            turn_id TEXT,
            role TEXT NOT NULL,
            text TEXT NOT NULL,
            is_streaming INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_thread_activities (
            activity_id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            turn_id TEXT,
            tone TEXT NOT NULL,
            kind TEXT NOT NULL,
            summary TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_thread_sessions (
            thread_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            provider_name TEXT,
            provider_session_id TEXT,
            provider_thread_id TEXT,
            active_turn_id TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_turns (
            row_id INTEGER PRIMARY KEY AUTOINCREMENT,
            thread_id TEXT NOT NULL,
            turn_id TEXT,
            pending_message_id TEXT,
            assistant_message_id TEXT,
            state TEXT NOT NULL,
            requested_at TEXT NOT NULL,
            started_at TEXT,
            completed_at TEXT,
            checkpoint_turn_count INTEGER,
            checkpoint_ref TEXT,
            checkpoint_status TEXT,
            checkpoint_files_json TEXT NOT NULL,
            UNIQUE (thread_id, turn_id)
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_pending_approvals (
            request_id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            turn_id TEXT,
            status TEXT NOT NULL,
            decision TEXT,
            created_at TEXT NOT NULL,
            resolved_at TEXT
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_thread_proposed_plans (
            plan_id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            turn_id TEXT,
            plan_markdown TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        ) STRICT",
    )?;
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS projection_state (
            projector TEXT PRIMARY KEY,
            last_applied_sequence INTEGER NOT NULL,
            updated_at TEXT NOT NULL
        ) STRICT",
    )?;
    exec(connection, "PRAGMA foreign_keys = ON")?;
    exec(connection, "PRAGMA journal_mode = WAL")?;
    exec(connection, "PRAGMA synchronous = FULL")?;
    Ok(())
}

fn query_text(
    connection: &mut VerifiedDatabaseConnection<'_>,
    sql: &str,
    binds: &[&str],
) -> Result<Option<String>, OrchestrationError> {
    let statement = Statement::prepare(connection.as_ptr(), sql)?;
    for (index, value) in binds.iter().enumerate() {
        statement.bind_text((index + 1) as i32, value)?;
    }
    if statement.step_row()? {
        Ok(Some(statement.column_text(0)?))
    } else {
        Ok(None)
    }
}

fn query_i64(
    connection: &mut VerifiedDatabaseConnection<'_>,
    sql: &str,
) -> Result<i64, OrchestrationError> {
    let statement = Statement::prepare(connection.as_ptr(), sql)?;
    if !statement.step_row()? {
        return Ok(0);
    }
    statement
        .column_text(0)?
        .parse::<i64>()
        .map_err(|_| OrchestrationError::Invalid("integer column"))
}

fn hit(fault: Option<FaultPoint>, point: FaultPoint) -> Result<(), OrchestrationError> {
    if fault == Some(point) {
        Err(OrchestrationError::Fault(point))
    } else {
        Ok(())
    }
}

fn rollback(connection: &mut VerifiedDatabaseConnection<'_>) {
    let _ = connection.execute("ROLLBACK");
}

fn insert_event(
    connection: &mut VerifiedDatabaseConnection<'_>,
    command_id: &str,
    event: &EventProposal,
    sequence: i64,
    stream_version: i64,
) -> Result<(), OrchestrationError> {
    let payload = event.payload_json();
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO orchestration_events (
            sequence, event_id, aggregate_kind, stream_id, stream_version, event_type,
            occurred_at, command_id, causation_event_id, correlation_id, actor_kind,
            payload_json, metadata_json
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, 'client', ?, '{}')",
    )?;
    statement.bind_i64(1, sequence)?;
    statement.bind_text(2, event.event_id())?;
    statement.bind_text(3, event.aggregate_kind())?;
    statement.bind_text(4, event.stream_id())?;
    statement.bind_i64(5, stream_version)?;
    statement.bind_text(6, event.event_type())?;
    statement.bind_text(7, event.occurred_at())?;
    statement.bind_text(8, command_id)?;
    statement.bind_text(9, command_id)?;
    statement.bind_text(10, &payload)?;
    statement.step_done()?;
    Ok(())
}

fn project_event(
    connection: &mut VerifiedDatabaseConnection<'_>,
    event: &EventProposal,
) -> Result<(), OrchestrationError> {
    match event {
        EventProposal::ProjectCreated {
            project_id,
            title,
            workspace_root,
            occurred_at,
            ..
        } => {
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO projection_projects (
                    project_id, title, workspace_root, default_model, scripts_json,
                    created_at, updated_at, deleted_at
                 ) VALUES (?, ?, ?, NULL, '[]', ?, ?, NULL)",
            )?;
            statement.bind_text(1, project_id)?;
            statement.bind_text(2, title)?;
            statement.bind_text(3, workspace_root)?;
            statement.bind_text(4, occurred_at)?;
            statement.bind_text(5, occurred_at)?;
            statement.step_done().map_err(|_| {
                OrchestrationError::ProjectorRejected("project.created")
            })?;
        }
        EventProposal::ThreadCreated {
            thread_id,
            project_id,
            title,
            model,
            occurred_at,
            ..
        } => {
            let exists = query_text(
                connection,
                "SELECT project_id FROM projection_projects WHERE project_id = ? AND deleted_at IS NULL",
                &[project_id],
            )?;
            if exists.is_none() {
                return Err(OrchestrationError::ProjectorRejected("thread.created"));
            }
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO projection_threads (
                    thread_id, project_id, title, model, branch, worktree_path,
                    latest_turn_id, created_at, updated_at, deleted_at
                 ) VALUES (?, ?, ?, ?, NULL, NULL, NULL, ?, ?, NULL)",
            )?;
            statement.bind_text(1, thread_id)?;
            statement.bind_text(2, project_id)?;
            statement.bind_text(3, title)?;
            statement.bind_text(4, model)?;
            statement.bind_text(5, occurred_at)?;
            statement.bind_text(6, occurred_at)?;
            statement.step_done().map_err(|_| {
                OrchestrationError::ProjectorRejected("thread.created")
            })?;
        }
        EventProposal::MessageSent {
            message_id,
            thread_id,
            role,
            text,
            occurred_at,
            ..
        } => {
            let exists = query_text(
                connection,
                "SELECT thread_id FROM projection_threads WHERE thread_id = ? AND deleted_at IS NULL",
                &[thread_id],
            )?;
            if exists.is_none() {
                return Err(OrchestrationError::ProjectorRejected("thread.message-sent"));
            }
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO projection_thread_messages (
                    message_id, thread_id, turn_id, role, text, is_streaming,
                    created_at, updated_at
                 ) VALUES (?, ?, NULL, ?, ?, 0, ?, ?)",
            )?;
            statement.bind_text(1, message_id)?;
            statement.bind_text(2, thread_id)?;
            statement.bind_text(3, role)?;
            statement.bind_text(4, text)?;
            statement.bind_text(5, occurred_at)?;
            statement.bind_text(6, occurred_at)?;
            statement.step_done().map_err(|_| {
                OrchestrationError::ProjectorRejected("thread.message-sent")
            })?;
            let statement = Statement::prepare(
                connection.as_ptr(),
                "UPDATE projection_threads SET updated_at = ? WHERE thread_id = ?",
            )?;
            statement.bind_text(1, occurred_at)?;
            statement.bind_text(2, thread_id)?;
            statement.step_done()?;
        }
        EventProposal::ActivityAppended {
            activity_id,
            thread_id,
            tone,
            kind,
            summary,
            occurred_at,
            ..
        } => {
            require_thread(connection, thread_id)?;
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO projection_thread_activities (
                    activity_id, thread_id, turn_id, tone, kind, summary, payload_json, created_at
                 ) VALUES (?, ?, NULL, ?, ?, ?, '{}', ?)",
            )?;
            statement.bind_text(1, activity_id)?;
            statement.bind_text(2, thread_id)?;
            statement.bind_text(3, tone)?;
            statement.bind_text(4, kind)?;
            statement.bind_text(5, summary)?;
            statement.bind_text(6, occurred_at)?;
            statement
                .step_done()
                .map_err(|_| OrchestrationError::ProjectorRejected("thread.activity-appended"))?;
        }
        EventProposal::SessionSet {
            thread_id,
            status,
            occurred_at,
            ..
        } => {
            require_thread(connection, thread_id)?;
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO projection_thread_sessions (
                    thread_id, status, provider_name, provider_session_id, provider_thread_id,
                    active_turn_id, last_error, updated_at
                 ) VALUES (?, ?, NULL, NULL, NULL, NULL, NULL, ?)
                 ON CONFLICT (thread_id) DO UPDATE SET status = excluded.status, updated_at = excluded.updated_at",
            )?;
            statement.bind_text(1, thread_id)?;
            statement.bind_text(2, status)?;
            statement.bind_text(3, occurred_at)?;
            statement
                .step_done()
                .map_err(|_| OrchestrationError::ProjectorRejected("thread.session-set"))?;
        }
        EventProposal::TurnRequested {
            thread_id,
            turn_id,
            occurred_at,
            ..
        } => {
            require_thread(connection, thread_id)?;
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO projection_turns (
                    thread_id, turn_id, pending_message_id, assistant_message_id, state,
                    requested_at, started_at, completed_at, checkpoint_turn_count, checkpoint_ref,
                    checkpoint_status, checkpoint_files_json
                 ) VALUES (?, ?, NULL, NULL, 'requested', ?, NULL, NULL, NULL, NULL, NULL, '[]')",
            )?;
            statement.bind_text(1, thread_id)?;
            statement.bind_text(2, turn_id)?;
            statement.bind_text(3, occurred_at)?;
            statement
                .step_done()
                .map_err(|_| OrchestrationError::ProjectorRejected("thread.turn-requested"))?;
        }
        EventProposal::ApprovalRequested {
            thread_id,
            request_id,
            occurred_at,
            ..
        } => {
            require_thread(connection, thread_id)?;
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO projection_pending_approvals (
                    request_id, thread_id, turn_id, status, decision, created_at, resolved_at
                 ) VALUES (?, ?, NULL, 'pending', NULL, ?, NULL)",
            )?;
            statement.bind_text(1, request_id)?;
            statement.bind_text(2, thread_id)?;
            statement.bind_text(3, occurred_at)?;
            statement.step_done().map_err(|_| {
                OrchestrationError::ProjectorRejected("thread.approval-response-requested")
            })?;
        }
        EventProposal::ProposedPlanUpserted {
            thread_id,
            plan_id,
            markdown,
            occurred_at,
            ..
        } => {
            require_thread(connection, thread_id)?;
            let statement = Statement::prepare(
                connection.as_ptr(),
                "INSERT INTO projection_thread_proposed_plans (
                    plan_id, thread_id, turn_id, plan_markdown, created_at, updated_at
                 ) VALUES (?, ?, NULL, ?, ?, ?)",
            )?;
            statement.bind_text(1, plan_id)?;
            statement.bind_text(2, thread_id)?;
            statement.bind_text(3, markdown)?;
            statement.bind_text(4, occurred_at)?;
            statement.bind_text(5, occurred_at)?;
            statement.step_done().map_err(|_| {
                OrchestrationError::ProjectorRejected("thread.proposed-plan-upserted")
            })?;
        }
        EventProposal::CheckpointCompleted { .. } => {}
    }
    Ok(())
}

fn require_thread(
    connection: &mut VerifiedDatabaseConnection<'_>,
    thread_id: &str,
) -> Result<(), OrchestrationError> {
    let exists = query_text(
        connection,
        "SELECT thread_id FROM projection_threads WHERE thread_id = ? AND deleted_at IS NULL",
        &[thread_id],
    )?;
    if exists.is_none() {
        Err(OrchestrationError::ProjectorRejected("missing thread"))
    } else {
        Ok(())
    }
}

fn advance_cursor(
    connection: &mut VerifiedDatabaseConnection<'_>,
    sequence: i64,
    occurred_at: &str,
) -> Result<(), OrchestrationError> {
    for projector in [
        "projection.projects",
        "projection.threads",
        "projection.thread-messages",
        "projection.thread-proposed-plans",
        "projection.thread-activities",
        "projection.thread-sessions",
        "projection.thread-turns",
        "projection.checkpoints",
        "projection.pending-approvals",
    ] {
        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO projection_state (projector, last_applied_sequence, updated_at)
             VALUES (?, ?, ?)
             ON CONFLICT (projector) DO UPDATE SET
                last_applied_sequence = excluded.last_applied_sequence,
                updated_at = excluded.updated_at",
        )?;
        statement.bind_text(1, projector)?;
        statement.bind_i64(2, sequence)?;
        statement.bind_text(3, occurred_at)?;
        statement.step_done()?;
    }
    Ok(())
}

pub fn commit_orchestration(
    connection: &mut VerifiedDatabaseConnection<'_>,
    command: OrchestrationCommand,
    fault: Option<FaultPoint>,
) -> Result<OrchestrationReceipt, OrchestrationError> {
    require_id(&command.command_id, "commandId")?;
    require_id(&command.command_type, "commandType")?;
    if command.events.is_empty() {
        return Err(OrchestrationError::Invalid("events"));
    }
    for event in &command.events {
        require_id(event.event_id(), "eventId")?;
        require_id(event.stream_id(), "streamId")?;
        require_id(event.occurred_at(), "occurredAt")?;
    }
    let fingerprint = fingerprint(&command);
    exec(connection, "BEGIN IMMEDIATE")?;
    let outcome = (|| {
        if !command.access_admitted {
            return Err(OrchestrationError::AccessDenied);
        }
        if let Some(existing) = query_text(
            connection,
            "SELECT fingerprint FROM orchestration_intents WHERE command_id = ?",
            &[&command.command_id],
        )? {
            if existing != fingerprint {
                return Err(OrchestrationError::OperationConflict);
            }
            let sequence = query_text(
                connection,
                "SELECT result_sequence FROM orchestration_command_receipts WHERE command_id = ?",
                &[&command.command_id],
            )?
            .ok_or(OrchestrationError::Invalid("reconcile missing receipt"))?
            .parse::<i64>()
            .map_err(|_| OrchestrationError::Invalid("result_sequence"))?;
            return Ok(OrchestrationReceipt {
                disposition: "RECONCILED",
                command_id: command.command_id.clone(),
                result_sequence: sequence,
                fingerprint,
            });
        }

        let intent_json = format!(
            "{{\"commandId\":\"{}\",\"commandType\":\"{}\"}}",
            escape(&command.command_id),
            escape(&command.command_type)
        );
        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO orchestration_intents (command_id, command_type, fingerprint, canonical_json)
             VALUES (?, ?, ?, ?)",
        )?;
        statement.bind_text(1, &command.command_id)?;
        statement.bind_text(2, &command.command_type)?;
        statement.bind_text(3, &fingerprint)?;
        statement.bind_text(4, &intent_json)?;
        statement.step_done()?;
        hit(fault, FaultPoint::AfterIntent)?;

        let mut last_sequence = 0i64;
        let mut last_kind = "";
        let mut last_stream = String::new();
        let mut last_occurred = String::new();
        for (index, event) in command.events.iter().enumerate() {
            let next_sequence = query_i64(connection, "SELECT COALESCE(MAX(sequence), 0) FROM orchestration_events")? + 1;
            let next_version = query_text(
                connection,
                "SELECT COALESCE(MAX(stream_version), -1) FROM orchestration_events WHERE aggregate_kind = ? AND stream_id = ?",
                &[event.aggregate_kind(), event.stream_id()],
            )?
            .unwrap_or_else(|| "-1".into())
            .parse::<i64>()
            .map_err(|_| OrchestrationError::Invalid("stream_version"))?
                + 1;
            if index == 0 {
                match command.expected_stream_version {
                    None if next_version != 0 => return Err(OrchestrationError::StreamConflict),
                    Some(expected) if next_version != expected => {
                        return Err(OrchestrationError::StreamConflict)
                    }
                    _ => {}
                }
            }
            insert_event(connection, &command.command_id, event, next_sequence, next_version)?;
            hit(fault, FaultPoint::AfterEvent(index))?;
            project_event(connection, event)?;
            hit(fault, FaultPoint::AfterProjector(index))?;
            last_sequence = next_sequence;
            last_kind = event.aggregate_kind();
            last_stream = event.stream_id().to_owned();
            last_occurred = event.occurred_at().to_owned();
        }
        advance_cursor(connection, last_sequence, &last_occurred)?;
        hit(fault, FaultPoint::AfterCursor)?;

        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO orchestration_command_receipts (
                command_id, aggregate_kind, aggregate_id, accepted_at, result_sequence, status, error
             ) VALUES (?, ?, ?, ?, ?, 'accepted', NULL)",
        )?;
        statement.bind_text(1, &command.command_id)?;
        statement.bind_text(2, last_kind)?;
        statement.bind_text(3, &last_stream)?;
        statement.bind_text(4, &last_occurred)?;
        statement.bind_i64(5, last_sequence)?;
        statement.step_done()?;
        hit(fault, FaultPoint::AfterReceipt)?;
        if fault == Some(FaultPoint::RevokeBeforeCommit) {
            return Err(OrchestrationError::AccessDenied);
        }
        hit(fault, FaultPoint::BeforeCommit)?;
        Ok(OrchestrationReceipt {
            disposition: "COMMITTED",
            command_id: command.command_id.clone(),
            result_sequence: last_sequence,
            fingerprint,
        })
    })();
    match outcome {
        Ok(receipt) => {
            if let Err(error) = exec(connection, "COMMIT") {
                rollback(connection);
                return Err(error.into());
            }
            Ok(receipt)
        }
        Err(error) => {
            rollback(connection);
            Err(error)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredEvent {
    pub sequence: i64,
    pub event_id: String,
    pub event_type: String,
    pub aggregate_kind: String,
    pub stream_id: String,
    pub stream_version: i64,
    pub occurred_at: String,
    pub payload_json: String,
}

pub fn read_events(
    connection: &mut VerifiedDatabaseConnection<'_>,
    aggregate_kind: &str,
    stream_id: &str,
    from_sequence_exclusive: i64,
    limit: i64,
) -> Result<Vec<StoredEvent>, OrchestrationError> {
    require_id(aggregate_kind, "aggregateKind")?;
    require_id(stream_id, "streamId")?;
    if limit <= 0 || limit > 1000 {
        return Err(OrchestrationError::Invalid("limit"));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT sequence, event_id, event_type, aggregate_kind, stream_id, stream_version,
                occurred_at, payload_json
         FROM orchestration_events
         WHERE aggregate_kind = ? AND stream_id = ? AND sequence > ?
         ORDER BY sequence ASC
         LIMIT ?",
    )?;
    statement.bind_text(1, aggregate_kind)?;
    statement.bind_text(2, stream_id)?;
    statement.bind_i64(3, from_sequence_exclusive)?;
    statement.bind_i64(4, limit)?;
    let mut events = Vec::new();
    while statement.step_row()? {
        events.push(StoredEvent {
            sequence: statement
                .column_text(0)?
                .parse::<i64>()
                .map_err(|_| OrchestrationError::Invalid("sequence"))?,
            event_id: statement.column_text(1)?,
            event_type: statement.column_text(2)?,
            aggregate_kind: statement.column_text(3)?,
            stream_id: statement.column_text(4)?,
            stream_version: statement
                .column_text(5)?
                .parse::<i64>()
                .map_err(|_| OrchestrationError::Invalid("stream_version"))?,
            occurred_at: statement.column_text(6)?,
            payload_json: statement.column_text(7)?,
        });
    }
    Ok(events)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSnapshot {
    pub project_id: String,
    pub title: String,
    pub workspace_root: String,
    pub updated_at: String,
}

pub fn read_snapshot_projects(
    connection: &mut VerifiedDatabaseConnection<'_>,
    limit: i64,
) -> Result<Vec<ProjectSnapshot>, OrchestrationError> {
    if limit <= 0 || limit > 1000 {
        return Err(OrchestrationError::Invalid("limit"));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT project_id, title, workspace_root, updated_at
         FROM projection_projects
         WHERE deleted_at IS NULL
         ORDER BY updated_at ASC, project_id ASC
         LIMIT ?",
    )?;
    statement.bind_i64(1, limit)?;
    let mut rows = Vec::new();
    while statement.step_row()? {
        rows.push(ProjectSnapshot {
            project_id: statement.column_text(0)?,
            title: statement.column_text(1)?,
            workspace_root: statement.column_text(2)?,
            updated_at: statement.column_text(3)?,
        });
    }
    Ok(rows)
}

pub fn record_rejected_command(
    connection: &mut VerifiedDatabaseConnection<'_>,
    command_id: &str,
    command_type: &str,
    aggregate_kind: &str,
    aggregate_id: &str,
    occurred_at: &str,
    error: &str,
    access_admitted: bool,
) -> Result<OrchestrationReceipt, OrchestrationError> {
    require_id(command_id, "commandId")?;
    require_id(command_type, "commandType")?;
    require_id(aggregate_kind, "aggregateKind")?;
    require_id(aggregate_id, "aggregateId")?;
    require_id(occurred_at, "occurredAt")?;
    require_id(error, "error")?;
    let fingerprint = content_hash(
        format!(
            "{{\"commandId\":\"{}\",\"commandType\":\"{}\",\"error\":\"{}\",\"status\":\"rejected\"}}",
            escape(command_id),
            escape(command_type),
            escape(error)
        )
        .as_bytes(),
    );
    exec(connection, "BEGIN IMMEDIATE")?;
    let outcome = (|| {
        if !access_admitted {
            return Err(OrchestrationError::AccessDenied);
        }
        if let Some(existing) = query_text(
            connection,
            "SELECT fingerprint FROM orchestration_intents WHERE command_id = ?",
            &[command_id],
        )? {
            if existing != fingerprint {
                return Err(OrchestrationError::OperationConflict);
            }
            return Ok(OrchestrationReceipt {
                disposition: "RECONCILED",
                command_id: command_id.to_owned(),
                result_sequence: 0,
                fingerprint,
            });
        }
        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO orchestration_intents (command_id, command_type, fingerprint, canonical_json)
             VALUES (?, ?, ?, ?)",
        )?;
        statement.bind_text(1, command_id)?;
        statement.bind_text(2, command_type)?;
        statement.bind_text(3, &fingerprint)?;
        statement.bind_text(4, error)?;
        statement.step_done()?;
        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO orchestration_command_receipts (
                command_id, aggregate_kind, aggregate_id, accepted_at, result_sequence, status, error
             ) VALUES (?, ?, ?, ?, 0, 'rejected', ?)",
        )?;
        statement.bind_text(1, command_id)?;
        statement.bind_text(2, aggregate_kind)?;
        statement.bind_text(3, aggregate_id)?;
        statement.bind_text(4, occurred_at)?;
        statement.bind_text(5, error)?;
        statement.step_done()?;
        Ok(OrchestrationReceipt {
            disposition: "COMMITTED",
            command_id: command_id.to_owned(),
            result_sequence: 0,
            fingerprint,
        })
    })();
    match outcome {
        Ok(receipt) => {
            if let Err(error) = exec(connection, "COMMIT") {
                rollback(connection);
                return Err(error.into());
            }
            Ok(receipt)
        }
        Err(error) => {
            rollback(connection);
            Err(error)
        }
    }
}

pub fn count_rows(
    connection: &mut VerifiedDatabaseConnection<'_>,
    table: &str,
) -> Result<i64, OrchestrationError> {
    if !matches!(
        table,
        "orchestration_intents"
            | "orchestration_events"
            | "orchestration_command_receipts"
            | "projection_projects"
            | "projection_threads"
            | "projection_thread_messages"
            | "projection_thread_activities"
            | "projection_thread_sessions"
            | "projection_turns"
            | "projection_pending_approvals"
            | "projection_thread_proposed_plans"
            | "projection_state"
    ) {
        return Err(OrchestrationError::Invalid("table"));
    }
    query_i64(connection, &format!("SELECT COUNT(*) FROM {table}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("gogoke-orch-{label}-{nonce}"));
        std::fs::create_dir(&root).expect("scratch root");
        root
    }

    fn at() -> &'static str {
        "2026-09-20T00:00:00Z"
    }

    fn create_project() -> OrchestrationCommand {
        OrchestrationCommand {
            command_id: "cmd-project".into(),
            command_type: "project.create".into(),
            access_admitted: true,
            expected_stream_version: None,
            events: vec![EventProposal::ProjectCreated {
                event_id: "ev-project".into(),
                project_id: "proj-1".into(),
                title: "one".into(),
                workspace_root: "C:/tmp/one".into(),
                occurred_at: at().into(),
            }],
        }
    }

    fn create_thread() -> OrchestrationCommand {
        OrchestrationCommand {
            command_id: "cmd-thread".into(),
            command_type: "thread.create".into(),
            access_admitted: true,
            expected_stream_version: None,
            events: vec![EventProposal::ThreadCreated {
                event_id: "ev-thread".into(),
                thread_id: "thr-1".into(),
                project_id: "proj-1".into(),
                title: "chat".into(),
                model: "local".into(),
                occurred_at: at().into(),
            }],
        }
    }

    fn send_message() -> OrchestrationCommand {
        OrchestrationCommand {
            command_id: "cmd-message".into(),
            command_type: "thread.message.user.append".into(),
            access_admitted: true,
            expected_stream_version: Some(1),
            events: vec![EventProposal::MessageSent {
                event_id: "ev-message".into(),
                message_id: "msg-1".into(),
                thread_id: "thr-1".into(),
                role: "user".into(),
                text: "hello".into(),
                occurred_at: at().into(),
            }],
        }
    }

    fn cleanup(root: RootLock, root_path: std::path::PathBuf, database_path: std::path::PathBuf) {
        drop(root);
        std::fs::remove_file(&database_path).ok();
        let _ = std::fs::remove_file(format!("{}-wal", database_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database_path.display()));
        std::fs::remove_dir(&root_path).ok();
    }

    #[test]
    fn project_thread_message_commit_together_and_reconcile() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("happy");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_orchestration_slice_schema(&mut connection).expect("schema");

        let project = commit_orchestration(&mut connection, create_project(), None).expect("project");
        assert_eq!(project.disposition, "COMMITTED");
        let snapshot = read_snapshot_projects(&mut connection, 10).expect("snapshot");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].project_id, "proj-1");
        let thread = commit_orchestration(&mut connection, create_thread(), None).expect("thread");
        assert_eq!(thread.disposition, "COMMITTED");
        let message = commit_orchestration(&mut connection, send_message(), None).expect("message");
        assert_eq!(message.disposition, "COMMITTED");
        assert_eq!(count_rows(&mut connection, "orchestration_events").unwrap(), 3);
        assert_eq!(count_rows(&mut connection, "projection_projects").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "projection_threads").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "projection_thread_messages").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "orchestration_command_receipts").unwrap(), 3);
        let thread_events = read_events(&mut connection, "thread", "thr-1", 0, 10).expect("read");
        assert_eq!(thread_events.len(), 2);
        assert_eq!(thread_events[0].event_type, "thread.created");
        assert_eq!(thread_events[1].event_type, "thread.message-sent");
        assert!(read_events(&mut connection, "thread", "thr-1", 0, 0).is_err());

        let again = commit_orchestration(&mut connection, create_project(), None).expect("reconcile");
        assert_eq!(again.disposition, "RECONCILED");
        assert_eq!(count_rows(&mut connection, "orchestration_events").unwrap(), 3);

        let mut changed = create_project();
        changed.events[0] = EventProposal::ProjectCreated {
            event_id: "ev-project".into(),
            project_id: "proj-1".into(),
            title: "other".into(),
            workspace_root: "C:/tmp/one".into(),
            occurred_at: at().into(),
        };
        let error = commit_orchestration(&mut connection, changed, None).expect_err("conflict");
        assert!(matches!(error, OrchestrationError::OperationConflict), "{error:?}");
        assert_eq!(count_rows(&mut connection, "projection_projects").unwrap(), 1);

        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn faults_roll_back_the_complete_write_set() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("fault");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_orchestration_slice_schema(&mut connection).expect("schema");

        for fault in [
            FaultPoint::AfterIntent,
            FaultPoint::AfterEvent(0),
            FaultPoint::AfterProjector(0),
            FaultPoint::AfterCursor,
            FaultPoint::AfterReceipt,
            FaultPoint::BeforeCommit,
        ] {
            let error =
                commit_orchestration(&mut connection, create_project(), Some(fault)).expect_err("fault");
            assert!(matches!(error, OrchestrationError::Fault(point) if point == fault), "{error:?}");
            assert_eq!(count_rows(&mut connection, "orchestration_intents").unwrap(), 0);
            assert_eq!(count_rows(&mut connection, "orchestration_events").unwrap(), 0);
            assert_eq!(count_rows(&mut connection, "projection_projects").unwrap(), 0);
            assert_eq!(count_rows(&mut connection, "orchestration_command_receipts").unwrap(), 0);
        }

        commit_orchestration(&mut connection, create_project(), None).expect("recover");
        assert_eq!(count_rows(&mut connection, "projection_projects").unwrap(), 1);
        let thread = commit_orchestration(&mut connection, create_thread(), None).expect("thread");
        assert_eq!(thread.disposition, "COMMITTED");

        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn projector_rejection_leaves_no_accepted_receipt() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("reject");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_orchestration_slice_schema(&mut connection).expect("schema");
        let error = commit_orchestration(&mut connection, create_thread(), None).expect_err("no project");
        assert!(
            matches!(error, OrchestrationError::ProjectorRejected("thread.created")),
            "{error:?}"
        );
        assert_eq!(count_rows(&mut connection, "orchestration_events").unwrap(), 0);
        assert_eq!(count_rows(&mut connection, "orchestration_command_receipts").unwrap(), 0);
        assert_eq!(count_rows(&mut connection, "projection_threads").unwrap(), 0);
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn stale_stream_revision_writes_nothing() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("stale");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_orchestration_slice_schema(&mut connection).expect("schema");
        commit_orchestration(&mut connection, create_project(), None).expect("project");
        commit_orchestration(&mut connection, create_thread(), None).expect("thread");
        let mut stale = send_message();
        stale.expected_stream_version = Some(0);
        let error = commit_orchestration(&mut connection, stale, None).expect_err("stale");
        assert!(matches!(error, OrchestrationError::StreamConflict), "{error:?}");
        assert_eq!(count_rows(&mut connection, "projection_thread_messages").unwrap(), 0);
        assert_eq!(count_rows(&mut connection, "orchestration_command_receipts").unwrap(), 2);
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn revoked_access_does_not_disclose_receipt_or_commit() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("access");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_orchestration_slice_schema(&mut connection).expect("schema");
        commit_orchestration(&mut connection, create_project(), None).expect("project");

        let mut denied = create_project();
        denied.access_admitted = false;
        let error = commit_orchestration(&mut connection, denied, None).expect_err("denied lookup");
        assert!(matches!(error, OrchestrationError::AccessDenied), "{error:?}");
        assert_eq!(count_rows(&mut connection, "orchestration_events").unwrap(), 1);

        let error = commit_orchestration(
            &mut connection,
            create_thread(),
            Some(FaultPoint::RevokeBeforeCommit),
        )
        .expect_err("denied before commit");
        assert!(matches!(error, OrchestrationError::AccessDenied), "{error:?}");
        assert_eq!(count_rows(&mut connection, "projection_threads").unwrap(), 0);
        assert_eq!(count_rows(&mut connection, "orchestration_command_receipts").unwrap(), 1);
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn rejected_command_writes_no_accepted_event() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("reject-cmd");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_orchestration_slice_schema(&mut connection).expect("schema");
        let receipt = record_rejected_command(
            &mut connection,
            "cmd-reject",
            "project.create",
            "project",
            "proj-1",
            at(),
            "workspace-root-taken",
            true,
        )
        .expect("rejected");
        assert_eq!(receipt.disposition, "COMMITTED");
        assert_eq!(count_rows(&mut connection, "orchestration_events").unwrap(), 0);
        assert_eq!(count_rows(&mut connection, "projection_projects").unwrap(), 0);
        let status = super::query_text(
            &mut connection,
            "SELECT status FROM orchestration_command_receipts WHERE command_id = ?",
            &["cmd-reject"],
        )
        .expect("status")
        .expect("row");
        assert_eq!(status, "rejected");
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn nine_projector_families_share_one_commit() {
        let _guard = route_b_test_guard();
        let root_path = scratch_root("nine");
        let root = RootLock::acquire(&root_path).expect("root");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_orchestration_slice_schema(&mut connection).expect("schema");
        commit_orchestration(&mut connection, create_project(), None).expect("project");
        commit_orchestration(&mut connection, create_thread(), None).expect("thread");
        let command = OrchestrationCommand {
            command_id: "cmd-nine".into(),
            command_type: "thread.work-batch".into(),
            access_admitted: true,
            expected_stream_version: Some(1),
            events: vec![
                EventProposal::MessageSent {
                    event_id: "ev-m".into(),
                    message_id: "msg-9".into(),
                    thread_id: "thr-1".into(),
                    role: "user".into(),
                    text: "go".into(),
                    occurred_at: at().into(),
                },
                EventProposal::ActivityAppended {
                    event_id: "ev-a".into(),
                    activity_id: "act-9".into(),
                    thread_id: "thr-1".into(),
                    tone: "info".into(),
                    kind: "status".into(),
                    summary: "working".into(),
                    occurred_at: at().into(),
                },
                EventProposal::SessionSet {
                    event_id: "ev-s".into(),
                    thread_id: "thr-1".into(),
                    status: "running".into(),
                    occurred_at: at().into(),
                },
                EventProposal::TurnRequested {
                    event_id: "ev-t".into(),
                    thread_id: "thr-1".into(),
                    turn_id: "turn-9".into(),
                    occurred_at: at().into(),
                },
                EventProposal::ApprovalRequested {
                    event_id: "ev-p".into(),
                    thread_id: "thr-1".into(),
                    request_id: "req-9".into(),
                    occurred_at: at().into(),
                },
                EventProposal::ProposedPlanUpserted {
                    event_id: "ev-n".into(),
                    thread_id: "thr-1".into(),
                    plan_id: "plan-9".into(),
                    markdown: "# plan".into(),
                    occurred_at: at().into(),
                },
                EventProposal::CheckpointCompleted {
                    event_id: "ev-c".into(),
                    thread_id: "thr-1".into(),
                    occurred_at: at().into(),
                },
            ],
        };
        let started = std::time::Instant::now();
        commit_orchestration(&mut connection, command, None).expect("batch");
        let elapsed = started.elapsed();
        assert!(elapsed.as_nanos() > 0);
        assert_eq!(count_rows(&mut connection, "projection_thread_messages").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "projection_thread_activities").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "projection_thread_sessions").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "projection_turns").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "projection_pending_approvals").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "projection_thread_proposed_plans").unwrap(), 1);
        assert_eq!(count_rows(&mut connection, "projection_state").unwrap(), 9);
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }
}
