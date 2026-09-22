use super::super::{initialize_profile, OwnerIssuer};
use super::{
    append_trusted_task_material, read_trusted_task_material, AppendTaskMaterial,
    MaterialVisibility, TaskMaterial,
};
use crate::root::RootLock;
use crate::store::atomic::{count_table, initialize_product_core_schema};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{route_b_test_guard, VerifiedDatabaseConnection};
use crate::store::session::open_product_database;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-task-material-{nonce}"));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = open_product_database(&root, &database).unwrap();
    initialize_product_core_schema(&mut db).unwrap();
    let owner = initialize_profile(&mut db, &root).unwrap();
    run(&mut db, &owner);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(&database).ok();
    std::fs::remove_file(format!("{}-wal", database.display())).ok();
    std::fs::remove_file(format!("{}-shm", database.display())).ok();
    std::fs::remove_dir(path).ok();
}

fn input(
    owner: &OwnerIssuer,
    operation: &str,
    previous: Option<&str>,
    content: &str,
) -> AppendTaskMaterial {
    AppendTaskMaterial {
        operation_id: operation.into(),
        expected_previous_revision: previous.map(str::to_owned),
        material: TaskMaterial {
            material_id: "material-one".into(),
            project_id: "project-one".into(),
            domain_id: "domain-one".into(),
            owner_principal_id: owner.principal_id().into(),
            material_class: "task-context".into(),
            visibility: MaterialVisibility::Private,
            content: content.into(),
        },
        provenance_ref: "evidence:task-source-1".into(),
        event_id: format!("event-{operation}"),
        receipt_id: format!("receipt-{operation}"),
        recorded_at: "2026-09-22T00:00:00Z".into(),
    }
}

#[test]
fn owner_append_revise_replay_and_read_preserve_unicode_nul_and_provenance() {
    fixture(|db, owner| {
        let first = input(owner, "create", None, "雪\u{0000}and🌙\u{0000}end");
        let receipt = append_trusted_task_material(db, owner, &first).unwrap();
        assert_eq!(receipt.disposition, "COMMITTED");
        assert_eq!(
            receipt.authority_status,
            "PREPARATORY_TRUSTED_INGRESS_REQUIRED"
        );
        assert_eq!(receipt.current.revision, "1");
        assert_eq!(
            receipt.current.material.content,
            "雪\u{0000}and🌙\u{0000}end"
        );
        assert_eq!(receipt.current.provenance_ref, "evidence:task-source-1");
        assert!(receipt.current.content_hash.starts_with("sha256:"));
        assert_eq!(
            append_trusted_task_material(db, owner, &first)
                .unwrap()
                .disposition,
            "RECONCILED"
        );

        let second = input(owner, "revise", Some("1"), "revised\u{0000}內容");
        assert_eq!(
            append_trusted_task_material(db, owner, &second)
                .unwrap()
                .current
                .revision,
            "2"
        );
        let read = read_trusted_task_material(db, owner, "domain-one", "material-one").unwrap();
        assert_eq!(read.material.content, "revised\u{0000}內容");
        assert_eq!(read.revision, "2");
        assert_eq!(count_table(db, "gogoke_objects").unwrap(), 2);
        assert_eq!(count_table(db, "gogoke_events").unwrap(), 2);
        assert_eq!(count_table(db, "gogoke_receipts").unwrap(), 2);
    });
}

