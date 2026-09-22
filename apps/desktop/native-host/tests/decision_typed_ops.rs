use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn host() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_gogoke-native-host"))
}
fn write_frame(client: &mut std::fs::File, frame: &str) {
    client.write_all(&(frame.len() as u32).to_le_bytes()).unwrap();
    client.write_all(frame.as_bytes()).unwrap();
}
fn read_frame(client: &mut std::fs::File) -> String {
    let mut length=[0u8;4];client.read_exact(&mut length).unwrap();
    let mut body=vec![0u8;u32::from_le_bytes(length) as usize];client.read_exact(&mut body).unwrap();
    String::from_utf8(body).unwrap()
}
fn digest(ch:char)->String{format!("sha256:{}",ch.to_string().repeat(64))}

#[test]
fn authenticated_service_can_atomically_commit_and_replay_bounded_decision() {
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root=std::env::temp_dir().join(format!("gogoke-decision-typed-{nonce}"));std::fs::create_dir(&root).unwrap();
    let mut child=Command::new(host()).arg("--root").arg(&root).stdout(Stdio::piped()).spawn().unwrap();
    let mut stdout=BufReader::new(child.stdout.take().unwrap());let mut line=String::new();
    stdout.read_line(&mut line).unwrap();assert!(line.starts_with("LOCKED"));
    line.clear();stdout.read_line(&mut line).unwrap();assert!(line.starts_with("PIPE\t"));
    let path=line.trim()[5..].to_owned();
    line.clear();stdout.read_line(&mut line).unwrap();assert!(line.starts_with("CAPABILITY\t"));
    let capability=line.trim()[11..].to_owned();
    let mut client=OpenOptions::new().read(true).write(true).open(path).unwrap();
    client.write_all(&[0x47]).unwrap();
    write_frame(&mut client,&format!("{{\"capability\":\"{capability}\",\"operation\":\"AuthenticateService\"}}"));
    assert!(read_frame(&mut client).contains("\"authenticated\":true"));

    let action_digest=digest('c');
    write_frame(&mut client,&format!(
        "{{\"actionDigest\":\"{action_digest}\",\"actionOperationId\":\"opr_11111111111111111111111111111111\",\"authRevision\":\"2\",\"bindingGeneration\":\"7\",\"bindingId\":\"binding-one\",\"candidateHash\":\"{}\",\"candidateId\":\"candidate-one\",\"capabilityRevision\":\"3\",\"capacityTotal\":\"2\",\"operation\":\"PublishDecisionSnapshot\",\"operationId\":\"decision-op\",\"policyRevision\":\"1\",\"resourceRef\":\"pool-one\",\"resourceRevision\":\"5\",\"stateViewHash\":\"{}\",\"taskRevision\":\"1\"}}",
        digest('b'),digest('a')
    ));
    assert!(read_frame(&mut client).contains("\"published\":true"));

    let decision=format!(
        "{{\"actionIntentRef\":\"opr_11111111111111111111111111111111\",\"backendKind\":\"FAKE\",\"bindingGeneration\":\"7\",\"budgetUnits\":\"1\",\"candidateHash\":\"{}\",\"capabilityRevision\":\"3\",\"choice\":\"candidate-one\",\"deadlineEpochMs\":\"1000\",\"decisionId\":\"decision-one\",\"domainId\":\"domain-one\",\"eventId\":\"decision-event\",\"family\":\"RESOURCE_SELECTION\",\"modelRequested\":\"\",\"modelResolved\":\"fake-v1\",\"operation\":\"CommitDecision\",\"operationId\":\"decision-op\",\"policyRevision\":\"1\",\"questionVersion\":\"1\",\"reason\":\"QUALIFIED_BOUNDED_SELECTION\",\"receiptId\":\"decision-receipt\",\"recordedAt\":\"2026-09-21T00:00:00Z\",\"requiredCapacityUnits\":\"1\",\"resourceReservationRef\":\"capacity-lease-one\",\"rubricVersion\":\"1\",\"scenarioId\":\"DF02\",\"stateViewHash\":\"{}\",\"taskRevision\":\"1\"}}",
        digest('b'),digest('a')
    );
    write_frame(&mut client,&decision);
    let committed=read_frame(&mut client);
    assert!(committed.contains("\"kind\":\"committed\""),"{committed}");
    assert!(committed.contains("\"decisionReceiptId\":\"decision-receipt\""),"{committed}");

    write_frame(&mut client,&decision);
    let replay=read_frame(&mut client);
    assert!(replay.contains("\"kind\":\"replayed\""),"{replay}");
    assert!(replay.contains("\"choice\":\"candidate-one\""),"{replay}");

    write_frame(&mut client,r#"{"domainId":"domain-one","operation":"ReadDecisionReplay","operationId":"decision-op"}"#);
    let readback=read_frame(&mut client);
    assert!(readback.contains("\"kind\":\"replayed\""),"{readback}");
    assert!(readback.contains("\"stateViewHash\":\"sha256:aaaaaaaa"),"{readback}");

    write_frame(&mut client,r#"{"operation":"Shutdown"}"#);let _=read_frame(&mut client);
    let _=child.wait();std::fs::remove_dir_all(root).ok();
}
