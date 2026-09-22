use super::session_lineage::{
    apply_session_lineage_command, initialize_session_lineage_schema, read_exposure_receipt,
    read_session_lineage, ExposureReceipt, NativeSessionIdentity, NativeSourceCoverage,
    PendingActionRef, SessionLineageCommand, SessionLineageOperation, SourceObservation,
};
use super::transaction;
use super::{initialize_profile, SessionLineageReceipt};
use crate::root::RootLock;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-session-lineage-{nonce}"));
    std::fs::create_dir(&path).unwrap();
    path
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path.join("state.sqlite"));
    let _ = std::fs::remove_file(path.join("state.sqlite-wal"));
    let _ = std::fs::remove_file(path.join("state.sqlite-shm"));
    let _ = std::fs::remove_dir(path);
}

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
    let _guard = route_b_test_guard();
    let path = scratch();
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    crate::store::atomic::apply_core_schema(&mut connection).unwrap();
    let _owner = initialize_profile(&mut connection, &root).unwrap();
    initialize_session_lineage_schema(&mut connection).unwrap();
    run(&mut connection);
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

fn native(native_id: &str, binding: &str, generation: &str, epoch: &str) -> NativeSessionIdentity {
    NativeSessionIdentity {
        native_session_id: native_id.into(),
        binding_id: binding.into(),
        generation: generation.into(),
        source_epoch: epoch.into(),
        domain_id: "domain-one".into(),
    }
}

fn command(operation_id: &str, operation: SessionLineageOperation) -> SessionLineageCommand {
    SessionLineageCommand {
        operation_id: operation_id.into(),
        domain_id: "domain-one".into(),
        event_id: format!("event-{operation_id}"),
        receipt_id: format!("receipt-{operation_id}"),
        recorded_at: "2026-09-22T12:00:00Z".into(),
        operation,
    }
}

fn create_session(
    connection: &mut VerifiedDatabaseConnection<'_>,
    session_id: &str,
    native: NativeSessionIdentity,
) -> SessionLineageReceipt {
    apply_session_lineage_command(
        connection,
        &command(
            &format!("create-{session_id}"),
            SessionLineageOperation::NewClean {
                session_id: session_id.into(),
                native,
            },
        ),
    )
    .unwrap()
}

fn exposure(
    receipt_id: &str,
    binding_id: &str,
    generation: &str,
    taint: &[&str],
) -> ExposureReceipt {
    ExposureReceipt {
        receipt_id: receipt_id.into(),
        manifest_id: "manifest-one".into(),
        binding_id: binding_id.into(),
        generation: generation.into(),
        evidence_level: "NATIVE_ACKED".into(),
        native_source_coverage: NativeSourceCoverage {
            complete: true,
            observations: vec![SourceObservation {
                source_ref: "manifest-source-one".into(),
                status: "COMPLETE".into(),
            }],
            unknown_sources: Vec::new(),
            inherited_from_receipt_id: None,
        },
        taint_labels: taint.iter().map(|value| (*value).into()).collect(),
        evidence_refs: vec!["evidence-native-ack".into()],
    }
}

fn retain_action(
    connection: &mut VerifiedDatabaseConnection<'_>,
    session_id: &str,
    expected_revision: &str,
    operation_id: &str,
    binding_id: &str,
    generation: &str,
) -> SessionLineageReceipt {
    apply_session_lineage_command(
        connection,
        &command(
            operation_id,
            SessionLineageOperation::RetainPendingAction {
                session_id: session_id.into(),
                expected_revision: expected_revision.into(),
                action: PendingActionRef {
                    action_id: format!("action-{operation_id}"),
                    operation_id: format!("action-op-{operation_id}"),
                    binding_id: binding_id.into(),
                    generation: generation.into(),
                },
            },
        ),
    )
    .unwrap()
}

fn two_revision_history(connection: &mut VerifiedDatabaseConnection<'_>, session_id: &str) {
    let identity = native("native-history", "binding-history", "1", "1");
    create_session(connection, session_id, identity.clone());
    apply_session_lineage_command(
        connection,
        &command(
            "resume-history",
            SessionLineageOperation::Resume {
                session_id: session_id.into(),
                expected_revision: "1".into(),
                native: identity,
            },
        ),
    )
    .unwrap();
}