#[test]
fn conflicting_replay_stale_revision_and_wrong_owner_fail_closed() {
    fixture(|db, owner| {
        let first = input(owner, "create", None, "original");
        append_trusted_task_material(db, owner, &first).unwrap();
        let mut changed_replay = first.clone();
        changed_replay.material.content = "changed".into();
        assert!(matches!(
            append_trusted_task_material(db, owner, &changed_replay),
            Err(OrchestrationError::OperationConflict)
        ));
        assert!(matches!(
            append_trusted_task_material(db, owner, &input(owner, "stale", Some("9"), "stale")),
            Err(OrchestrationError::OperationConflict)
        ));
        let mut wrong_owner = input(owner, "wrong-owner", Some("1"), "forged");
        wrong_owner.material.owner_principal_id = "another-principal".into();
        assert!(append_trusted_task_material(db, owner, &wrong_owner).is_err());
        let mut invalid_identity = input(owner, "unicode-id", Some("1"), "opaque");
        invalid_identity.material.material_id = "雪-material".into();
        assert!(append_trusted_task_material(db, owner, &invalid_identity).is_err());
        assert_eq!(
            read_trusted_task_material(db, owner, "domain-one", "material-one")
                .unwrap()
                .revision,
            "1"
        );
        assert_eq!(count_table(db, "gogoke_objects").unwrap(), 1);
        assert_eq!(count_table(db, "gogoke_events").unwrap(), 1);
        assert_eq!(count_table(db, "gogoke_receipts").unwrap(), 1);
    });
}

#[test]
fn object_event_and_receipt_corruption_are_rejected_on_read() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "create", None, "checked")).unwrap();
        db.execute(
            "UPDATE gogoke_events SET event_type='Forged' WHERE event_type='TaskMaterialAppended'",
        )
        .unwrap();
        assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
    });

    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "create", None, "checked")).unwrap();
        db.execute("UPDATE gogoke_receipts SET receipt_type='Forged' WHERE receipt_type='TaskMaterialAppended'").unwrap();
        assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
    });

    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "create", None, "checked")).unwrap();
        db.execute("UPDATE gogoke_objects SET content_hash='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE object_type='TaskMaterial'").unwrap();
        assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
    });
}

// Fresh-review regression probes, transferred from the read-only audit custody.
#[test]
fn audit_trigger_cannot_corrupt_prior_revision_while_append_reports_committed() {
    audit_trigger_case("");
}

#[test]
fn audit_temp_trigger_cannot_corrupt_prior_revision_while_append_reports_committed() {
    audit_trigger_case("TEMP ");
}

fn audit_trigger_case(temp: &str) {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "original")).unwrap();
        db.execute(&format!("CREATE {temp}TRIGGER audit_material_hook AFTER INSERT ON main.gogoke_receipts BEGIN UPDATE gogoke_objects SET canonical_json=X'7b7d' WHERE object_type='TaskMaterial' AND object_version='1'; END")).unwrap();
        let result =
            append_trusted_task_material(db, owner, &input(owner, "second", Some("1"), "new"));
        assert!(
            result.is_err(),
            "trigger changed immutable old material while append returned {result:?}"
        );
    });
}

#[test]
fn audit_corrupt_historical_object_is_rejected() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "old")).unwrap();
        append_trusted_task_material(db, owner, &input(owner, "second", Some("1"), "new")).unwrap();
        db.execute("UPDATE main.gogoke_objects SET canonical_json=X'7b7d' WHERE object_type='TaskMaterial' AND object_version='1'").unwrap();
        assert!(
            read_trusted_task_material(db, owner, "domain-one", "material-one").is_err(),
            "corrupt historical material was ignored"
        );
    });
}

#[test]
fn audit_corrupt_historical_event_and_receipt_are_rejected() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "old")).unwrap();
        append_trusted_task_material(db, owner, &input(owner, "second", Some("1"), "new")).unwrap();
        db.execute("UPDATE main.gogoke_events SET event_type='Forged' WHERE object_type='TaskMaterial' AND object_version='1'").unwrap();
        assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
    });

    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "old")).unwrap();
        append_trusted_task_material(db, owner, &input(owner, "second", Some("1"), "new")).unwrap();
        db.execute("UPDATE main.gogoke_receipts SET receipt_type='Forged' WHERE object_type='TaskMaterial' AND object_version='1'").unwrap();
        assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
    });
}

