use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn host() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_gogoke-native-host"))
}

fn write_frame(client: &mut std::fs::File, frame: &[u8]) {
    client
        .write_all(&(frame.len() as u32).to_le_bytes())
        .expect("length");
    client.write_all(frame).expect("frame");
}

fn read_frame(client: &mut std::fs::File) -> String {
    let mut length = [0u8; 4];
    client.read_exact(&mut length).expect("length");
    let mut body = vec![0u8; u32::from_le_bytes(length) as usize];
    client.read_exact(&mut body).expect("body");
    String::from_utf8(body).expect("utf8")
}

fn legacy_action_frame(operation:&str,digest:&str,reservation:&str,session:&str)->String{
    let bound=format!("sha256:{}","a".repeat(64));
    format!("{{\"actionKind\":\"queue\",\"authRevision\":\"7\",\"bindingId\":\"binding-1\",\"childCeilingDigest\":\"{bound}\",\"childExecutionId\":\"execution-1\",\"childGeneration\":\"11\",\"childSessionId\":\"{session}\",\"executionId\":\"execution-1\",\"generation\":\"11\",\"instructionDigest\":\"{bound}\",\"lane\":\"work\",\"materialSetDigest\":\"{bound}\",\"operation\":\"{operation}\",\"operationId\":\"opr_11111111111111111111111111111111\",\"packageDigest\":\"{bound}\",\"parentCeilingDigest\":\"{bound}\",\"parentGrantRef\":\"grant-one\",\"parentGrantRevision\":\"1\",\"payloadHex\":\"7b7d\",\"policyAction\":\"delegate\",\"profileId\":\"profile-1\",\"reservationId\":\"{reservation}\",\"route\":\"controller-worker\",\"runtimeInstanceId\":\"runtime-1\",\"semanticDigest\":\"{digest}\",\"sessionId\":\"{session}\",\"sink\":\"task-package\",\"sourceDomainId\":\"domain-source\",\"sourceExecutionId\":\"source-execution\",\"sourceGeneration\":\"1\",\"sourcePrincipalId\":\"principal-source\",\"sourceProjectId\":\"project-one\",\"sourceRole\":\"controller\",\"sourceSessionId\":\"source-session\",\"targetDomainId\":\"domain-target\",\"targetPrincipalId\":\"principal-target\",\"targetProjectId\":\"project-one\",\"targetRole\":\"worker\"}}")
}

#[test]
fn authenticated_service_uses_typed_host_without_sql_transport() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gogoke-host-{nonce}"));
    std::fs::create_dir(&root).expect("root");
    let mut child = Command::new(host())
        .arg("--root")
        .arg(&root)
        .stdout(Stdio::piped())
        .spawn()
        .expect("host");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    let mut line = String::new();
    stdout.read_line(&mut line).expect("locked");
    assert!(line.starts_with("LOCKED"), "{line}");
    line.clear();
    stdout.read_line(&mut line).expect("pipe");
    assert!(line.starts_with("PIPE\t"), "{line}");
    let path = line.trim()[5..].to_owned();
    line.clear();
    stdout.read_line(&mut line).expect("service capability");
    assert!(line.starts_with("CAPABILITY\t"), "{line}");
    let capability = line.trim()[11..].to_owned();
    let mut client = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open pipe");
    client.write_all(&[0x47]).expect("preface");
    let authenticate = format!("{{\"capability\":\"{capability}\",\"operation\":\"AuthenticateService\"}}");
    write_frame(&mut client, authenticate.as_bytes());
    let authenticated = read_frame(&mut client);
    assert!(authenticated.contains("\"authenticated\":true"), "{authenticated}");

    write_frame(&mut client, br#"{"operation":"execute","sql":"SELECT 1"}"#);
    let sql_reply = read_frame(&mut client);
    assert!(sql_reply.starts_with("ERR"), "{sql_reply}");

    let started = Instant::now();
    let commit = br#"{"commandId":"cmd-project","commandType":"project.create","events":[{"eventId":"ev-p","occurredAt":"2026-09-20T00:00:00Z","projectId":"proj-1","title":"one","type":"project.created","workspaceRoot":"C:/tmp/one"}],"operation":"CommitOrchestration"}"#;
    write_frame(&mut client, commit);
    let committed = read_frame(&mut client);
    assert!(committed.contains("COMMITTED"), "{committed}");
    assert!(started.elapsed().as_nanos() > 0);

    write_frame(&mut client, br#"{"limit":"10","operation":"ReadSnapshot"}"#);
    let snapshot = read_frame(&mut client);
    assert!(snapshot.contains("\"count\":1"), "{snapshot}");

    write_frame(
        &mut client,
        br#"{"commandId":"cmd-project","operation":"GetReceipt"}"#,
    );
    let receipt = read_frame(&mut client);
    assert!(receipt.contains("\"found\":true"), "{receipt}");

    let digest = format!("sha256:{}", "a".repeat(64));
    let legacy = legacy_action_frame("ReserveAction",&digest,"reservation-1","session-1");
    write_frame(&mut client, legacy.as_bytes());
    let legacy_reply = read_frame(&mut client);
    assert!(legacy_reply.starts_with("ERR"), "{legacy_reply}");

    let reserve = br#"{"actionKind":"queue","actionOperationId":"opr_11111111111111111111111111111111","contextManifestId":"manifest-one","domainId":"domain-one","lane":"work","operation":"ReserveAction","packageOperationId":"package-one","parentGrantRef":"grant-one","payload":"instruction","recipeId":"recipe-one","reservationId":"reservation-one","sessionId":"session-one","taskId":"task-one"}"#;
    write_frame(&mut client, reserve);
    let missing_authority = read_frame(&mut client);
    assert!(missing_authority.starts_with("ERR"), "{missing_authority}");

    let begin = br#"{"domainId":"domain-one","operation":"BeginActionCommitment","operationId":"opr_11111111111111111111111111111111","reservationId":"reservation-one"}"#;
    write_frame(&mut client, begin);
    let no_reservation = read_frame(&mut client);
    assert!(no_reservation.starts_with("ERR"), "{no_reservation}");

    write_frame(&mut client, br#"{"operation":"Shutdown"}"#);
    let _ = read_frame(&mut client);
    let _ = child.wait();
    std::fs::remove_dir_all(&root).ok();
}
