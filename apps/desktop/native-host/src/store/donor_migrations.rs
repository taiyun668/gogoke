//! Donor migration SQL extracted from third_party/t3code persistence Migrations.
//! Donor TypeScript files are not modified. Dynamic steps are native-owned.

pub struct DonorMigration {
    pub id: i64,
    pub name: &'static str,
    pub statements: &'static [&'static str],
    pub dynamic: DynamicStep,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DynamicStep {
    None,
    CopyLegacyThreadPullRequests,
}

pub const DONOR_MIGRATIONS: &[DonorMigration] = &[
    DonorMigration {
        id: 1,
        name: "OrchestrationEvents",
        statements: &[
            "CREATE TABLE IF NOT EXISTS orchestration_events (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
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
 metadata_json TEXT NOT NULL
 )",
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_orch_events_stream_version
 ON orchestration_events(aggregate_kind, stream_id, stream_version)",
            "CREATE INDEX IF NOT EXISTS idx_orch_events_stream_sequence
 ON orchestration_events(aggregate_kind, stream_id, sequence)",
            "CREATE INDEX IF NOT EXISTS idx_orch_events_command_id
 ON orchestration_events(command_id)",
            "CREATE INDEX IF NOT EXISTS idx_orch_events_correlation_id
 ON orchestration_events(correlation_id)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 2,
        name: "OrchestrationCommandReceipts",
        statements: &[
            "CREATE TABLE IF NOT EXISTS orchestration_command_receipts (
 command_id TEXT PRIMARY KEY,
 aggregate_kind TEXT NOT NULL,
 aggregate_id TEXT NOT NULL,
 accepted_at TEXT NOT NULL,
 result_sequence INTEGER NOT NULL,
 status TEXT NOT NULL,
 error TEXT
 )",
            "CREATE INDEX IF NOT EXISTS idx_orch_command_receipts_aggregate
 ON orchestration_command_receipts(aggregate_kind, aggregate_id)",
            "CREATE INDEX IF NOT EXISTS idx_orch_command_receipts_sequence
 ON orchestration_command_receipts(result_sequence)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 3,
        name: "CheckpointDiffBlobs",
        statements: &[
            "CREATE TABLE IF NOT EXISTS checkpoint_diff_blobs (
 thread_id TEXT NOT NULL,
 from_turn_count INTEGER NOT NULL,
 to_turn_count INTEGER NOT NULL,
 diff TEXT NOT NULL,
 created_at TEXT NOT NULL,
 UNIQUE (thread_id, from_turn_count, to_turn_count)
 )",
            "CREATE INDEX IF NOT EXISTS idx_checkpoint_diff_blobs_thread_to_turn
 ON checkpoint_diff_blobs(thread_id, to_turn_count)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 4,
        name: "ProviderSessionRuntime",
        statements: &[
            "CREATE TABLE IF NOT EXISTS provider_session_runtime (
 thread_id TEXT PRIMARY KEY,
 provider_name TEXT NOT NULL,
 adapter_key TEXT NOT NULL,
 runtime_mode TEXT NOT NULL DEFAULT 'full-access',
 status TEXT NOT NULL,
 last_seen_at TEXT NOT NULL,
 resume_cursor_json TEXT,
 runtime_payload_json TEXT
 )",
            "CREATE INDEX IF NOT EXISTS idx_provider_session_runtime_status
 ON provider_session_runtime(status)",
            "CREATE INDEX IF NOT EXISTS idx_provider_session_runtime_provider
 ON provider_session_runtime(provider_name)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 5,
        name: "Projections",
        statements: &[
            "CREATE TABLE IF NOT EXISTS projection_projects (
 project_id TEXT PRIMARY KEY,
 title TEXT NOT NULL,
 workspace_root TEXT NOT NULL,
 default_model TEXT,
 scripts_json TEXT NOT NULL,
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL,
 deleted_at TEXT
 )",
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
 )",
            "CREATE TABLE IF NOT EXISTS projection_thread_messages (
 message_id TEXT PRIMARY KEY,
 thread_id TEXT NOT NULL,
 turn_id TEXT,
 role TEXT NOT NULL,
 text TEXT NOT NULL,
 is_streaming INTEGER NOT NULL,
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
 )",
            "CREATE TABLE IF NOT EXISTS projection_thread_activities (
 activity_id TEXT PRIMARY KEY,
 thread_id TEXT NOT NULL,
 turn_id TEXT,
 tone TEXT NOT NULL,
 kind TEXT NOT NULL,
 summary TEXT NOT NULL,
 payload_json TEXT NOT NULL,
 created_at TEXT NOT NULL
 )",
            "CREATE TABLE IF NOT EXISTS projection_thread_sessions (
 thread_id TEXT PRIMARY KEY,
 status TEXT NOT NULL,
 provider_name TEXT,
 provider_session_id TEXT,
 provider_thread_id TEXT,
 active_turn_id TEXT,
 last_error TEXT,
 updated_at TEXT NOT NULL
 )",
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
 UNIQUE (thread_id, turn_id),
 UNIQUE (thread_id, checkpoint_turn_count)
 )",
            "CREATE TABLE IF NOT EXISTS projection_pending_approvals (
 request_id TEXT PRIMARY KEY,
 thread_id TEXT NOT NULL,
 turn_id TEXT,
 status TEXT NOT NULL,
 decision TEXT,
 created_at TEXT NOT NULL,
 resolved_at TEXT
 )",
            "CREATE TABLE IF NOT EXISTS projection_state (
 projector TEXT PRIMARY KEY,
 last_applied_sequence INTEGER NOT NULL,
 updated_at TEXT NOT NULL
 )",
            "CREATE INDEX IF NOT EXISTS idx_projection_projects_updated_at
 ON projection_projects(updated_at)",
            "CREATE INDEX IF NOT EXISTS idx_projection_threads_project_id
 ON projection_threads(project_id)",
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_messages_thread_created
 ON projection_thread_messages(thread_id, created_at)",
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_activities_thread_created
 ON projection_thread_activities(thread_id, created_at)",
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_sessions_provider_session
 ON projection_thread_sessions(provider_session_id)",
            "CREATE INDEX IF NOT EXISTS idx_projection_turns_thread_requested
 ON projection_turns(thread_id, requested_at)",
            "CREATE INDEX IF NOT EXISTS idx_projection_turns_thread_checkpoint_completed
 ON projection_turns(thread_id, checkpoint_turn_count, completed_at)",
            "CREATE INDEX IF NOT EXISTS idx_projection_pending_approvals_thread_status
 ON projection_pending_approvals(thread_id, status)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 6,
        name: "ProjectionThreadSessionRuntimeModeColumns",
        statements: &[
            "ALTER TABLE projection_thread_sessions
 ADD COLUMN runtime_mode TEXT NOT NULL DEFAULT 'full-access'",
            "UPDATE projection_thread_sessions
 SET runtime_mode = 'full-access'
 WHERE runtime_mode IS NULL",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 7,
        name: "ProjectionThreadMessageAttachments",
        statements: &[
            "ALTER TABLE projection_thread_messages
 ADD COLUMN attachments_json TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 8,
        name: "ProjectionThreadActivitySequence",
        statements: &[
            "ALTER TABLE projection_thread_activities
 ADD COLUMN sequence INTEGER",
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_activities_thread_sequence
 ON projection_thread_activities(thread_id, sequence)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 9,
        name: "ProviderSessionRuntimeMode",
        statements: &[
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 10,
        name: "ProjectionThreadsRuntimeMode",
        statements: &[
            "ALTER TABLE projection_threads
 ADD COLUMN runtime_mode TEXT NOT NULL DEFAULT 'full-access'",
            "UPDATE projection_threads
 SET runtime_mode = 'full-access'
 WHERE runtime_mode IS NULL",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 11,
        name: "OrchestrationThreadCreatedRuntimeMode",
        statements: &[
            "UPDATE orchestration_events
 SET payload_json = json_set(payload_json, '$.runtimeMode', 'full-access')
 WHERE event_type = 'thread.created'
 AND json_type(payload_json, '$.runtimeMode') IS NULL",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 12,
        name: "ProjectionThreadsInteractionMode",
        statements: &[
            "ALTER TABLE projection_threads
 ADD COLUMN interaction_mode TEXT NOT NULL DEFAULT 'default'",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 13,
        name: "ProjectionThreadProposedPlans",
        statements: &[
            "CREATE TABLE IF NOT EXISTS projection_thread_proposed_plans (
 plan_id TEXT PRIMARY KEY,
 thread_id TEXT NOT NULL,
 turn_id TEXT,
 plan_markdown TEXT NOT NULL,
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
 )",
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_proposed_plans_thread_created
 ON projection_thread_proposed_plans(thread_id, created_at)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 14,
        name: "ProjectionThreadProposedPlanImplementation",
        statements: &[
            "ALTER TABLE projection_thread_proposed_plans
 ADD COLUMN implemented_at TEXT",
            "ALTER TABLE projection_thread_proposed_plans
 ADD COLUMN implementation_thread_id TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 15,
        name: "ProjectionTurnsSourceProposedPlan",
        statements: &[
            "ALTER TABLE projection_turns
 ADD COLUMN source_proposed_plan_thread_id TEXT",
            "ALTER TABLE projection_turns
 ADD COLUMN source_proposed_plan_id TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 16,
        name: "CanonicalizeModelSelections",
        statements: &[
            "ALTER TABLE projection_projects
 ADD COLUMN default_model_selection_json TEXT",
            "UPDATE projection_projects
 SET default_model_selection_json = CASE
 WHEN default_model IS NULL THEN NULL
 ELSE json_object(
 'provider',
 CASE
 WHEN lower(default_model) LIKE '%claude%' THEN 'claudeAgent'
 ELSE 'codex'
 END,
 'model',
 default_model
 )
 END
 WHERE default_model_selection_json IS NULL",
            "ALTER TABLE projection_threads
 ADD COLUMN model_selection_json TEXT",
            "UPDATE projection_threads
 SET model_selection_json = json_object(
 'provider',
 COALESCE(
 (
 SELECT provider_name
 FROM projection_thread_sessions
 WHERE projection_thread_sessions.thread_id = projection_threads.thread_id
 ),
 CASE
 WHEN lower(model) LIKE '%claude%' THEN 'claudeAgent'
 ELSE 'codex'
 END,
 'codex'
 ),
 'model',
 model
 )
 WHERE model_selection_json IS NULL",
            "ALTER TABLE projection_projects
 DROP COLUMN default_model",
            "ALTER TABLE projection_threads
 DROP COLUMN model",
            "UPDATE orchestration_events
 SET payload_json = CASE
 WHEN json_type(payload_json, '$.defaultModel') = 'null' THEN json_remove(
 json_set(payload_json, '$.defaultModelSelection', json('null')),
 '$.defaultProvider',
 '$.defaultModel',
 '$.defaultModelOptions'
 )
 ELSE json_remove(
 json_set(
 payload_json,
 '$.defaultModelSelection',
 json_patch(
 json_object(
 'provider',
 CASE
 WHEN json_extract(payload_json, '$.defaultProvider') IS NOT NULL
 THEN json_extract(payload_json, '$.defaultProvider')
 WHEN lower(json_extract(payload_json, '$.defaultModel')) LIKE '%claude%'
 THEN 'claudeAgent'
 ELSE 'codex'
 END,
 'model',
 json_extract(payload_json, '$.defaultModel')
 ),
 CASE
 WHEN json_type(payload_json, '$.defaultModelOptions') IS NULL THEN '{}'
 WHEN json_type(payload_json, '$.defaultModelOptions.codex') IS NOT NULL
 OR json_type(payload_json, '$.defaultModelOptions.claudeAgent') IS NOT NULL
 THEN CASE
 WHEN (
 CASE
 WHEN json_extract(payload_json, '$.defaultProvider') IS NOT NULL
 THEN json_extract(payload_json, '$.defaultProvider')
 WHEN lower(json_extract(payload_json, '$.defaultModel')) LIKE '%claude%'
 THEN 'claudeAgent'
 ELSE 'codex'
 END
 ) = 'claudeAgent'
 THEN CASE
 WHEN json_type(payload_json, '$.defaultModelOptions.claudeAgent') IS NOT NULL
 THEN json_object(
 'options',
 json(json_extract(payload_json, '$.defaultModelOptions.claudeAgent'))
 )
 WHEN json_type(payload_json, '$.defaultModelOptions.codex') IS NOT NULL
 THEN json_object(
 'options',
 json(json_extract(payload_json, '$.defaultModelOptions.codex'))
 )
 ELSE '{}'
 END
 ELSE CASE
 WHEN json_type(payload_json, '$.defaultModelOptions.codex') IS NOT NULL
 THEN json_object(
 'options',
 json(json_extract(payload_json, '$.defaultModelOptions.codex'))
 )
 WHEN json_type(payload_json, '$.defaultModelOptions.claudeAgent') IS NOT NULL
 THEN json_object(
 'options',
 json(json_extract(payload_json, '$.defaultModelOptions.claudeAgent'))
 )
 ELSE '{}'
 END
 END
 ELSE json_object(
 'options',
 json(json_extract(payload_json, '$.defaultModelOptions'))
 )
 END
 )
 ),
 '$.defaultProvider',
 '$.defaultModel',
 '$.defaultModelOptions'
 )
 END
 WHERE event_type IN ('project.created', 'project.meta-updated')
 AND json_type(payload_json, '$.defaultModelSelection') IS NULL
 AND json_type(payload_json, '$.defaultModel') IS NOT NULL",
            "UPDATE orchestration_events
 SET payload_json = json_remove(
 json_set(
 payload_json,
 '$.modelSelection',
 json_patch(
 json_object(
 'provider',
 CASE
 WHEN json_extract(payload_json, '$.provider') IS NOT NULL
 THEN json_extract(payload_json, '$.provider')
 WHEN lower(json_extract(payload_json, '$.model')) LIKE '%claude%'
 THEN 'claudeAgent'
 ELSE 'codex'
 END,
 'model',
 json_extract(payload_json, '$.model')
 ),
 CASE
 WHEN json_type(payload_json, '$.modelOptions') IS NULL THEN '{}'
 WHEN json_type(payload_json, '$.modelOptions.codex') IS NOT NULL
 OR json_type(payload_json, '$.modelOptions.claudeAgent') IS NOT NULL
 THEN CASE
 WHEN (
 CASE
 WHEN json_extract(payload_json, '$.provider') IS NOT NULL
 THEN json_extract(payload_json, '$.provider')
 WHEN lower(json_extract(payload_json, '$.model')) LIKE '%claude%'
 THEN 'claudeAgent'
 ELSE 'codex'
 END
 ) = 'claudeAgent'
 THEN CASE
 WHEN json_type(payload_json, '$.modelOptions.claudeAgent') IS NOT NULL
 THEN json_object(
 'options',
 json(json_extract(payload_json, '$.modelOptions.claudeAgent'))
 )
 WHEN json_type(payload_json, '$.modelOptions.codex') IS NOT NULL
 THEN json_object(
 'options',
 json(json_extract(payload_json, '$.modelOptions.codex'))
 )
 ELSE '{}'
 END
 ELSE CASE
 WHEN json_type(payload_json, '$.modelOptions.codex') IS NOT NULL
 THEN json_object(
 'options',
 json(json_extract(payload_json, '$.modelOptions.codex'))
 )
 WHEN json_type(payload_json, '$.modelOptions.claudeAgent') IS NOT NULL
 THEN json_object(
 'options',
 json(json_extract(payload_json, '$.modelOptions.claudeAgent'))
 )
 ELSE '{}'
 END
 END
 ELSE json_object('options', json(json_extract(payload_json, '$.modelOptions')))
 END
 )
 ),
 '$.provider',
 '$.model',
 '$.modelOptions'
 )
 WHERE event_type IN ('thread.created', 'thread.meta-updated', 'thread.turn-start-requested')
 AND json_type(payload_json, '$.modelSelection') IS NULL
 AND json_type(payload_json, '$.model') IS NOT NULL",
            "UPDATE orchestration_events
 SET payload_json = json_set(
 payload_json,
 '$.modelSelection',
 json(json_object('provider', 'codex', 'model', 'gpt-5.4'))
 )
 WHERE event_type = 'thread.created'
 AND json_type(payload_json, '$.modelSelection') IS NULL
 AND json_type(payload_json, '$.model') IS NULL",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 17,
        name: "ProjectionThreadsArchivedAt",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN archived_at TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 18,
        name: "ProjectionThreadsArchivedAtIndex",
        statements: &[
            "CREATE INDEX IF NOT EXISTS idx_projection_threads_project_archived_at
 ON projection_threads(project_id, archived_at)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 19,
        name: "ProjectionSnapshotLookupIndexes",
        statements: &[
            "CREATE INDEX IF NOT EXISTS idx_projection_projects_workspace_root_deleted_at
 ON projection_projects(workspace_root, deleted_at)",
            "CREATE INDEX IF NOT EXISTS idx_projection_threads_project_deleted_created
 ON projection_threads(project_id, deleted_at, created_at)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 20,
        name: "AuthAccessManagement",
        statements: &[
            "CREATE TABLE IF NOT EXISTS auth_pairing_links (
 id TEXT PRIMARY KEY,
 credential TEXT NOT NULL UNIQUE,
 method TEXT NOT NULL,
 role TEXT NOT NULL,
 subject TEXT NOT NULL,
 created_at TEXT NOT NULL,
 expires_at TEXT NOT NULL,
 consumed_at TEXT,
 revoked_at TEXT
 )",
            "CREATE INDEX IF NOT EXISTS idx_auth_pairing_links_active
 ON auth_pairing_links(revoked_at, consumed_at, expires_at)",
            "CREATE TABLE IF NOT EXISTS auth_sessions (
 session_id TEXT PRIMARY KEY,
 subject TEXT NOT NULL,
 role TEXT NOT NULL,
 method TEXT NOT NULL,
 issued_at TEXT NOT NULL,
 expires_at TEXT NOT NULL,
 revoked_at TEXT
 )",
            "CREATE INDEX IF NOT EXISTS idx_auth_sessions_active
 ON auth_sessions(revoked_at, expires_at, issued_at)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 21,
        name: "AuthSessionClientMetadata",
        statements: &[
            "PRAGMA table_info(auth_pairing_links)",
            "ALTER TABLE auth_pairing_links
 ADD COLUMN label TEXT",
            "PRAGMA table_info(auth_sessions)",
            "ALTER TABLE auth_sessions
 ADD COLUMN client_label TEXT",
            "ALTER TABLE auth_sessions
 ADD COLUMN client_ip_address TEXT",
            "ALTER TABLE auth_sessions
 ADD COLUMN client_user_agent TEXT",
            "ALTER TABLE auth_sessions
 ADD COLUMN client_device_type TEXT NOT NULL DEFAULT 'unknown'",
            "ALTER TABLE auth_sessions
 ADD COLUMN client_os TEXT",
            "ALTER TABLE auth_sessions
 ADD COLUMN client_browser TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 22,
        name: "AuthSessionLastConnectedAt",
        statements: &[
            "PRAGMA table_info(auth_sessions)",
            "ALTER TABLE auth_sessions
 ADD COLUMN last_connected_at TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 23,
        name: "ProjectionThreadShellSummary",
        statements: &[
            "ALTER TABLE projection_threads
 ADD COLUMN latest_user_message_at TEXT",
            "ALTER TABLE projection_threads
 ADD COLUMN pending_approval_count INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE projection_threads
 ADD COLUMN pending_user_input_count INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE projection_threads
 ADD COLUMN has_actionable_proposed_plan INTEGER NOT NULL DEFAULT 0",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 24,
        name: "BackfillProjectionThreadShellSummary",
        statements: &[
            "INSERT OR IGNORE INTO projection_pending_approvals (
 request_id,
 thread_id,
 turn_id,
 status,
 decision,
 created_at,
 resolved_at
 )
 SELECT
 requested.request_id,
 requested.thread_id,
 requested.turn_id,
 'pending',
 NULL,
 requested.created_at,
 NULL
 FROM (
 SELECT
 json_extract(payload_json, '$.requestId') AS request_id,
 thread_id,
 turn_id,
 created_at,
 ROW_NUMBER() OVER (
 PARTITION BY json_extract(payload_json, '$.requestId')
 ORDER BY created_at ASC, activity_id ASC
 ) AS row_number
 FROM projection_thread_activities
 WHERE kind = 'approval.requested'
 AND json_extract(payload_json, '$.requestId') IS NOT NULL
 ) AS requested
 WHERE requested.row_number = 1",
            "WITH latest_resolutions AS (
 SELECT
 resolved.request_id,
 resolved.resolved_at,
 resolved.decision
 FROM (
 SELECT
 json_extract(payload_json, '$.requestId') AS request_id,
 created_at AS resolved_at,
 CASE
 WHEN json_extract(payload_json, '$.decision') IN (
 'accept',
 'acceptForSession',
 'decline',
 'cancel'
 )
 THEN json_extract(payload_json, '$.decision')
 ELSE NULL
 END AS decision,
 ROW_NUMBER() OVER (
 PARTITION BY json_extract(payload_json, '$.requestId')
 ORDER BY created_at DESC, activity_id DESC
 ) AS row_number
 FROM projection_thread_activities
 WHERE kind = 'approval.resolved'
 AND json_extract(payload_json, '$.requestId') IS NOT NULL
 ) AS resolved
 WHERE resolved.row_number = 1
 )
 UPDATE projection_pending_approvals
 SET
 status = 'resolved',
 decision = (
 SELECT latest_resolutions.decision
 FROM latest_resolutions
 WHERE latest_resolutions.request_id = projection_pending_approvals.request_id
 ),
 resolved_at = (
 SELECT latest_resolutions.resolved_at
 FROM latest_resolutions
 WHERE latest_resolutions.request_id = projection_pending_approvals.request_id
 )
 WHERE EXISTS (
 SELECT 1
 FROM latest_resolutions
 WHERE latest_resolutions.request_id = projection_pending_approvals.request_id
 )",
            "WITH latest_response_events AS (
 SELECT
 response.request_id,
 response.resolved_at,
 response.decision
 FROM (
 SELECT
 json_extract(payload_json, '$.requestId') AS request_id,
 occurred_at AS resolved_at,
 CASE
 WHEN json_extract(payload_json, '$.decision') IN (
 'accept',
 'acceptForSession',
 'decline',
 'cancel'
 )
 THEN json_extract(payload_json, '$.decision')
 ELSE NULL
 END AS decision,
 ROW_NUMBER() OVER (
 PARTITION BY json_extract(payload_json, '$.requestId')
 ORDER BY occurred_at DESC, sequence DESC
 ) AS row_number
 FROM orchestration_events
 WHERE event_type = 'thread.approval-response-requested'
 AND json_extract(payload_json, '$.requestId') IS NOT NULL
 ) AS response
 WHERE response.row_number = 1
 )
 UPDATE projection_pending_approvals
 SET
 status = 'resolved',
 decision = (
 SELECT latest_response_events.decision
 FROM latest_response_events
 WHERE latest_response_events.request_id = projection_pending_approvals.request_id
 ),
 resolved_at = (
 SELECT latest_response_events.resolved_at
 FROM latest_response_events
 WHERE latest_response_events.request_id = projection_pending_approvals.request_id
 )
 WHERE EXISTS (
 SELECT 1
 FROM latest_response_events
 WHERE latest_response_events.request_id = projection_pending_approvals.request_id
 )",
            "WITH latest_stale_failures AS (
 SELECT
 failure.request_id,
 failure.resolved_at
 FROM (
 SELECT
 json_extract(payload_json, '$.requestId') AS request_id,
 created_at AS resolved_at,
 ROW_NUMBER() OVER (
 PARTITION BY json_extract(payload_json, '$.requestId')
 ORDER BY created_at DESC, activity_id DESC
 ) AS row_number
 FROM projection_thread_activities
 WHERE kind = 'provider.approval.respond.failed'
 AND json_extract(payload_json, '$.requestId') IS NOT NULL
 AND (
 lower(COALESCE(json_extract(payload_json, '$.detail'), ''))
 LIKE '%stale pending approval request%'
 OR lower(COALESCE(json_extract(payload_json, '$.detail'), ''))
 LIKE '%unknown pending approval request%'
 OR lower(COALESCE(json_extract(payload_json, '$.detail'), ''))
 LIKE '%unknown pending permission request%'
 )
 ) AS failure
 WHERE failure.row_number = 1
 )
 UPDATE projection_pending_approvals
 SET
 status = 'resolved',
 decision = NULL,
 resolved_at = (
 SELECT latest_stale_failures.resolved_at
 FROM latest_stale_failures
 WHERE latest_stale_failures.request_id = projection_pending_approvals.request_id
 )
 WHERE status = 'pending'
 AND EXISTS (
 SELECT 1
 FROM latest_stale_failures
 WHERE latest_stale_failures.request_id = projection_pending_approvals.request_id
 )",
            "UPDATE projection_threads
 SET
 latest_user_message_at = (
 SELECT MAX(message.created_at)
 FROM projection_thread_messages AS message
 WHERE message.thread_id = projection_threads.thread_id
 AND message.role = 'user'
 ),
 pending_approval_count = COALESCE((
 SELECT COUNT(*)
 FROM projection_pending_approvals
 WHERE projection_pending_approvals.thread_id = projection_threads.thread_id
 AND projection_pending_approvals.status = 'pending'
 ), 0),
 pending_user_input_count = COALESCE((
 WITH latest_user_input_states AS (
 SELECT
 latest.request_id,
 latest.kind,
 latest.detail
 FROM (
 SELECT
 json_extract(activity.payload_json, '$.requestId') AS request_id,
 activity.kind,
 lower(COALESCE(json_extract(activity.payload_json, '$.detail'), '')) AS detail,
 ROW_NUMBER() OVER (
 PARTITION BY json_extract(activity.payload_json, '$.requestId')
 ORDER BY activity.created_at DESC, activity.activity_id DESC
 ) AS row_number
 FROM projection_thread_activities AS activity
 WHERE activity.thread_id = projection_threads.thread_id
 AND json_extract(activity.payload_json, '$.requestId') IS NOT NULL
 AND activity.kind IN (
 'user-input.requested',
 'user-input.resolved',
 'provider.user-input.respond.failed'
 )
 ) AS latest
 WHERE latest.row_number = 1
 )
 SELECT COUNT(*)
 FROM latest_user_input_states
 WHERE latest_user_input_states.kind = 'user-input.requested'
 OR (
 latest_user_input_states.kind = 'provider.user-input.respond.failed'
 AND latest_user_input_states.detail NOT LIKE '%stale pending user-input request%'
 AND latest_user_input_states.detail NOT LIKE '%unknown pending user-input request%'
 )
 ), 0),
 has_actionable_proposed_plan = COALESCE((
 SELECT CASE
 WHEN projection_threads.latest_turn_id IS NOT NULL
 AND EXISTS (
 SELECT 1
 FROM projection_thread_proposed_plans AS latest_turn_plan_exists
 WHERE latest_turn_plan_exists.thread_id = projection_threads.thread_id
 AND latest_turn_plan_exists.turn_id = projection_threads.latest_turn_id
 )
 THEN CASE
 WHEN (
 SELECT latest_turn_plan.implemented_at
 FROM projection_thread_proposed_plans AS latest_turn_plan
 WHERE latest_turn_plan.thread_id = projection_threads.thread_id
 AND latest_turn_plan.turn_id = projection_threads.latest_turn_id
 ORDER BY latest_turn_plan.updated_at DESC, latest_turn_plan.plan_id DESC
 LIMIT 1
 ) IS NULL
 THEN 1
 ELSE 0
 END
 WHEN EXISTS (
 SELECT 1
 FROM projection_thread_proposed_plans AS any_plan
 WHERE any_plan.thread_id = projection_threads.thread_id
 )
 THEN CASE
 WHEN (
 SELECT latest_plan.implemented_at
 FROM projection_thread_proposed_plans AS latest_plan
 WHERE latest_plan.thread_id = projection_threads.thread_id
 ORDER BY latest_plan.updated_at DESC, latest_plan.plan_id DESC
 LIMIT 1
 ) IS NULL
 THEN 1
 ELSE 0
 END
 ELSE 0
 END
 ), 0)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 25,
        name: "CleanupInvalidProjectionPendingApprovals",
        statements: &[
            "DELETE FROM projection_pending_approvals
 WHERE NOT EXISTS (
 SELECT 1
 FROM projection_thread_activities AS activity
 WHERE activity.kind = 'approval.requested'
 AND json_extract(activity.payload_json, '$.requestId')
 = projection_pending_approvals.request_id
 )",
            "UPDATE projection_threads
 SET pending_approval_count = COALESCE((
 SELECT COUNT(*)
 FROM projection_pending_approvals
 WHERE projection_pending_approvals.thread_id = projection_threads.thread_id
 AND projection_pending_approvals.status = 'pending'
 ), 0)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 26,
        name: "CanonicalizeModelSelectionOptions",
        statements: &[
            "UPDATE projection_threads
 SET model_selection_json = json_set(
 model_selection_json,
 '$.options',
 (
 SELECT json_group_array(
 json_object(
 'id', key,
 'value',
 CASE type
 WHEN 'true' THEN json('true')
 WHEN 'false' THEN json('false')
 ELSE atom
 END
 )
 )
 FROM json_each(json_extract(model_selection_json, '$.options'))
 WHERE (type = 'text' AND trim(coalesce(atom, '')) != '')
 OR type IN ('true', 'false')
 )
 )
 WHERE model_selection_json IS NOT NULL
 AND json_type(model_selection_json, '$.options') = 'object'",
            "UPDATE projection_projects
 SET default_model_selection_json = json_set(
 default_model_selection_json,
 '$.options',
 (
 SELECT json_group_array(
 json_object(
 'id', key,
 'value',
 CASE type
 WHEN 'true' THEN json('true')
 WHEN 'false' THEN json('false')
 ELSE atom
 END
 )
 )
 FROM json_each(json_extract(default_model_selection_json, '$.options'))
 WHERE (type = 'text' AND trim(coalesce(atom, '')) != '')
 OR type IN ('true', 'false')
 )
 )
 WHERE default_model_selection_json IS NOT NULL
 AND json_type(default_model_selection_json, '$.options') = 'object'",
            "UPDATE orchestration_events
 SET payload_json = json_set(
 payload_json,
 '$.modelSelection.options',
 (
 SELECT json_group_array(
 json_object(
 'id', key,
 'value',
 CASE type
 WHEN 'true' THEN json('true')
 WHEN 'false' THEN json('false')
 ELSE atom
 END
 )
 )
 FROM json_each(json_extract(payload_json, '$.modelSelection.options'))
 WHERE (type = 'text' AND trim(coalesce(atom, '')) != '')
 OR type IN ('true', 'false')
 )
 )
 WHERE event_type IN (
 'thread.created',
 'thread.meta-updated',
 'thread.turn-start-requested'
 )
 AND json_type(payload_json, '$.modelSelection.options') = 'object'",
            "UPDATE orchestration_events
 SET payload_json = json_set(
 payload_json,
 '$.defaultModelSelection.options',
 (
 SELECT json_group_array(
 json_object(
 'id', key,
 'value',
 CASE type
 WHEN 'true' THEN json('true')
 WHEN 'false' THEN json('false')
 ELSE atom
 END
 )
 )
 FROM json_each(json_extract(payload_json, '$.defaultModelSelection.options'))
 WHERE (type = 'text' AND trim(coalesce(atom, '')) != '')
 OR type IN ('true', 'false')
 )
 )
 WHERE event_type IN ('project.created', 'project.meta-updated')
 AND json_type(payload_json, '$.defaultModelSelection.options') = 'object'",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 27,
        name: "ProviderSessionRuntimeInstanceId",
        statements: &[
            "PRAGMA table_info(provider_session_runtime)",
            "ALTER TABLE provider_session_runtime
 ADD COLUMN provider_instance_id TEXT",
            "CREATE INDEX IF NOT EXISTS idx_provider_session_runtime_instance
 ON provider_session_runtime(provider_instance_id)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 28,
        name: "ProjectionThreadSessionInstanceId",
        statements: &[
            "PRAGMA table_info(projection_thread_sessions)",
            "ALTER TABLE projection_thread_sessions
 ADD COLUMN provider_instance_id TEXT",
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_sessions_instance
 ON projection_thread_sessions(provider_instance_id)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 29,
        name: "ProjectionThreadDetailOrderingIndexes",
        statements: &[
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_activities_thread_sequence_created_id
 ON projection_thread_activities(thread_id, sequence, created_at, activity_id)",
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_messages_thread_created_id
 ON projection_thread_messages(thread_id, created_at, message_id)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 30,
        name: "ProjectionThreadShellArchiveIndexes",
        statements: &[
            "CREATE INDEX IF NOT EXISTS idx_projection_threads_shell_active
 ON projection_threads(deleted_at, archived_at, project_id, created_at, thread_id)",
            "CREATE INDEX IF NOT EXISTS idx_projection_threads_shell_archived
 ON projection_threads(deleted_at, archived_at, project_id, thread_id)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 31,
        name: "AuthAuthorizationScopes",
        statements: &[
            "DROP TABLE IF EXISTS auth_pairing_links",
            "DROP TABLE IF EXISTS auth_sessions",
            "CREATE TABLE auth_pairing_links (
 id TEXT PRIMARY KEY,
 credential TEXT NOT NULL UNIQUE,
 method TEXT NOT NULL,
 scopes TEXT NOT NULL,
 subject TEXT NOT NULL,
 label TEXT,
 created_at TEXT NOT NULL,
 expires_at TEXT NOT NULL,
 consumed_at TEXT,
 revoked_at TEXT
 )",
            "CREATE INDEX idx_auth_pairing_links_active
 ON auth_pairing_links(revoked_at, consumed_at, expires_at)",
            "CREATE TABLE auth_sessions (
 session_id TEXT PRIMARY KEY,
 subject TEXT NOT NULL,
 scopes TEXT NOT NULL,
 method TEXT NOT NULL,
 client_label TEXT,
 client_ip_address TEXT,
 client_user_agent TEXT,
 client_device_type TEXT NOT NULL DEFAULT 'unknown',
 client_os TEXT,
 client_browser TEXT,
 issued_at TEXT NOT NULL,
 expires_at TEXT NOT NULL,
 last_connected_at TEXT,
 revoked_at TEXT
 )",
            "CREATE INDEX idx_auth_sessions_active
 ON auth_sessions(revoked_at, expires_at, issued_at)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 32,
        name: "AuthPairingProofKeyThumbprint",
        statements: &[
            "PRAGMA table_info(auth_pairing_links)",
            "ALTER TABLE auth_pairing_links
 ADD COLUMN proof_key_thumbprint TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 33,
        name: "ProjectionThreadsSettled",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN settled_override TEXT",
            "ALTER TABLE projection_threads
 ADD COLUMN settled_at TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 34,
        name: "ProjectionThreadsSnoozed",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN snoozed_until TEXT",
            "ALTER TABLE projection_threads
 ADD COLUMN snoozed_at TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 35,
        name: "ProjectionThreadTitleRegeneration",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN title_regeneration_request_id TEXT",
            "ALTER TABLE projection_threads
 ADD COLUMN title_regeneration_started_at TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 36,
        name: "ProjectionThreadsPinned",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN pinned_at TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 37,
        name: "ProjectionTurnsKeysetIndex",
        statements: &[
            "CREATE INDEX IF NOT EXISTS idx_projection_turns_thread_keyset
 ON projection_turns(thread_id, requested_at, turn_id)",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 38,
        name: "ProjectionThreadsPinOrderKey",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN pin_order_key TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 39,
        name: "ProjectionProjectsDefaultThreadEnvMode",
        statements: &[
            "PRAGMA table_info(projection_projects)",
            "ALTER TABLE projection_projects
 ADD COLUMN default_thread_env_mode TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 40,
        name: "ProjectionProjectFaviconPath",
        statements: &[
            "PRAGMA table_info(projection_projects)",
            "ALTER TABLE projection_projects
 ADD COLUMN favicon_path TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 41,
        name: "AuthSessionClientConnection",
        statements: &[
            "PRAGMA table_info(auth_sessions)",
            "ALTER TABLE auth_sessions
 ADD COLUMN client_surface TEXT",
            "ALTER TABLE auth_sessions
 ADD COLUMN client_app_version TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 42,
        name: "ProjectionThreadLinkedPullRequest",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN linked_pull_request_json TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 43,
        name: "ProjectionThreadsUnsettledAt",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN unsettled_at TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 44,
        name: "ClearAutomaticProjectModelDefaults",
        statements: &[
            "WITH automatically_seeded_projects AS (
 SELECT created.stream_id AS project_id
 FROM orchestration_events AS created
 WHERE created.aggregate_kind = 'project'
 AND created.event_type = 'project.created'
 AND json_type(created.payload_json, '$.defaultModelSelection') IS NOT NULL
 AND json_type(created.payload_json, '$.defaultModelSelection') <> 'null'
 AND NOT EXISTS (
 SELECT 1
 FROM orchestration_events AS configured
 WHERE configured.aggregate_kind = 'project'
 AND configured.stream_id = created.stream_id
 AND configured.event_type = 'project.meta-updated'
 AND json_type(configured.payload_json, '$.defaultModelSelection') IS NOT NULL
 )
 )
 UPDATE projection_projects
 SET default_model_selection_json = NULL
 WHERE project_id IN (SELECT project_id FROM automatically_seeded_projects)",
            "UPDATE orchestration_events AS created
 SET payload_json = json_set(
 created.payload_json,
 '$.defaultModelSelection',
 json('null')
 )
 WHERE created.aggregate_kind = 'project'
 AND created.event_type = 'project.created'
 AND json_type(created.payload_json, '$.defaultModelSelection') IS NOT NULL
 AND json_type(created.payload_json, '$.defaultModelSelection') <> 'null'
 AND NOT EXISTS (
 SELECT 1
 FROM orchestration_events AS configured
 WHERE configured.aggregate_kind = 'project'
 AND configured.stream_id = created.stream_id
 AND configured.event_type = 'project.meta-updated'
 AND json_type(configured.payload_json, '$.defaultModelSelection') IS NOT NULL
 )",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 45,
        name: "ProjectionProjectsAutoPull",
        statements: &[
            "PRAGMA table_info(projection_projects)",
            "ALTER TABLE projection_projects
 ADD COLUMN auto_pull INTEGER NOT NULL DEFAULT 0",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 46,
        name: "RepairAutomaticSettlementTimestamps",
        statements: &[
            "WITH activity_timestamps AS (
 SELECT thread_id, created_at AS activity_at
 FROM projection_thread_messages
 WHERE role = 'user'
 UNION ALL
 SELECT thread_id, requested_at
 FROM projection_turns
 UNION ALL
 SELECT thread_id, started_at
 FROM projection_turns
 WHERE started_at IS NOT NULL
 UNION ALL
 SELECT thread_id, completed_at
 FROM projection_turns
 WHERE completed_at IS NOT NULL
 ),
 automatic_settlements AS (
 SELECT
 stream_id AS thread_id,
 occurred_at,
 json_extract(payload_json, '$.settledAt') AS settled_at
 FROM orchestration_events
 WHERE aggregate_kind = 'thread'
 AND event_type = 'thread.settled'
 AND actor_kind = 'server'
 AND command_id LIKE 'server:auto-settle:%'
 AND json_type(payload_json, '$.settledAt') = 'text'
 AND json_extract(payload_json, '$.settledAt') = occurred_at
 )
 UPDATE projection_threads AS thread
 SET settled_at = (
 SELECT COALESCE(
 (
 SELECT activity.activity_at
 FROM activity_timestamps AS activity
 WHERE activity.thread_id = thread.thread_id
 AND julianday(activity.activity_at) IS NOT NULL
 AND julianday(activity.activity_at) <= julianday(automatic.occurred_at)
 ORDER BY julianday(activity.activity_at) DESC
 LIMIT 1
 ),
 thread.created_at
 )
 FROM automatic_settlements AS automatic
 WHERE automatic.thread_id = thread.thread_id
 AND automatic.settled_at = thread.settled_at
 LIMIT 1
 )
 WHERE thread.settled_override = 'settled'
 AND EXISTS (
 SELECT 1
 FROM automatic_settlements AS automatic
 WHERE automatic.thread_id = thread.thread_id
 AND automatic.settled_at = thread.settled_at
 )",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 47,
        name: "ProjectionProjectIcon",
        statements: &[
            "PRAGMA table_info(projection_projects)",
            "ALTER TABLE projection_projects
 ADD COLUMN project_icon_json TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 48,
        name: "ProjectionThreadBranchPullRequest",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN branch_pull_request_json TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 49,
        name: "ProjectionThreadsActiveOrderKey",
        statements: &[
            "PRAGMA table_info(projection_threads)",
            "ALTER TABLE projection_threads
 ADD COLUMN active_order_key TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 50,
        name: "ProjectionThreadPullRequests",
        statements: &[
            "CREATE TABLE IF NOT EXISTS projection_thread_pull_requests (
 thread_id TEXT NOT NULL,
 host TEXT NOT NULL,
 repository TEXT NOT NULL,
 number INTEGER NOT NULL,
 url TEXT NOT NULL,
 source TEXT NOT NULL,
 linked_at TEXT NOT NULL,
 snapshot_json TEXT,
 stack_json TEXT,
 PRIMARY KEY (thread_id, host, repository, number)
 )",
            "CREATE INDEX IF NOT EXISTS idx_projection_thread_pull_requests_pr
 ON projection_thread_pull_requests(host, repository, number)",
            "SELECT
 thread_id AS \"threadId\",
 updated_at AS \"updatedAt\",
 linked_pull_request_json AS \"linkedPullRequestJson\"
 FROM projection_threads
 WHERE linked_pull_request_json IS NOT NULL",
        ],
        dynamic: DynamicStep::CopyLegacyThreadPullRequests,
    },
    DonorMigration {
        id: 51,
        name: "ProjectionThreadMessageContext",
        statements: &[
            "PRAGMA table_info(projection_thread_messages)",
            "ALTER TABLE projection_thread_messages
 ADD COLUMN context_json TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 52,
        name: "ProjectionThreadTitleState",
        statements: &[
            "ALTER TABLE projection_threads ADD COLUMN title_state_json TEXT",
        ],
        dynamic: DynamicStep::None,
    },
    DonorMigration {
        id: 53,
        name: "PullRequestFilesViewed",
        statements: &[
            "CREATE TABLE IF NOT EXISTS pull_request_files_viewed (
 provider TEXT NOT NULL,
 host TEXT NOT NULL,
 repository TEXT NOT NULL,
 number INTEGER NOT NULL,
 viewer TEXT NOT NULL,
 path TEXT NOT NULL,
 revision TEXT,
 viewed_at TEXT NOT NULL,
 PRIMARY KEY (provider, host, repository, number, viewer, path)
 ) WITHOUT ROWID",
        ],
        dynamic: DynamicStep::None,
    },
];