#[test]
fn audit_historical_previous_revision_link_is_checked() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "old")).unwrap();
        append_trusted_task_material(db, owner, &input(owner, "second", Some("1"), "new")).unwrap();
        super::transaction::run(db, |tx| {
            let rows = tx.query("SELECT event_id,CAST(canonical_json AS TEXT) FROM main.gogoke_events WHERE object_type='TaskMaterial' AND object_version='2'", &[], 2)?;
            let changed = rows[0][1].replace("\"expectedPreviousRevision\":\"1\"", "\"expectedPreviousRevision\":\"0\"");
            let hash = super::content_hash(changed.as_bytes());
            tx.write("UPDATE main.gogoke_events SET canonical_json=CAST(? AS BLOB),content_hash=? WHERE event_id=?", &[&changed,&hash,&rows[0][0]])
        }).unwrap();
        assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
    });
}

#[test]
fn audit_missing_historical_revision_is_rejected_even_when_latest_remains() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "old")).unwrap();
        append_trusted_task_material(db, owner, &input(owner, "second", Some("1"), "new")).unwrap();
        db.execute("DELETE FROM main.gogoke_receipts WHERE object_type='TaskMaterial' AND object_version='1'").unwrap();
        db.execute("DELETE FROM main.gogoke_events WHERE object_type='TaskMaterial' AND object_version='1'").unwrap();
        db.execute("DELETE FROM main.gogoke_objects WHERE object_type='TaskMaterial' AND object_version='1'").unwrap();
        assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
    });
}

#[test]
fn audit_receipt_fingerprint_must_match_facts() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "old")).unwrap();
        db.execute("UPDATE main.gogoke_receipts SET operation_fingerprint='sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE object_type='TaskMaterial'").unwrap();
        assert!(
            read_trusted_task_material(db, owner, "domain-one", "material-one").is_err(),
            "arbitrary well-shaped fingerprint accepted"
        );
    });
}

#[test]
fn audit_noncanonical_object_bytes_must_be_rejected_even_with_matching_hash() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "opaque")).unwrap();
        super::transaction::run(db, |tx| {
            let rows = tx.query("SELECT CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE object_type='TaskMaterial'", &[], 1)?;
            let modified = rows[0][0].replacen("{", "{ ", 1);
            let hash = super::content_hash(modified.as_bytes());
            tx.write("UPDATE main.gogoke_objects SET canonical_json=CAST(? AS BLOB),content_hash=? WHERE object_type='TaskMaterial'", &[&modified,&hash])
        }).unwrap();
        assert!(
            read_trusted_task_material(db, owner, "domain-one", "material-one").is_err(),
            "noncanonical bytes accepted as canonical material"
        );
    });
}

#[test]
fn audit_shadow_cannot_be_used_as_authoritative_storage() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "original")).unwrap();
        db.execute(
            "CREATE TEMP TABLE gogoke_receipts AS SELECT * FROM main.gogoke_receipts WHERE 0",
        )
        .unwrap();
        assert!(
            append_trusted_task_material(db, owner, &input(owner, "second", Some("1"), "new"))
                .is_err()
        );
        db.execute("DROP TABLE temp.gogoke_receipts").unwrap();
        assert_eq!(
            read_trusted_task_material(db, owner, "domain-one", "material-one")
                .unwrap()
                .revision,
            "1"
        );
        assert_eq!(count_table(db, "gogoke_objects").unwrap(), 1);
        assert_eq!(count_table(db, "gogoke_events").unwrap(), 1);
        assert_eq!(count_table(db, "gogoke_receipts").unwrap(), 1);
    });
}

