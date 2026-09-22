//! Real native typed-line ingress regression. Requires controlled Windows.
//! This is not a private-pipe/process qualification and zero tests is not PASS.
#![cfg(windows)]
use gogoke_native_host::root::RootLock;
use gogoke_native_host::store::session::{open_product_database, serve_lines};
use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn unauthenticated_typed_ingress_cannot_write_project_or_global_context() {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-authority-ingress-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = open_product_database(&root, &database).unwrap();
    let source = format!(
        "{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"sha256:{}\",\"contextId\":\"source-one\",\"derivedFrom\":\"\",\"domainId\":\"domain-one\",\"kind\":\"fact\",\"operation\":\"CommitContextVersion\",\"operationId\":\"source-operation\",\"readGrantRefs\":\"\",\"scope\":\"PROJECT\",\"sourceAuthorityKind\":\"repository\",\"sourceAuthorityRef\":\"authority://one\",\"sourceHash\":\"sha256:{}\",\"sourceRef\":\"source://one\",\"supersedes\":\"\",\"version\":\"1\",\"visibility\":\"OWNER_PRIVATE\"}}",
        "a".repeat(64), "b".repeat(64));
    let global = source.replace("\"scope\":\"PROJECT\"", "\"scope\":\"GLOBAL\"")
        .replace("\"contextId\":\"source-one\"", "\"contextId\":\"global-one\"")
        .replace("\"operationId\":\"source-operation\"", "\"operationId\":\"global-operation\"")
        .replace("\"derivedFrom\":\"\"", "\"derivedFrom\":\"source-one@1\"")
        .replace("\"visibility\":\"OWNER_PRIVATE\"", "\"visibility\":\"OWNER_PRIVATE\",\"sourceVersionRef\":\"source-one@1\",\"sourceGrantRef\":\"grant://source\",\"targetGrantRef\":\"grant://target\",\"provenanceRefs\":\"evidence://review\"");
    let input = format!(
        "{source}\n{global}\n{global}\n{{\"operation\":\"PublishContextAssemblySnapshot\"}}\n{{\"operation\":\"ReadGranteeContextSet\"}}\n{{\"operation\":\"CommitContextManifest\"}}\n{{\"operation\":\"ReadContextManifest\"}}\n"
    );
    let mut output = Vec::new();
    serve_lines(&mut connection, Cursor::new(input.as_bytes()), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 8, "one readiness line and seven actual replies required");
    assert_eq!(lines[0], "READY");
    assert!(lines[1].starts_with("ERR\tAccessDenied\t"), "{text}");
    assert!(lines[2].starts_with("ERR\tAccessDenied\t"), "{text}");
    assert!(lines[3].starts_with("ERR\tAccessDenied\t"), "{text}");
    for line in &lines[4..=7] {
        assert!(line.starts_with("ERR\tAccessDenied\t"), "{text}");
    }
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(e) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {e}"); }
}
