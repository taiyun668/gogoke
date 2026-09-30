//! Directed cloud-only observation of the actual pinned CLI. This module is
//! absent from release builds. The on-disk CLI bytes, token and ACL are retained;
//! software breakpoints change execution memory and timing for measurement.
use crate::process::PreparedCustody;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub(super) struct CliTrace {
    debugger: Child,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Drop for CliTrace {
    fn drop(&mut self) {
        // The fixture's own Job is responsible for its process. Terminate only
        // this measurement process if it did not exit with the target.
        let exited = match self.debugger.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => {
                if let Err(error) = self.debugger.kill() { eprintln!("CDB cleanup: {error}"); }
                match self.debugger.wait() {
                    Ok(_) => true,
                    Err(error) => { eprintln!("CDB wait: {error}"); false }
                }
            }
            Err(error) => { eprintln!("CDB status: {error}"); false }
        };
        if exited {
            if let Some(reader) = self.reader.take() {
                if reader.join().is_err() { eprintln!("CDB evidence reader panicked"); }
            }
        }
    }
}

pub(super) fn before_activation(prepared: &PreparedCustody) -> Option<CliTrace> {
    before_identity(&prepared.identity)
}

pub(super) fn before_identity(identity: &crate::process::ProcessIdentity) -> Option<CliTrace> {
    let Some(debugger) = std::env::var_os("GOGOKE_CI_CDB") else { return None; };
    let evidence = std::path::PathBuf::from(std::env::var_os("GOGOKE_CI_CDB_LOGROOT")
        .expect("directed debugger evidence root"));
    let log = evidence.join(format!("cli-state-{}-{}.log", identity.pid,
        identity.creation_time_100ns));
    let mut output = std::fs::OpenOptions::new().create_new(true).write(true)
        .open(log).expect("exclusive directed debugger log");
    // Persistent syscall-return sites avoid removing a caller-return INT3
    // while another CLI thread has already reached that same instruction.
    // Validate the actual x64 syscall/ret bytes before placing either site;
    // an unfamiliar stub is an instrument failure, never a product finding.
    // This is direct metadata/error capture, not product acceptance. It does
    // not inspect any file contents, credential bytes or query buffers.
    let mut nt_calls = String::new();
    let methods = [
        ("NtOpenFile", r#".printf \"GOGOKE_NT_ENTRY NtOpenFile tid=%x access=%x options=%x name=%msu\\n\", @$tid, @rdx, dwo(@rsp+0x30), poi(@r8+0x10); .if (@rdx == 0x100000) { k 8; }"#),
        ("NtDeleteFile", r#".printf \"GOGOKE_NT_ENTRY NtDeleteFile tid=%x name=%msu\\n\", @$tid, poi(@rcx+0x10);"#),
        ("NtCreateFile", r#".printf \"GOGOKE_NT_ENTRY NtCreateFile tid=%x access=%x disposition=%x options=%x name=%msu\\n\", @$tid, @rdx, dwo(@rsp+0x40), dwo(@rsp+0x48), poi(@r8+0x10);"#),
        ("NtQueryAttributesFile", r#".printf \"GOGOKE_NT_ENTRY NtQueryAttributesFile tid=%x name=%msu\\n\", @$tid, poi(@rcx+0x10);"#),
        ("NtQueryFullAttributesFile", r#".printf \"GOGOKE_NT_ENTRY NtQueryFullAttributesFile tid=%x name=%msu\\n\", @$tid, poi(@rcx+0x10);"#),
        ("NtSetInformationFile", r#".printf \"GOGOKE_NT_ENTRY NtSetInformationFile tid=%x handle=%p class=%x\\n\", @$tid, @rcx, dwo(@rsp+0x28);"#),
        ("NtCreateSection", r#".printf \"GOGOKE_NT_ENTRY NtCreateSection tid=%x access=%x protection=%x allocation=%x file=%p\\n\", @$tid, @rdx, dwo(@rsp+0x28), dwo(@rsp+0x30), poi(@rsp+0x38);"#),
        ("NtMapViewOfSection", r#".printf \"GOGOKE_NT_ENTRY NtMapViewOfSection tid=%x section=%p process=%p\\n\", @$tid, @rcx, @rdx;"#),
    ];
    for (method, entry) in methods {
        nt_calls.push_str(&format!(r#".if ((wo(ntdll!{method}+0x12) != 0x050f) or (by(ntdll!{method}+0x14) != 0xc3) or (wo(ntdll!{method}+0x15) != 0x2ecd) or (by(ntdll!{method}+0x17) != 0xc3)) {{ .echo GOGOKE_CDB_UNSUPPORTED_SYSCALL_STUB; qd }}
u ntdll!{method} L10
bp ntdll!{method} "{entry} gc"
bp ntdll!{method}+0x14 ".printf \"GOGOKE_NT_RETURN {method} tid=%x status=%x\\n\", @$tid, @rax; gc"
bp ntdll!{method}+0x17 ".printf \"GOGOKE_NT_RETURN {method} tid=%x status=%x\\n\", @$tid, @rax; gc"
"#));
    }
    let commands = format!(r#"sxe -c ".if (@rip == ntdll!LdrpDoDebuggerBreak+0x35) {{ .echo GOGOKE_LOADER_BREAK_CONTINUE; gh }}" bpe
{nt_calls}bp KERNELBASE!DeleteFileW ".printf \"GOGOKE_DELETE_ENTRY tid=%x name=%mu\\n\", @$tid, @rcx; k 8; gc"
.echo GOGOKE_CDB_READY
g
"#);
    // The debugger's initial -c command has a bounded line size. A command
    // file keeps each guarded setup/breakpoint command on its own short line.
    let command_path = evidence.join(format!("cli-state-commands-{}-{}.txt",
        identity.pid, identity.creation_time_100ns));
    let mut command_file = std::fs::OpenOptions::new().create_new(true).write(true)
        .open(&command_path).expect("exclusive direct-error command file");
    command_file.write_all(commands.as_bytes()).and_then(|_| command_file.sync_all())
        .expect("flush direct-error commands before attachment");
    let mut child = Command::new(debugger).args(["-G", "-pd", "-p", &identity.pid.to_string(),
        "-cf"]).arg(&command_path).stdin(Stdio::null()).stdout(Stdio::piped())
        .stderr(Stdio::inherit()).spawn().expect("attach existing SDK debugger to exact fixture PID");
    let stdout = child.stdout.take().expect("directed debugger stdout");
    let (ready, waiting) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = match line { Ok(line) => line, Err(error) => {
                eprintln!("CDB output read: {error}"); break;
            } };
            if let Err(error) = writeln!(output, "{line}").and_then(|_| output.flush()) {
                eprintln!("CDB evidence write: {error}"); break;
            }
            if line.trim() == "GOGOKE_CDB_READY" {
                if let Err(error) = ready.send(()) { eprintln!("CDB readiness: {error}"); }
            }
        }
    });
    let trace = CliTrace { debugger: child, reader: Some(reader) };
    waiting.recv_timeout(Duration::from_secs(20)).expect("actual debugger attachment readiness");
    Some(trace)
}