#[test]
fn every_historical_record_and_fingerprint_is_authoritative() {
    for axis in 0..4 {
        fixture(|connection| {
            two_revision_history(connection, "session-history");
            let sql = match axis {
                0 => "UPDATE main.gogoke_objects SET canonical_json=x'7b7d' WHERE object_type='SessionLineage' AND object_id='session-history' AND object_version='1'",
                1 => "UPDATE main.gogoke_events SET canonical_json=x'7b7d' WHERE object_type='SessionLineage' AND object_id='session-history' AND object_version='1'",
                2 => "UPDATE main.gogoke_receipts SET canonical_json=x'7b7d' WHERE object_type='SessionLineage' AND object_id='session-history' AND object_version='1'",
                _ => "UPDATE main.gogoke_receipts SET operation_fingerprint='sha256:0000000000000000000000000000000000000000000000000000000000000000' WHERE operation_id='resume-history'",
            };
            transaction::run(connection, |tx| tx.write(sql, &[])).unwrap();
            assert!(
                read_session_lineage(connection, "domain-one", "session-history").is_err(),
                "history corruption axis {axis} was accepted"
            );
        });
    }
}

#[test]
fn more_than_sixty_four_lineage_revisions_are_all_revalidated_after_reopen() {
    let _guard = route_b_test_guard();
    let path = scratch();
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    crate::store::atomic::apply_core_schema(&mut connection).unwrap();
    let _owner = initialize_profile(&mut connection, &root).unwrap();
    initialize_session_lineage_schema(&mut connection).unwrap();
    let identity = native("native-many", "binding-many", "1", "1");
    create_session(&mut connection, "session-many", identity.clone());
    for revision in 1..=70 {
        apply_session_lineage_command(
            &mut connection,
            &command(
                &format!("resume-many-{revision}"),
                SessionLineageOperation::Resume {
                    session_id: "session-many".into(),
                    expected_revision: revision.to_string(),
                    native: identity.clone(),
                },
            ),
        )
        .unwrap();
    }
    connection.close_checked().unwrap();
    let mut connection =
        crate::store::same_open::open_existing(&root, &path.join("state.sqlite")).unwrap();
    initialize_session_lineage_schema(&mut connection).unwrap();
    assert_eq!(
        read_session_lineage(&mut connection, "domain-one", "session-many")
            .unwrap()
            .revision,
        "71"
    );
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

#[test]
fn core_table_triggers_cannot_commit_lineage_corruption() {
    for temporary in [false, true] {
        fixture(|connection| {
            create_session(
                connection,
                "session-trigger",
                native("native-trigger", "binding-trigger", "1", "1"),
            );
            let sql = if temporary {
                "CREATE TEMP TRIGGER lineage_corrupt_history AFTER INSERT ON main.gogoke_objects WHEN NEW.object_type='SessionLineage' AND NEW.object_version='2' BEGIN UPDATE gogoke_objects SET canonical_json=x'7b7d' WHERE object_type='SessionLineage' AND object_id=NEW.object_id AND object_version='1'; END"
            } else {
                "CREATE TRIGGER lineage_corrupt_history AFTER INSERT ON gogoke_objects WHEN NEW.object_type='SessionLineage' AND NEW.object_version='2' BEGIN UPDATE gogoke_objects SET canonical_json=x'7b7d' WHERE object_type='SessionLineage' AND object_id=NEW.object_id AND object_version='1'; END"
            };
            transaction::run(connection, |tx| tx.write(sql, &[])).unwrap();
            let result = apply_session_lineage_command(
                connection,
                &command(
                    "resume-trigger",
                    SessionLineageOperation::Resume {
                        session_id: "session-trigger".into(),
                        expected_revision: "1".into(),
                        native: native("native-trigger", "binding-trigger", "1", "1"),
                    },
                ),
            );
            assert!(result.is_err(), "trigger corruption was committed");
        });
    }
}

#[test]
fn main_core_views_cannot_impersonate_authoritative_tables_on_read() {
    for table in ["gogoke_events", "gogoke_receipts", "gogoke_stream_heads"] {
        fixture(|connection| {
            create_session(
                connection,
                "session-core-view",
                native("native-core-view", "binding-core-view", "1", "1"),
            );
            let renamed = format!("{table}_replaced");
            transaction::run(connection, |tx| {
                tx.write(
                    &format!("ALTER TABLE main.{table} RENAME TO {renamed}"),
                    &[],
                )?;
                tx.write(
                    &format!("CREATE VIEW main.{table} AS SELECT * FROM main.{renamed}"),
                    &[],
                )
            })
            .unwrap();
            assert!(
                read_session_lineage(connection, "domain-one", "session-core-view").is_err(),
                "MAIN view impersonated {table}"
            );
        });
    }
}

#[test]
fn exposure_history_is_resolvable_and_clean_append_cannot_clear_taint() {
    fixture(|connection| {
        create_session(
            connection,
            "session-exposure-history",
            native(
                "native-exposure-history",
                "binding-exposure-history",
                "1",
                "1",
            ),
        );
        apply_session_lineage_command(
            connection,
            &command(
                "append-tainted",
                SessionLineageOperation::AppendExposureReceipt {
                    session_id: "session-exposure-history".into(),
                    expected_revision: "1".into(),
                    exposure: exposure(
                        "receipt-tainted",
                        "binding-exposure-history",
                        "1",
                        &["private"],
                    ),
                },
            ),
        )
        .unwrap();
        let stored = read_exposure_receipt(connection, "domain-one", "receipt-tainted").unwrap();
        assert_eq!(stored.session_id, "session-exposure-history");
        assert_eq!(stored.session_revision, "2");
        assert_eq!(stored.exposure.taint_labels, vec!["private"]);

        let clean = apply_session_lineage_command(
            connection,
            &command(
                "append-clean",
                SessionLineageOperation::AppendExposureReceipt {
                    session_id: "session-exposure-history".into(),
                    expected_revision: "2".into(),
                    exposure: exposure("receipt-clean", "binding-exposure-history", "1", &[]),
                },
            ),
        )
        .unwrap();
        assert!(clean
            .snapshot
            .lineage
            .inherited_exposure
            .taint_labels
            .contains(&"private".into()));
        assert!(clean
            .snapshot
            .lineage
            .inherited_exposure
            .source_receipt_refs
            .contains(&"receipt-tainted".into()));
        assert_eq!(clean.snapshot.exposure_assessment.classification, "TAINTED");
        assert_eq!(
            read_exposure_receipt(connection, "domain-one", "receipt-tainted")
                .unwrap()
                .exposure
                .receipt_id,
            "receipt-tainted"
        );
    });
}

#[test]
fn exposure_receipt_remains_resolvable_when_later_snapshots_carry_it() {
    fixture(|connection| {
        let identity = native("native-carry", "binding-carry", "1", "1");
        create_session(connection, "session-carry", identity.clone());
        apply_session_lineage_command(
            connection,
            &command(
                "append-carry",
                SessionLineageOperation::AppendExposureReceipt {
                    session_id: "session-carry".into(),
                    expected_revision: "1".into(),
                    exposure: exposure("receipt-carry", "binding-carry", "1", &["private"]),
                },
            ),
        )
        .unwrap();
        apply_session_lineage_command(
            connection,
            &command(
                "resume-carry",
                SessionLineageOperation::Resume {
                    session_id: "session-carry".into(),
                    expected_revision: "2".into(),
                    native: identity,
                },
            ),
        )
        .unwrap();
        retain_action(
            connection,
            "session-carry",
            "3",
            "retain-carry",
            "binding-carry",
            "1",
        );
        apply_session_lineage_command(
            connection,
            &command(
                "archive-carry",
                SessionLineageOperation::Archive {
                    session_id: "session-carry".into(),
                    expected_revision: "4".into(),
                },
            ),
        )
        .unwrap();
        let stored = read_exposure_receipt(connection, "domain-one", "receipt-carry").unwrap();
        assert_eq!(stored.session_revision, "2");
        assert_eq!(stored.exposure.taint_labels, vec!["private"]);
    });
}

#[test]
fn new_clean_is_durable_replay_safe_and_has_no_parent_exposure_or_pending_action() {
    fixture(|connection| {
        let request = command(
            "op-new-clean",
            SessionLineageOperation::NewClean {
                session_id: "session-clean".into(),
                native: native("native-clean", "binding-clean", "1", "0"),
            },
        );
        let created = apply_session_lineage_command(connection, &request).unwrap();
        assert_eq!(created.disposition, "COMMITTED");
        assert_eq!(
            created.authority_status,
            "PREPARATORY_TRUSTED_INGRESS_REQUIRED"
        );
        assert_eq!(created.snapshot.lineage.operation_kind, "NEW_CLEAN");
        assert!(created.snapshot.lineage.parent_refs.is_empty());
        assert!(created.snapshot.exposure.is_none());
        assert!(created.snapshot.pending_actions.is_empty());
        assert_eq!(created.snapshot.lifecycle, "ACTIVE");
        assert_eq!(created.snapshot.process_state, "RUNNING");

        let replay = apply_session_lineage_command(connection, &request).unwrap();
        assert_eq!(replay.disposition, "REPLAYED");
        assert_eq!(replay.snapshot, created.snapshot);
        assert_eq!(
            read_session_lineage(connection, "domain-one", "session-clean").unwrap(),
            created.snapshot
        );
        transaction::run(connection, |tx| {
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_session_lineage_heads", &[], 1)?[0][0], "1");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_objects WHERE object_type='SessionLineage'", &[], 1)?[0][0], "1");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_events WHERE event_type='SessionLineageCommitted'", &[], 1)?[0][0], "1");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_receipts WHERE receipt_type='SessionLineageCommitted'", &[], 1)?[0][0], "1");
            Ok(())
        }).unwrap();
    });
}

