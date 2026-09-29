use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn host() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_gogoke-native-host"))
}

fn write_frame(client: &mut std::fs::File, frame: &[u8]) {
    client
        .write_all(&(frame.len() as u32).to_le_bytes())
        .expect("length");
    client.write_all(frame).expect("frame");
}

#[track_caller]
fn read_frame(client: &mut std::fs::File) -> String {
    let mut length = [0u8; 4];
    client.read_exact(&mut length).expect("length");
    let mut body = vec![0u8; u32::from_le_bytes(length) as usize];
    client.read_exact(&mut body).unwrap_or_else(|error| {
        panic!("reply body length={}: {error}", body.len())
    });
    String::from_utf8(body).expect("utf8")
}

fn verbatim_path(path: &Path) -> Option<PathBuf> {
    let text = path.to_str()?;
    if text.starts_with("\\\\?\\") {
        return None;
    }
    (text.as_bytes().get(1) == Some(&b':')).then(|| PathBuf::from(format!("\\\\?\\{text}")))
}

fn second_host_is_refused(root: &Path) -> bool {
    let mut contender = match Command::new(host())
        .arg("--root")
        .arg(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut exited = false;
    while Instant::now() < deadline {
        match contender.try_wait() {
            Ok(Some(_)) => {
                exited = true;
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => break,
        }
    }
    if !exited {
        let _ = contender.kill();
    }
    let output = match contender.wait_with_output() {
        Ok(output) => output,
        Err(_) => return false,
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    exited
        && !output.status.success()
        && stderr.contains("ROOT_ALREADY_LOCKED:")
        && !output.stdout.windows(7).any(|bytes| bytes == b"LOCKED\t")
        && !output.stdout.windows(5).any(|bytes| bytes == b"PIPE\t")
        && !output
            .stdout
            .windows(11)
            .any(|bytes| bytes == b"CAPABILITY\t")
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

    let same_spelling_refused = second_host_is_refused(&root);
    let verbatim_root = verbatim_path(&root);
    let verbatim_spelling_refused = verbatim_root
        .as_deref()
        .map(second_host_is_refused)
        .unwrap_or(true);

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
    assert!(
        same_spelling_refused,
        "second host acquired or served the same root"
    );
    if verbatim_root.is_some() {
        assert!(
            verbatim_spelling_refused,
            "second host acquired or served the verbatim spelling of the same root"
        );
    }
}

#[test]
fn desktop_host_survives_service_disconnect_but_stops_after_user_disconnect() {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("gogoke-desktop-host-{nonce}"));
    std::fs::create_dir(&root).expect("root");
    let mut child = Command::new(host())
        .arg("--root").arg(&root)
        .arg("--desktop-session").arg("--user-pid")
        .arg(std::process::id().to_string())
        .stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().expect("desktop host");
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut locked = String::new();
    let mut service_line = String::new();
    let mut capability_line = String::new();
    let mut user_line = String::new();
    output.read_line(&mut locked).expect("locked");
    output.read_line(&mut service_line).expect("service pipe");
    output.read_line(&mut capability_line).expect("service capability");
    output.read_line(&mut user_line).expect("user pipe");
    assert!(locked.starts_with("LOCKED\t"), "{locked}");
    assert!(service_line.starts_with("PIPE\t"), "{service_line}");
    assert!(capability_line.starts_with("CAPABILITY\t"), "invalid capability line");
    assert!(user_line.starts_with("USER_PIPE\t"), "{user_line}");
    let service_path = service_line.trim_end().strip_prefix("PIPE\t").unwrap();
    let capability = capability_line.trim_end().strip_prefix("CAPABILITY\t").unwrap();
    let user_path = user_line.trim_end().strip_prefix("USER_PIPE\t").unwrap();
    let mut user = OpenOptions::new().read(true).write(true).open(user_path).expect("User pipe");
    user.write_all(&[0x47]).expect("User preface");
    let host_path = host().to_string_lossy().replace('\\', "\\\\");
    let user_request = format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-INSTANCE\",\"operation\":\"register\",\"requestId\":\"registerA\",\"targetId\":\"instanceA\",\"domainId\":\"global\",\"expectedRevision\":\"0\",\"payload\":{{\"driverId\":\"codex\",\"programPath\":\"{host_path}\",\"version\":\"test\"}}}}");
    let authenticate = format!("{{\"capability\":\"{capability}\",\"operation\":\"AuthenticateService\"}}");
    let connect_service = || {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(mut service) = OpenOptions::new().read(true).write(true).open(service_path) {
                service.write_all(&[0x47]).expect("service preface");
                write_frame(&mut service, authenticate.as_bytes());
                assert!(read_frame(&mut service).contains("\"authenticated\":true"));
                return service;
            }
            assert!(Instant::now() < deadline, "service did not rebind");
            thread::sleep(Duration::from_millis(20));
        }
    };
    let mut first = connect_service();
    write_frame(&mut first, user_request.as_bytes());
    assert!(read_frame(&mut first).starts_with("ERR"), "service promoted User request");
    drop(first);
    // The same fresh request must still apply through User. A prior User
    // commit would make an accidental service replay invisible here.
    write_frame(&mut user, user_request.as_bytes());
    assert!(read_frame(&mut user).contains("\"status\":\"APPLIED\""));
    write_frame(&mut user, user_request.as_bytes());
    assert!(read_frame(&mut user).contains("\"status\":\"REPLAYED\""));
    let mut second = connect_service();
    write_frame(&mut second, br#"{"operation":"Shutdown"}"#);
    assert!(read_frame(&mut second).starts_with("ERR"), "shared service stopped host");
    drop(second);
    assert!(child.try_wait().unwrap().is_none(), "service disconnect stopped host");
    drop(user);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("host status") { break status; }
        assert!(Instant::now() < deadline, "User disconnect did not stop host");
        thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        let mut stderr = String::new();
        child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
        panic!("desktop host exit: {status}; stderr={stderr}");
    }
    std::fs::remove_dir_all(&root).expect("owned root cleanup");
}
