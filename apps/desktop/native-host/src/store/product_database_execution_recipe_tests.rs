//! Product Database tests for durable ExecutionRecipe versions and non-claims.
use super::*;
use crate::root::RootLock;
use crate::store::atomic::Statement;
use crate::store::authority::{
    AppendExecutionRecipe, RecipeJsonString, RecipeJsonValue, CURRENTNESS_STATUS,
};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::route_b_test_guard;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-execution-recipe-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&path).expect("private test root");
    path
}

fn cleanup(path: &Path, database: &Path) {
    let _ = std::fs::remove_file(database);
    let _ = std::fs::remove_file(database.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(database.with_extension("sqlite-shm"));
    if let Err(error) = std::fs::remove_dir(path) {
        eprintln!("owned recipe test root retained: {error}");
    }
}

fn fixture(run: impl FnOnce(&RootLock, &Path, &mut ProductDatabase<'_>)) {
    let _guard = route_b_test_guard();
    let path = temp_root("single");
    let root = RootLock::acquire(&path).expect("root lock");
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).expect("product database");
    run(&root, &database, &mut product);
    product.close_checked().expect("close");
    drop(root);
    cleanup(&path, &database);
}

fn scalar(product: &ProductDatabase<'_>, sql: &str) -> String {
    let statement = Statement::prepare(product.connection.as_ptr(), sql).expect("query");
    assert!(statement.step_row().expect("step"));
    let value = statement.column_text(0).expect("column");
    assert!(!statement.step_row().expect("end"));
    value
}

fn key(value: &str) -> RecipeJsonString {
    RecipeJsonString::from(value)
}

fn seed_two_recipe_revisions(product: &mut ProductDatabase<'_>) {
    product
        .append_execution_recipe(&recipe_input(
            "recipe-history",
            "operation-history-one",
            "event-history-one",
            "receipt-history-one",
            None,
        ))
        .unwrap();
    product
        .append_execution_recipe(&recipe_input(
            "recipe-history",
            "operation-history-two",
            "event-history-two",
            "receipt-history-two",
            Some("1"),
        ))
        .unwrap();
}

#[test]
fn every_historical_record_and_fingerprint_is_authoritative() {
    for revision in 1..=2 {
        for axis in 0..4 {
            fixture(|_, _, product| {
                seed_two_recipe_revisions(product);
                let table = match axis {
                    0 => "gogoke_objects",
                    1 => "gogoke_events",
                    _ => "gogoke_receipts",
                };
                let change = if axis == 3 {
                    "operation_fingerprint='sha256:0000000000000000000000000000000000000000000000000000000000000000'"
                } else {
                    "canonical_json=x'7b7d'"
                };
                product
                    .connection
                    .execute(&format!(
                        "UPDATE main.{table} SET {change} WHERE object_type='ExecutionRecipe' AND object_id='recipe-history' AND object_version='{revision}'"
                    ))
                    .unwrap();
                assert!(
                    product
                        .read_current_execution_recipe("domain-one", "recipe-history")
                        .is_err(),
                    "accepted damaged revision {revision} axis {axis}"
                );
            });
        }
    }
}

#[test]
fn owner_issuer_is_bound_to_the_current_product_database() {
    fixture(|_, _, product| {
        let other_path = temp_root("foreign-owner");
        let other_root = RootLock::acquire(&other_path).unwrap();
        let other_database = other_path.join("state.sqlite");
        let other = ProductDatabase::open(&other_root, &other_database).unwrap();
        let input = recipe_input(
            "recipe-owner",
            "operation-owner",
            "event-owner",
            "receipt-owner",
            None,
        );
        assert!(authority::append_owner_execution_recipe(
            &mut product.connection,
            &other.owner,
            &input
        )
        .is_err());
        product.append_execution_recipe(&input).unwrap();
        assert!(authority::read_current_execution_recipe(
            &mut product.connection,
            &other.owner,
            "domain-one",
            "recipe-owner"
        )
        .is_err());
        assert!(authority::read_execution_recipe_revision(
            &mut product.connection,
            &other.owner,
            "domain-one",
            "recipe-owner",
            "1"
        )
        .is_err());
        other.close_checked().unwrap();
        drop(other_root);
        cleanup(&other_path, &other_database);
    });
}

#[test]
fn core_and_head_schema_substitutes_and_effects_fail_closed() {
    for table in [
        "gogoke_objects",
        "gogoke_events",
        "gogoke_receipts",
        "gogoke_stream_heads",
        "gogoke_execution_recipe_heads",
    ] {
        for axis in 0..4 {
            fixture(|_, _, product| {
                seed_two_recipe_revisions(product);
                let sql = match axis {
                    0 => format!(
                        "CREATE TEMP VIEW {} AS SELECT * FROM main.{table}",
                        table.to_uppercase()
                    ),
                    1 => format!(
                        "CREATE TEMP TRIGGER recipe_schema_guard AFTER INSERT ON main.{table} BEGIN SELECT 1; END"
                    ),
                    2 => format!(
                        "CREATE TRIGGER recipe_schema_guard AFTER INSERT ON main.{table} BEGIN SELECT 1; END"
                    ),
                    _ => format!(
                        "ALTER TABLE main.{table} RENAME TO replaced_recipe_table; CREATE VIEW main.{table} AS SELECT * FROM main.replaced_recipe_table"
                    ),
                };
                product.connection.execute(&sql).unwrap();
                assert!(
                    product
                        .read_current_execution_recipe("domain-one", "recipe-history")
                        .is_err(),
                    "schema substitute accepted for {table} axis {axis}"
                );
            });
        }
    }
}

#[test]
fn trigger_cannot_corrupt_old_recipe_and_commit_new_revision() {
    for temporary in [false, true] {
        fixture(|_, _, product| {
            product
                .append_execution_recipe(&recipe_input(
                    "recipe-trigger",
                    "operation-trigger-one",
                    "event-trigger-one",
                    "receipt-trigger-one",
                    None,
                ))
                .unwrap();
            let prefix = if temporary { "TEMP " } else { "" };
            product
                .connection
                .execute(&format!(
                    "CREATE {prefix}TRIGGER recipe_corrupt_history AFTER INSERT ON main.gogoke_objects WHEN NEW.object_type='ExecutionRecipe' AND NEW.object_version='2' BEGIN UPDATE gogoke_objects SET canonical_json=x'7b7d' WHERE object_type='ExecutionRecipe' AND object_id=NEW.object_id AND object_version='1'; END"
                ))
                .unwrap();
            assert!(product
                .append_execution_recipe(&recipe_input(
                    "recipe-trigger",
                    "operation-trigger-two",
                    "event-trigger-two",
                    "receipt-trigger-two",
                    Some("1"),
                ))
                .is_err());
        });
    }
}

#[test]
fn more_than_sixty_four_revisions_are_revalidated() {
    fixture(|_, _, product| {
        for index in 0..70 {
            let previous = (index > 0).then(|| index.to_string());
            let mut input = recipe_input(
                "recipe-many",
                &format!("operation-many-{index}"),
                &format!("event-many-{index}"),
                &format!("receipt-many-{index}"),
                previous.as_deref(),
            );
            input.budget_policy = RecipeJsonValue::Array(
                (0..70)
                    .map(|value| RecipeJsonValue::Number(value as f64 / 2.0))
                    .collect(),
            );
            product.append_execution_recipe(&input).unwrap();
        }
        assert_eq!(
            product
                .read_current_execution_recipe("domain-one", "recipe-many")
                .unwrap()
                .unwrap()
                .recipe
                .revision,
            "70"
        );
        assert_eq!(
            product
                .read_execution_recipe_revision("domain-one", "recipe-many", "1")
                .unwrap()
                .unwrap()
                .recipe
                .revision,
            "1"
        );
    });
}

fn json_string(value: &str) -> RecipeJsonValue {
    RecipeJsonValue::String(RecipeJsonString::from(value))
}

fn recipe_input(
    recipe_id: &str,
    operation_id: &str,
    event_id: &str,
    receipt_id: &str,
    expected_previous_revision: Option<&str>,
) -> AppendExecutionRecipe {
    AppendExecutionRecipe {
        operation_id: operation_id.to_owned(),
        domain_id: "domain-one".to_owned(),
        expected_previous_revision: expected_previous_revision.map(str::to_owned),
        recipe_id: recipe_id.to_owned(),
        seat_id: "seat-worker-one".to_owned(),
        runtime_instance_id: "runtime-instance-one".to_owned(),
        model_ref: BTreeMap::from([
            (key("capabilityRevision"), json_string("4")),
            (key("nativeModelId"), json_string("native-model-one")),
            (key("resolvedVersion"), json_string("version-one")),
            (
                key("runtimeInstanceId"),
                json_string("runtime-instance-one"),
            ),
        ]),
        tool_profile: RecipeJsonValue::Object(BTreeMap::from([
            (key("temperature"), RecipeJsonValue::Number(0.25)),
            (
                key("tools"),
                RecipeJsonValue::Array(vec![json_string("read"), json_string("write")]),
            ),
        ])),
        isolation_profile: RecipeJsonValue::Object(BTreeMap::from([(
            key("mode"),
            json_string("private-test"),
        )])),
        context_manifest_id: "manifest-one".to_owned(),
        budget_policy: RecipeJsonValue::Object(BTreeMap::from([(
            key("maxSteps"),
            RecipeJsonValue::Number(128.0),
        )])),
        admission_ref: "admission-one".to_owned(),
        event_id: event_id.to_owned(),
        receipt_id: receipt_id.to_owned(),
        recorded_at: "2026-09-22T00:00:00Z".to_owned(),
    }
}

fn table_count(product: &ProductDatabase<'_>, table: &str, filter: &str) -> String {
    scalar(
        product,
        &format!("SELECT count(*) FROM {table} WHERE {filter}"),
    )
}

#[test]
fn typed_recipe_commits_and_read_receipt_stays_preparatory() {
    fixture(|_, _, product| {
        let input = recipe_input(
            "recipe-one",
            "operation-one",
            "event-one",
            "receipt-one",
            None,
        );
        let receipt = product
            .append_execution_recipe(&input)
            .expect("append typed recipe");
        assert_eq!(receipt.disposition, "COMMITTED");
        assert_eq!(receipt.storage_receipt.disposition, "COMMITTED");
        assert_eq!(receipt.currentness_status, CURRENTNESS_STATUS);
        assert_eq!(
            receipt.currentness_status,
            "PREPARATORY_CURRENTNESS_REQUIRED"
        );
        assert_eq!(receipt.version.recipe.revision, "1");
        assert_eq!(
            product
                .read_current_execution_recipe("domain-one", "recipe-one")
                .unwrap()
                .unwrap(),
            receipt.version
        );
        assert_eq!(
            table_count(product, "gogoke_objects", "object_type='ExecutionRecipe'"),
            "1"
        );
        assert_eq!(
            table_count(
                product,
                "gogoke_events",
                "event_type='ExecutionRecipeVersionCommitted'"
            ),
            "1"
        );
        assert_eq!(
            table_count(
                product,
                "gogoke_receipts",
                "receipt_type='ExecutionRecipeVersionCommitted'"
            ),
            "1"
        );
        assert_ne!(
            scalar(
                product,
                "SELECT instr(CAST(canonical_json AS TEXT),'PREPARATORY_CURRENTNESS_REQUIRED') FROM gogoke_receipts WHERE operation_id='operation-one'",
            ),
            "0"
        );
        let object = scalar(
            product,
            "SELECT CAST(canonical_json AS TEXT) FROM gogoke_objects WHERE object_type='ExecutionRecipe' AND object_id='recipe-one' AND object_version='1'",
        );
        assert!(object.contains("\"modelRef\":{"));
        assert!(!object.contains("\"contentHash\""));
        assert!(!object.contains("\"provider\""));
        assert!(!object.contains("\"account\""));
        assert!(!object.contains("\"credential\""));
    });
}

#[test]
fn immutable_history_replay_and_head_conflicts_are_durable() {
    fixture(|_, _, product| {
        let first = recipe_input(
            "recipe-one",
            "operation-one",
            "event-one",
            "receipt-one",
            None,
        );
        let first_receipt = product.append_execution_recipe(&first).unwrap();
        let replay = product.append_execution_recipe(&first).unwrap();
        assert_eq!(replay.disposition, "RECONCILED");
        assert_eq!(replay.version, first_receipt.version);

        let mut second = recipe_input(
            "recipe-one",
            "operation-two",
            "event-two",
            "receipt-two",
            Some("1"),
        );
        second.tool_profile = RecipeJsonValue::Object(BTreeMap::from([(
            key("temperature"),
            RecipeJsonValue::Number(-0.5),
        )]));
        let second_receipt = product.append_execution_recipe(&second).unwrap();
        assert_eq!(second_receipt.version.recipe.revision, "2");
        let historical_replay = product.append_execution_recipe(&first).unwrap();
        assert_eq!(historical_replay.disposition, "RECONCILED");
        assert_eq!(historical_replay.version, first_receipt.version);
        assert_eq!(
            product
                .read_current_execution_recipe("domain-one", "recipe-one")
                .unwrap()
                .unwrap(),
            second_receipt.version
        );
        assert_eq!(
            product
                .read_execution_recipe_revision("domain-one", "recipe-one", "1")
                .unwrap()
                .unwrap(),
            first_receipt.version
        );

        let stale = recipe_input(
            "recipe-one",
            "operation-stale",
            "event-stale",
            "receipt-stale",
            None,
        );
        assert!(matches!(
            product.append_execution_recipe(&stale),
            Err(OrchestrationError::OperationConflict)
        ));
        let mut changed_replay = first.clone();
        changed_replay.budget_policy = RecipeJsonValue::Null;
        assert!(matches!(
            product.append_execution_recipe(&changed_replay),
            Err(OrchestrationError::OperationConflict)
        ));
        assert_eq!(
            table_count(product, "gogoke_objects", "object_type='ExecutionRecipe'"),
            "2"
        );
    });
}

#[test]
fn typed_json_values_preserve_unicode_order_negative_fractions_and_over_64_items() {
    fixture(|_, _, product| {
        let mut input = recipe_input(
            "recipe-large",
            "operation-large",
            "event-large",
            "receipt-large",
            None,
        );
        let mut profile = BTreeMap::new();
        profile.insert(key(""), RecipeJsonValue::Number(2.0));
        profile.insert(key("𐀀"), RecipeJsonValue::Number(-0.75));
        profile.insert(
            RecipeJsonString::from_utf16_units(vec![0xd800]),
            RecipeJsonValue::String(RecipeJsonString::from_utf16_units(vec![0xdc00])),
        );
        profile.insert(
            key("a"),
            RecipeJsonValue::Array(
                (0..65)
                    .map(|value| RecipeJsonValue::Number(value as f64))
                    .collect(),
            ),
        );
        input.tool_profile = RecipeJsonValue::Object(profile);
        let receipt = product.append_execution_recipe(&input).unwrap();
        let object = scalar(
            product,
            "SELECT CAST(canonical_json AS TEXT) FROM gogoke_objects WHERE object_type='ExecutionRecipe' AND object_id='recipe-large' AND object_version='1'",
        );
        assert!(object.find("\"a\"").unwrap() < object.find("\"𐀀\"").unwrap());
        assert!(object.find("\"𐀀\"").unwrap() < object.find("\"\"").unwrap());
        let read = product
            .read_current_execution_recipe("domain-one", "recipe-large")
            .unwrap()
            .unwrap();
        assert_eq!(read, receipt.version);
        let RecipeJsonValue::Object(profile) = read.recipe.tool_profile else {
            panic!("object profile preserved as typed object");
        };
        let RecipeJsonValue::Array(items) = &profile[&key("a")] else {
            panic!("array profile preserved");
        };
        assert_eq!(items.len(), 65);
        assert_eq!(profile[&key("𐀀")], RecipeJsonValue::Number(-0.75));
        assert_eq!(
            profile[&RecipeJsonString::from_utf16_units(vec![0xd800])],
            RecipeJsonValue::String(RecipeJsonString::from_utf16_units(vec![0xdc00]))
        );
        assert!(object.contains(r#""\ud800":"\udc00""#));
    });
}

#[test]
fn invalid_number_is_rejected_before_any_durable_write() {
    fixture(|_, _, product| {
        let mut input = recipe_input(
            "recipe-invalid",
            "operation-invalid",
            "event-invalid",
            "receipt-invalid",
            None,
        );
        input.tool_profile = RecipeJsonValue::Number(f64::INFINITY);
        assert!(matches!(
            product.append_execution_recipe(&input),
            Err(OrchestrationError::Invalid("execution recipe JSON number"))
        ));
        assert_eq!(
            table_count(product, "gogoke_objects", "object_type='ExecutionRecipe'"),
            "0"
        );
        assert_eq!(
            table_count(
                product,
                "gogoke_receipts",
                "operation_id='operation-invalid'"
            ),
            "0"
        );
    });
}

#[test]
fn event_collision_rolls_back_object_event_receipt_and_recipe_head_together() {
    fixture(|_, _, product| {
        let first = recipe_input(
            "recipe-one",
            "operation-one",
            "event-shared",
            "receipt-one",
            None,
        );
        product.append_execution_recipe(&first).unwrap();
        let second = recipe_input(
            "recipe-two",
            "operation-two",
            "event-shared",
            "receipt-two",
            None,
        );
        assert!(product.append_execution_recipe(&second).is_err());
        assert_eq!(
            table_count(
                product,
                "gogoke_objects",
                "object_type='ExecutionRecipe' AND object_id='recipe-two'",
            ),
            "0"
        );
        assert_eq!(
            table_count(
                product,
                "gogoke_execution_recipe_heads",
                "recipe_id='recipe-two'"
            ),
            "0"
        );
        assert_eq!(
            table_count(product, "gogoke_receipts", "operation_id='operation-two'"),
            "0"
        );
        assert_eq!(
            table_count(
                product,
                "gogoke_stream_heads",
                "stream_id='gogoke.execution-recipe.v1/recipe-two'"
            ),
            "0"
        );
    });
}

#[test]
fn persistent_temp_triggers_and_tampered_record_fail_closed() {
    fixture(|_, _, product| {
        let input = recipe_input(
            "recipe-one",
            "operation-one",
            "event-one",
            "receipt-one",
            None,
        );
        product.append_execution_recipe(&input).unwrap();

        product
            .connection
            .execute("CREATE TEMP TRIGGER recipe_temp_guard BEFORE INSERT ON main.gogoke_execution_recipe_heads BEGIN SELECT RAISE(ABORT,'fixture'); END")
            .expect("create temp trigger");
        assert!(matches!(
            product.read_current_execution_recipe("domain-one", "recipe-one"),
            Err(OrchestrationError::AccessDenied)
        ));
        product
            .connection
            .execute("DROP TRIGGER recipe_temp_guard")
            .unwrap();

        product
            .connection
            .execute("CREATE TRIGGER recipe_main_guard BEFORE INSERT ON gogoke_execution_recipe_heads BEGIN SELECT RAISE(ABORT,'fixture'); END")
            .expect("create main trigger");
        assert!(matches!(
            product.read_current_execution_recipe("domain-one", "recipe-one"),
            Err(OrchestrationError::AccessDenied)
        ));
        product
            .connection
            .execute("DROP TRIGGER recipe_main_guard")
            .unwrap();

        product
            .connection
            .execute("UPDATE gogoke_objects SET canonical_json=CAST('{}' AS BLOB) WHERE domain_id='domain-one' AND object_type='ExecutionRecipe' AND object_id='recipe-one' AND object_version='1'")
            .expect("tamper test row");
        assert!(matches!(
            product.read_current_execution_recipe("domain-one", "recipe-one"),
            Err(OrchestrationError::AccessDenied)
        ));
    });
}

#[test]
fn tampered_event_receipt_and_current_head_fail_closed() {
    fixture(|_, _, product| {
        for (recipe_id, operation_id, event_id, receipt_id) in [
            ("recipe-event", "op-event", "event-tamper", "receipt-event"),
            (
                "recipe-receipt",
                "op-receipt",
                "event-receipt",
                "receipt-tamper",
            ),
            ("recipe-head", "op-head", "event-head", "receipt-head"),
        ] {
            let input = recipe_input(recipe_id, operation_id, event_id, receipt_id, None);
            product.append_execution_recipe(&input).unwrap();
        }

        product
            .connection
            .execute("UPDATE gogoke_events SET canonical_json=CAST('{}' AS BLOB) WHERE domain_id='domain-one' AND event_id='event-tamper'")
            .expect("tamper event body");
        assert!(matches!(
            product.read_current_execution_recipe("domain-one", "recipe-event"),
            Err(OrchestrationError::AccessDenied)
        ));

        product
            .connection
            .execute("UPDATE gogoke_receipts SET canonical_json=CAST('{}' AS BLOB) WHERE domain_id='domain-one' AND receipt_id='receipt-tamper'")
            .expect("tamper receipt body");
        assert!(matches!(
            product.read_current_execution_recipe("domain-one", "recipe-receipt"),
            Err(OrchestrationError::AccessDenied)
        ));

        let forged_hash = format!("sha256:{}", "0".repeat(64));
        product
            .connection
            .execute(&format!(
                "UPDATE gogoke_execution_recipe_heads SET content_hash='{forged_hash}' WHERE domain_id='domain-one' AND recipe_id='recipe-head'"
            ))
            .expect("tamper current head");
        assert!(matches!(
            product.read_current_execution_recipe("domain-one", "recipe-head"),
            Err(OrchestrationError::AccessDenied)
        ));
    });
}

#[test]
fn execution_recipe_reopens_with_current_and_historical_versions() {
    let _guard = route_b_test_guard();
    let path = temp_root("reopen");
    let root = RootLock::acquire(&path).expect("root lock");
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).expect("product database");
    let first = recipe_input(
        "recipe-one",
        "operation-one",
        "event-one",
        "receipt-one",
        None,
    );
    let first_version = product.append_execution_recipe(&first).unwrap().version;
    let second = recipe_input(
        "recipe-one",
        "operation-two",
        "event-two",
        "receipt-two",
        Some("1"),
    );
    let second_version = product.append_execution_recipe(&second).unwrap().version;
    product.close_checked().expect("close before reopen");

    let mut reopened = ProductDatabase::open(&root, &database).expect("reopen same Product DB");
    assert_eq!(
        reopened
            .read_current_execution_recipe("domain-one", "recipe-one")
            .unwrap()
            .unwrap(),
        second_version
    );
    assert_eq!(
        reopened
            .read_execution_recipe_revision("domain-one", "recipe-one", "1")
            .unwrap()
            .unwrap(),
        first_version
    );
    reopened.close_checked().expect("close reopened");
    drop(root);
    cleanup(&path, &database);
}