#[test]
fn resume_requires_exact_native_tuple_and_retains_pending_action_without_replay() {
    fixture(|connection| {
        create_session(
            connection,
            "session-resume",
            native("native-resume", "binding-resume", "4", "12"),
        );
        retain_action(
            connection,
            "session-resume",
            "1",
            "op-retain-one",
            "binding-resume",
            "4",
        );
        let resume = command(
            "op-resume",
            SessionLineageOperation::Resume {
                session_id: "session-resume".into(),
                expected_revision: "2".into(),
                native: native("native-resume", "binding-resume", "4", "12"),
            },
        );
        let resumed = apply_session_lineage_command(connection, &resume).unwrap();
        assert_eq!(resumed.snapshot.lineage.operation_kind, "RESUME");
        assert_eq!(
            resumed.snapshot.pending_action_disposition,
            "RETAINED_NOT_REPLAYED"
        );
        assert_eq!(resumed.snapshot.pending_actions.len(), 1);
        assert_eq!(
            resumed.snapshot.pending_actions[0].action_id,
            "action-op-retain-one"
        );

        let wrong = command(
            "op-resume-wrong-epoch",
            SessionLineageOperation::Resume {
                session_id: "session-resume".into(),
                expected_revision: "3".into(),
                native: native("native-resume", "binding-resume", "4", "13"),
            },
        );
        assert!(apply_session_lineage_command(connection, &wrong).is_err());
        assert_eq!(
            read_session_lineage(connection, "domain-one", "session-resume").unwrap(),
            resumed.snapshot
        );
    });
}