#[test]
fn audit_profile_identity_and_scope_are_bound() {
    fixture(|db, owner| {
        let first = input(owner, "first", None, "original");
        append_trusted_task_material(db, owner, &first).unwrap();
        assert!(read_trusted_task_material(db, owner, "other-domain", "material-one").is_err());
        assert!(read_trusted_task_material(db, owner, "domain-one", "other-material").is_err());
        for field in [
            "profile_id",
            "root_identity",
            "owner_principal_id",
            "owner_seat_id",
            "issuer_id",
        ] {
            let original = super::transaction::run(db, |tx| {
                tx.query(
                    &format!("SELECT {field} FROM main.gogoke_authority_profile"),
                    &[],
                    1,
                )
            })
            .unwrap()[0][0]
                .clone();
            db.execute(&format!(
                "UPDATE main.gogoke_authority_profile SET {field}='wrong-identity'"
            ))
            .unwrap();
            assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
            super::transaction::run(db, |tx| {
                tx.write(
                    &format!("UPDATE main.gogoke_authority_profile SET {field}=?"),
                    &[&original],
                )
            })
            .unwrap();
            assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_ok());
        }
    });
}

#[test]
fn audit_cas_replay_conflicts_do_not_change_storage() {
    fixture(|db, owner| {
        let first = input(owner, "first", None, "initial");
        append_trusted_task_material(db, owner, &first).unwrap();
        for axis in 0..6 {
            let mut changed = first.clone();
            match axis {
                0 => changed.recorded_at = "2026-09-23T00:00:00Z".into(),
                1 => changed.provenance_ref = "other-evidence".into(),
                2 => changed.event_id = "other-event".into(),
                3 => changed.receipt_id = "other-receipt".into(),
                4 => changed.material.project_id = "other-project".into(),
                _ => changed.material.visibility = MaterialVisibility::Project,
            }
            assert!(
                append_trusted_task_material(db, owner, &changed).is_err(),
                "conflicting replay axis {axis}"
            );
        }
        let mut second = input(owner, "second", Some("1"), "next");
        second.receipt_id = first.receipt_id.clone();
        assert!(append_trusted_task_material(db, owner, &second).is_err());
        assert!(
            append_trusted_task_material(db, owner, &input(owner, "bad-zero", Some("0"), "x"))
                .is_err()
        );
        assert_eq!(
            read_trusted_task_material(db, owner, "domain-one", "material-one")
                .unwrap()
                .revision,
            "1"
        );
        for table in ["gogoke_objects", "gogoke_events", "gogoke_receipts"] {
            assert_eq!(count_table(db, table).unwrap(), 1);
        }
    });
}

#[test]
fn audit_read_and_append_more_than_64_material_revisions_and_large_content() {
    fixture(|db, owner| {
        let large = format!("{}\0雪🌙", "é\n\\\"".repeat(65536));
        for n in 0..70 {
            let mut value = input(
                owner,
                &format!("create-{n}"),
                None,
                if n == 0 { &large } else { "small" },
            );
            value.material.material_id = format!("material-{n}");
            append_trusted_task_material(db, owner, &value).unwrap();
        }
        assert_eq!(
            read_trusted_task_material(db, owner, "domain-one", "material-0")
                .unwrap()
                .material
                .content,
            large
        );
        append_trusted_task_material(db, owner, &input(owner, "revision-1", None, "v1")).unwrap();
        for n in 2..=70 {
            append_trusted_task_material(
                db,
                owner,
                &input(
                    owner,
                    &format!("revision-{n}"),
                    Some(&(n - 1).to_string()),
                    &format!("v{n}"),
                ),
            )
            .unwrap();
        }
        assert_eq!(
            read_trusted_task_material(db, owner, "domain-one", "material-one")
                .unwrap()
                .revision,
            "70"
        );
    });
}

