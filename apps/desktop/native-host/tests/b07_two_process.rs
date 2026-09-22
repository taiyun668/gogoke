use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn probe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_gogoke-root-lock-probe"))
}

#[test]
fn second_process_loses_on_alias_and_canonical_root() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gogoke-b07-{nonce}"));
    std::fs::create_dir(&root).expect("root");
    let mut holder = Command::new(probe())
        .arg(&root)
        .arg("3000")
        .stdout(Stdio::piped())
        .spawn()
        .expect("holder");
    let mut buffer = [0u8; 512];
    let read = holder
        .stdout
        .as_mut()
        .expect("stdout")
        .read(&mut buffer)
        .expect("read holder");
    let output = String::from_utf8_lossy(&buffer[..read]);
    assert!(output.contains("ACQUIRED"), "{output}");

    let busy = Command::new(probe()).arg(&root).output().expect("challenger");
    let text = String::from_utf8_lossy(&busy.stdout);
    assert!(
        text.contains("BUSY") || busy.status.code() == Some(23),
        "{text} err={}",
        String::from_utf8_lossy(&busy.stderr)
    );
    let _ = holder.kill();
    let _ = holder.wait();
    std::fs::remove_dir(&root).ok();
}