#[test]
fn fork_rebuild_and_handoff_keep_pending_actions_on_parent_and_propagate_exposure() {
    fixture(|connection| {
        create_session(
            connection,
            "session-parent",
            native("native-parent", "binding-parent", "7", "2"),
        );
        let exposure_cmd = command(
            "op-exposure-parent",
            SessionLineageOperation::AppendExposureReceipt {
                session_id: "session-parent".into(),
                expected_revision: "1".into(),
                exposure: exposure("exposure-parent", "binding-parent", "7", &["taint-source"]),
            },
        );
        apply_session_lineage_command(connection, &exposure_cmd).unwrap();
        retain_action(
            connection,
            "session-parent",
            "2",
            "op-parent-pending",
            "binding-parent",
            "7",
        );

        let fork = apply_session_lineage_command(
            connection,
            &command(
                "op-fork",
                SessionLineageOperation::NativeFork {
                    session_id: "session-fork".into(),
                    parent_session_id: "session-parent".into(),
                    expected_parent_revision: "3".into(),
                    native: native("native-fork", "binding-fork", "1", "3"),
                },
            ),
        )
        .unwrap();
        assert_eq!(fork.snapshot.lineage.parent_refs, vec!["session-parent"]);
        assert_eq!(fork.snapshot.lineage.operation_kind, "NATIVE_FORK");
        assert_eq!(fork.snapshot.exposure_assessment.classification, "TAINTED");
        assert!(fork
            .snapshot
            .lineage
            .inherited_exposure
            .taint_labels
            .contains(&"taint-source".into()));
        assert_eq!(
            fork.snapshot.pending_action_disposition,
            "RETAINED_ON_PARENT"
        );
        assert!(fork.snapshot.pending_actions.is_empty());

        let rebuild = apply_session_lineage_command(
            connection,
            &command(
                "op-rebuild",
                SessionLineageOperation::Rebuild {
                    session_id: "session-rebuild".into(),
                    parent_session_id: "session-parent".into(),
                    expected_parent_revision: "3".into(),
                    native: native("native-rebuild", "binding-rebuild", "2", "4"),
                },
            ),
        )
        .unwrap();
        assert_eq!(rebuild.snapshot.lineage.operation_kind, "REBUILD");
        assert_eq!(
            rebuild.snapshot.exposure_assessment.classification,
            "TAINTED"
        );
        assert!(rebuild.snapshot.pending_actions.is_empty());

        let handoff = apply_session_lineage_command(
            connection,
            &command(
                "op-handoff",
                SessionLineageOperation::Handoff {
                    session_id: "session-handoff".into(),
                    parent_session_id: "session-parent".into(),
                    expected_parent_revision: "3".into(),
                    native: native("native-handoff", "binding-handoff", "1", "5"),
                    material_ids: vec!["material-one".into()],
                },
            ),
        )
        .unwrap();
        assert_eq!(
            handoff
                .snapshot
                .material_handoff
                .as_ref()
                .unwrap()
                .native_resume_used,
            false
        );
        assert_eq!(
            handoff
                .snapshot
                .material_handoff
                .as_ref()
                .unwrap()
                .source_session_id,
            "session-parent"
        );
        assert!(handoff.snapshot.pending_actions.is_empty());
        assert_eq!(
            read_session_lineage(connection, "domain-one", "session-parent")
                .unwrap()
                .pending_actions
                .len(),
            1
        );
    });
}