#[test]
fn audit_all_content_controls_and_identity_fields() {
    fixture(|db, owner| {
        let content = (0u32..32)
            .map(|n| char::from_u32(n).unwrap())
            .collect::<String>()
            + "雪🌙e\u{0301}é\u{2028}\u{2029}";
        let value = input(owner, "controls", None, &content);
        assert_eq!(
            append_trusted_task_material(db, owner, &value)
                .unwrap()
                .current
                .material
                .content,
            content
        );
        for axis in 0..6 {
            let mut bad = input(owner, &format!("bad-{axis}"), Some("1"), "bad");
            match axis {
                0 => bad.material.material_id = "bad\0id".into(),
                1 => bad.material.domain_id = "bad\0domain".into(),
                2 => bad.material.project_id = "bad project".into(),
                3 => bad.material.owner_principal_id = "bad-owner".into(),
                4 => bad.material.material_class = "bad\0class".into(),
                _ => bad.provenance_ref = "bad\0source".into(),
            }
            assert!(
                append_trusted_task_material(db, owner, &bad).is_err(),
                "identity axis {axis}"
            );
        }
    });
}

#[test]
fn audit_reopen_preserves_material_revisions_and_replay() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-material-audit-reopen-{nonce}"));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = open_product_database(&root, &database).unwrap();
    initialize_product_core_schema(&mut db).unwrap();
    let owner = initialize_profile(&mut db, &root).unwrap();
    let first = input(&owner, "reopen-first", None, "before\0雪");
    append_trusted_task_material(&mut db, &owner, &first).unwrap();
    let second = input(&owner, "reopen-second", Some("1"), "after\0🌙");
    let expected = append_trusted_task_material(&mut db, &owner, &second).unwrap();
    db.close_checked().unwrap();
    let mut reopened = open_product_database(&root, &database).unwrap();
    let reopened_owner = initialize_profile(&mut reopened, &root).unwrap();
    let actual =
        read_trusted_task_material(&mut reopened, &reopened_owner, "domain-one", "material-one")
            .unwrap();
    assert_eq!(actual, expected.current);
    assert_eq!(
        append_trusted_task_material(&mut reopened, &reopened_owner, &second)
            .unwrap()
            .disposition,
        "RECONCILED"
    );
    for table in ["gogoke_objects", "gogoke_events", "gogoke_receipts"] {
        assert_eq!(count_table(&mut reopened, table).unwrap(), 2);
    }
    reopened.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(&database).ok();
    std::fs::remove_file(format!("{}-wal", database.display())).ok();
    std::fs::remove_file(format!("{}-shm", database.display())).ok();
    std::fs::remove_dir(path).ok();
}

#[test]
fn audit_wrong_root_owner_capability_rejected() {
    fixture(|db, owner| {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-material-audit-other-root-{nonce}"));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut other = open_product_database(&root, &database).unwrap();
        initialize_product_core_schema(&mut other).unwrap();
        let other_owner = initialize_profile(&mut other, &root).unwrap();
        append_trusted_task_material(db, owner, &input(owner, "first", None, "original")).unwrap();
        assert!(
            read_trusted_task_material(db, &other_owner, "domain-one", "material-one").is_err()
        );
        assert!(append_trusted_task_material(
            db,
            &other_owner,
            &input(&other_owner, "forged", Some("1"), "forged")
        )
        .is_err());
        other.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(&database).ok();
        std::fs::remove_file(format!("{}-wal", database.display())).ok();
        std::fs::remove_file(format!("{}-shm", database.display())).ok();
        std::fs::remove_dir(path).ok();
    });
}

#[test]
fn audit_main_table_shadow_fails_closed() {
    fixture(|db, owner| {
        append_trusted_task_material(db, owner, &input(owner, "first", None, "original")).unwrap();
        db.execute("ALTER TABLE main.gogoke_receipts RENAME TO gogoke_receipts_saved")
            .unwrap();
        db.execute("CREATE VIEW main.gogoke_receipts AS SELECT * FROM gogoke_receipts_saved")
            .unwrap();
        assert!(read_trusted_task_material(db, owner, "domain-one", "material-one").is_err());
        assert!(
            append_trusted_task_material(db, owner, &input(owner, "second", Some("1"), "new"))
                .is_err()
        );
    });
}
