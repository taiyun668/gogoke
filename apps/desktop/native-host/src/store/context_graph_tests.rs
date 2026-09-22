//! Native graph regressions. These definitions are not execution evidence.
use super::atomic::Statement;
use super::context::apply_context_schema;
use super::context_graph::{self as graph, ContextNode};
use super::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use crate::root::RootLock;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(test: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-qualified-graph-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    apply_context_schema(&mut connection).unwrap();
    test(&mut connection);
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) { eprintln!("owned graph fixture retained: {error}"); }
}

fn node(domain: &str, reference: &str) -> ContextNode {
    ContextNode { domain_id: domain.into(), version_ref: reference.into() }
}

fn rows(c: &VerifiedDatabaseConnection<'_>, sql: &str, columns: i32) -> Vec<Vec<String>> {
    let statement = Statement::prepare(c.as_ptr(), sql).unwrap();
    let mut rows = Vec::new();
    while statement.step_row().unwrap() {
        rows.push((0..columns).map(|i| statement.column_text(i).unwrap()).collect());
    }
    rows
}

fn begin(c: &mut VerifiedDatabaseConnection<'_>) {
    c.execute("BEGIN IMMEDIATE").unwrap();
    graph::ensure_schema(c).unwrap();
}

fn state(c: &VerifiedDatabaseConnection<'_>, n: &ContextNode) -> String {
    let s = Statement::prepare(c.as_ptr(), "SELECT state FROM gogoke_context_states WHERE domain_id=? AND version_ref=?").unwrap();
    s.bind_text(1, &n.domain_id).unwrap(); s.bind_text(2, &n.version_ref).unwrap();
    assert!(s.step_row().unwrap()); s.column_text(0).unwrap()
}

fn seed_state(c: &VerifiedDatabaseConnection<'_>, n: &ContextNode, value: &str) {
    let s = Statement::prepare(c.as_ptr(), "INSERT INTO gogoke_context_states(domain_id,version_ref,state) VALUES(?,?,?)").unwrap();
    s.bind_text(1, &n.domain_id).unwrap(); s.bind_text(2, &n.version_ref).unwrap();
    s.bind_text(3, value).unwrap(); s.step_done().unwrap();
}

#[test]
fn graph_rejects_autocommit_without_mutating_schema() {
    fixture(|c| {
        let before = rows(c, "SELECT sql FROM sqlite_schema WHERE name='gogoke_context_edges'", 1);
        assert!(graph::ensure_schema(c).is_err());
        assert_eq!(rows(c, "SELECT sql FROM sqlite_schema WHERE name='gogoke_context_edges'", 1), before);
    });
}

#[test]
fn legacy_edges_keep_both_domains_and_schema_change_is_idempotent() {
    fixture(|c| {
        c.execute("INSERT INTO gogoke_context_edges VALUES('a','source@1','child@1','DERIVED')").unwrap();
        begin(c);
        graph::ensure_schema(c).unwrap();
        assert_eq!(rows(c, "SELECT domain_id,source_domain_id,from_ref,to_ref,edge_kind FROM gogoke_context_edges", 5),
            vec![vec!["a".to_owned(), "a".into(), "source@1".into(), "child@1".into(), "DERIVED".into()]]);
        c.execute("COMMIT").unwrap();
    });
}

#[test]
fn schema_and_rows_are_restored_by_outer_rollback() {
    fixture(|c| {
        c.execute("INSERT INTO gogoke_context_edges VALUES('a','source@1','child@1','DERIVED')").unwrap();
        let before = rows(c, "SELECT sql FROM sqlite_schema WHERE name='gogoke_context_edges'", 1);
        begin(c);
        c.execute("ROLLBACK").unwrap();
        assert_eq!(rows(c, "SELECT sql FROM sqlite_schema WHERE name='gogoke_context_edges'", 1), before);
        assert_eq!(rows(c, "SELECT domain_id,from_ref,to_ref,edge_kind FROM gogoke_context_edges", 4),
            vec![vec!["a".to_owned(), "source@1".into(), "child@1".into(), "DERIVED".into()]]);
        assert!(rows(c, "SELECT name FROM sqlite_schema WHERE name='gogoke_context_edges_r4_staging'", 1).is_empty());
    });
}

#[test]
fn unknown_graph_shape_is_not_guessed_or_migrated() {
    fixture(|c| {
        c.execute("DROP TABLE gogoke_context_edges").unwrap();
        c.execute("CREATE TABLE gogoke_context_edges(payload TEXT) STRICT").unwrap();
        c.execute("INSERT INTO gogoke_context_edges VALUES('retain-me')").unwrap();
        c.execute("BEGIN IMMEDIATE").unwrap();
        assert!(graph::ensure_schema(c).is_err());
        c.execute("ROLLBACK").unwrap();
        assert_eq!(rows(c, "SELECT payload FROM gogoke_context_edges", 1), vec![vec!["retain-me".to_owned()]]);
    });
}

#[test]
fn graph_dependencies_and_staging_collision_fail_closed() {
    fixture(|c| {
        for ddl in [
            "CREATE TRIGGER unrelated BEFORE INSERT ON gogoke_context_edges BEGIN SELECT 1; END",
            "CREATE VIEW graph_view AS SELECT * FROM GOGOKE_CONTEXT_EDGES",
            "CREATE INDEX graph_index ON gogoke_context_edges(from_ref)",
            "CREATE TABLE dependent(a TEXT,b TEXT,c TEXT,d TEXT,FOREIGN KEY(a,b,c,d) REFERENCES GOGOKE_CONTEXT_EDGES(domain_id,from_ref,to_ref,edge_kind))",
            "CREATE TABLE GOGOKE_CONTEXT_EDGES_R4_STAGING(value TEXT)",
        ] {
            c.execute("BEGIN IMMEDIATE").unwrap();
            c.execute(ddl).unwrap();
            assert!(graph::ensure_schema(c).is_err(), "{ddl}");
            c.execute("ROLLBACK").unwrap();
        }
    });
}