#[test]
fn archive_does_not_change_process_state_or_clear_pending_action() {
    fixture(|connection| {
        create_session(
            connection,
            "session-archive",
            native("native-archive", "binding-archive", "1", "1"),
        );
        retain_action(
            connection,
            "session-archive",
            "1",
            "op-archive-pending",
            "binding-archive",
            "1",
        );
        let archived = apply_session_lineage_command(
            connection,
            &command(
                "op-archive",
                SessionLineageOperation::Archive {
                    session_id: "session-archive".into(),
                    expected_revision: "2".into(),
                },
            ),
        )
        .unwrap();
        assert_eq!(archived.snapshot.lifecycle, "ARCHIVED");
        assert_eq!(archived.snapshot.process_state, "RUNNING");
        assert_eq!(archived.snapshot.pending_actions.len(), 1);
        assert_eq!(
            archived.snapshot.pending_actions[0].action_id,
            "action-op-archive-pending"
        );
    });
}

#[test]
fn exposure_requires_current_binding_and_replay_never_erases_unknown_acceptance() {
    fixture(|connection| {
        create_session(
            connection,
            "session-exposure",
            native("native-exposure", "binding-exposure", "2", "9"),
        );
        let mut bad = exposure("exposure-bad", "other-binding", "2", &[]);
        bad.native_source_coverage.complete = false;
        let invalid = command(
            "op-exposure-invalid",
            SessionLineageOperation::AppendExposureReceipt {
                session_id: "session-exposure".into(),
                expected_revision: "1".into(),
                exposure: bad,
            },
        );
        assert!(apply_session_lineage_command(connection, &invalid).is_err());
        assert_eq!(
            read_session_lineage(connection, "domain-one", "session-exposure")
                .unwrap()
                .revision,
            "1"
        );

        let receipt = exposure("exposure-unknown", "binding-exposure", "2", &[]);
        let commit = command(
            "op-exposure-unknown",
            SessionLineageOperation::AppendExposureReceipt {
                session_id: "session-exposure".into(),
                expected_revision: "1".into(),
                exposure: receipt,
            },
        );
        let first = apply_session_lineage_command(connection, &commit).unwrap();
        assert_eq!(first.snapshot.exposure_assessment.classification, "CLEAN");
        let replay = apply_session_lineage_command(connection, &commit).unwrap();
        assert_eq!(replay.disposition, "REPLAYED");
        assert_eq!(replay.snapshot, first.snapshot);
        assert_eq!(
            read_session_lineage(connection, "domain-one", "session-exposure").unwrap(),
            first.snapshot
        );
    });
}