#[test]
fn same_reference_in_different_domains_does_not_alias() {
    fixture(|c| {
        begin(c);
        let a = node("a", "same@1"); let b = node("b", "same@1"); let child = node("c", "child@1");
        graph::insert_edge(c, &a, &child, "DERIVED").unwrap();
        assert_eq!(graph::descendants(c, &a).unwrap(), vec![child.clone()]);
        assert!(graph::descendants(c, &b).unwrap().is_empty());
        graph::insert_edge(c, &b, &child, "DERIVED").unwrap();
        assert_eq!(rows(c, "SELECT count(*) FROM gogoke_context_edges", 1)[0][0], "2");
        c.execute("COMMIT").unwrap();
    });
}

#[test]
fn recursive_traversal_keeps_domain_on_every_hop() {
    fixture(|c| {
        begin(c);
        let root = node("a", "root@1"); let mid = node("b", "same@1");
        let leaf = node("c", "leaf@1"); let decoy = node("d", "same@1"); let hidden = node("e", "hidden@1");
        graph::insert_edge(c, &root, &mid, "DERIVED").unwrap();
        graph::insert_edge(c, &mid, &leaf, "DERIVED").unwrap();
        graph::insert_edge(c, &decoy, &hidden, "DERIVED").unwrap();
        assert_eq!(graph::descendants(c, &root).unwrap(), vec![mid, leaf]);
        c.execute("COMMIT").unwrap();
    });
}

#[test]
fn invalidation_keeps_nonactive_and_unrelated_states() {
    fixture(|c| {
        begin(c);
        let root = node("a", "root@1"); let child = node("b", "child@1");
        let revoked = node("c", "child@1"); let unrelated = node("d", "child@1");
        for (n, value) in [(&root,"SUPERSEDED"),(&child,"ACTIVE"),(&revoked,"REVOKED"),(&unrelated,"ACTIVE")] {
            seed_state(c,n,value);
        }
        graph::insert_edge(c,&root,&child,"DERIVED").unwrap();
        graph::insert_edge(c,&child,&revoked,"DERIVED").unwrap();
        assert_eq!(graph::invalidate_descendants(c,&root).unwrap(),vec![child.clone(),revoked.clone()]);
        assert_eq!(state(c,&root),"SUPERSEDED"); assert_eq!(state(c,&child),"STALE");
        assert_eq!(state(c,&revoked),"REVOKED"); assert_eq!(state(c,&unrelated),"ACTIVE");
        c.execute("ROLLBACK").unwrap();
    });
}

#[test]
fn invalidation_changes_rollback_together() {
    fixture(|c| {
        begin(c);
        let root=node("a","root@1"); let child=node("b","child@1");
        seed_state(c,&root,"ACTIVE"); seed_state(c,&child,"ACTIVE");
        graph::insert_edge(c,&root,&child,"DERIVED").unwrap(); c.execute("COMMIT").unwrap();
        c.execute("BEGIN IMMEDIATE").unwrap();
        graph::invalidate_descendants(c,&root).unwrap(); assert_eq!(state(c,&child),"STALE");
        c.execute("ROLLBACK").unwrap(); assert_eq!(state(c,&child),"ACTIVE");
    });
}

#[test]
fn self_edges_and_cross_domain_supersede_are_rejected() {
    fixture(|c| {
        begin(c);
        let a=node("a","same@1"); let b=node("b","same@1");
        assert!(graph::insert_edge(c,&a,&a,"DERIVED").is_err());
        assert!(graph::insert_edge(c,&a,&b,"SUPERSEDES").is_err());
        assert!(graph::insert_edge(c,&a,&b,"UNKNOWN").is_err());
        graph::insert_edge(c,&a,&b,"DERIVED").unwrap();
        c.execute("COMMIT").unwrap();
    });
}

#[test]
fn source_cycle_is_detected_before_invalidation() {
    fixture(|c| {
        begin(c);
        let a=node("a","same@1"); let b=node("b","same@1");
        seed_state(c,&a,"ACTIVE"); seed_state(c,&b,"ACTIVE");
        graph::insert_edge(c,&a,&b,"DERIVED").unwrap(); graph::insert_edge(c,&b,&a,"DERIVED").unwrap();
        assert!(graph::invalidate_descendants(c,&a).is_err());
        assert_eq!(state(c,&a),"ACTIVE"); assert_eq!(state(c,&b),"ACTIVE");
        c.execute("ROLLBACK").unwrap();
    });
}

#[test]
fn oversized_set_is_rejected_before_any_state_write() {
    fixture(|c| {
        begin(c);
        c.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<16385) INSERT INTO gogoke_context_edges(domain_id,source_domain_id,from_ref,to_ref,edge_kind) SELECT 'b','a','root@1','child-'||x||'@1','DERIVED' FROM n").unwrap();
        c.execute("INSERT INTO gogoke_context_states SELECT 'b',to_ref,'ACTIVE' FROM gogoke_context_edges").unwrap();
        assert!(graph::invalidate_descendants(c,&node("a","root@1")).is_err());
        assert_eq!(rows(c,"SELECT count(*) FROM gogoke_context_states WHERE state='ACTIVE'",1)[0][0],"16385");
        c.execute("ROLLBACK").unwrap();
    });
}