#[test]
fn unknown_exposure_stays_unknown_across_multiple_native_children() {
    fixture(|connection| {
        create_session(
            connection,
            "session-unknown-parent",
            native("native-unknown-parent", "binding-unknown-parent", "1", "1"),
        );
        let mut unobserved = exposure(
            "exposure-host-delivered",
            "binding-unknown-parent",
            "1",
            &[],
        );
        unobserved.evidence_level = "HOST_DELIVERED".into();
        let mut coverage = unobserved.native_source_coverage.clone();
        coverage.complete = false;
        coverage.observations[0].status = "NOT_OBSERVED".into();
        unobserved.native_source_coverage = coverage;
        apply_session_lineage_command(
            connection,
            &command(
                "op-unknown-exposure",
                SessionLineageOperation::AppendExposureReceipt {
                    session_id: "session-unknown-parent".into(),
                    expected_revision: "1".into(),
                    exposure: unobserved,
                },
            ),
        )
        .unwrap();

        let first_child = apply_session_lineage_command(
            connection,
            &command(
                "op-unknown-fork-one",
                SessionLineageOperation::NativeFork {
                    session_id: "session-unknown-child-one".into(),
                    parent_session_id: "session-unknown-parent".into(),
                    expected_parent_revision: "2".into(),
                    native: native(
                        "native-unknown-child-one",
                        "binding-unknown-child-one",
                        "1",
                        "2",
                    ),
                },
            ),
        )
        .unwrap();
        assert_eq!(
            first_child.snapshot.exposure_assessment.classification,
            "UNKNOWN"
        );
        assert!(first_child
            .snapshot
            .lineage
            .inherited_exposure
            .unknown_sources
            .contains(&"manifest-source-one".into()));

        let second_child = apply_session_lineage_command(
            connection,
            &command(
                "op-unknown-rebuild-two",
                SessionLineageOperation::Rebuild {
                    session_id: "session-unknown-child-two".into(),
                    parent_session_id: "session-unknown-child-one".into(),
                    expected_parent_revision: "1".into(),
                    native: native(
                        "native-unknown-child-two",
                        "binding-unknown-child-two",
                        "1",
                        "3",
                    ),
                },
            ),
        )
        .unwrap();
        assert_eq!(
            second_child.snapshot.exposure_assessment.classification,
            "UNKNOWN"
        );
        assert!(second_child
            .snapshot
            .lineage
            .inherited_exposure
            .evidence_levels
            .contains(&"UNKNOWN".into()));
    });
}

#[test]
fn failure_after_domain_record_attempt_rolls_back_objects_events_receipts_and_head() {
    fixture(|connection| {
        create_session(
            connection,
            "session-atomic-first",
            native("native-atomic-first", "binding-atomic-first", "1", "1"),
        );
        let mut conflicting_event = command(
            "op-atomic-second",
            SessionLineageOperation::NewClean {
                session_id: "session-atomic-second".into(),
                native: native("native-atomic-second", "binding-atomic-second", "1", "2"),
            },
        );
        conflicting_event.event_id = "event-create-session-atomic-first".into();
        assert!(apply_session_lineage_command(connection, &conflicting_event).is_err());
        transaction::run(connection, |tx| {
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_session_lineage_heads WHERE session_id='session-atomic-second'", &[], 1)?[0][0], "0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_objects WHERE object_type='SessionLineage' AND object_id='session-atomic-second'", &[], 1)?[0][0], "0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_events WHERE event_id='event-create-session-atomic-first'", &[], 1)?[0][0], "1");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_receipts WHERE operation_id='op-atomic-second'", &[], 1)?[0][0], "0");
            assert_eq!(tx.query("SELECT count(*) FROM main.gogoke_stream_heads WHERE stream_id='gogoke.session-lineage.v1/session-atomic-second'", &[], 1)?[0][0], "0");
            Ok(())
        })
        .unwrap();
    });
}
