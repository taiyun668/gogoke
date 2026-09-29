//! Directed cloud-only observation of the actual pinned CLI. This module is
//! absent from release builds. It never changes the target image, token or ACL.
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
    let Some(debugger) = std::env::var_os("GOGOKE_CI_CDB") else { return None; };
    let evidence = std::path::PathBuf::from(std::env::var_os("GOGOKE_CI_CDB_LOGROOT")
        .expect("directed debugger evidence root"));
    let log = evidence.join(format!("cli-finalpath-{}-{}.log", prepared.identity.pid,
        prepared.identity.creation_time_100ns));
    let mut output = std::fs::OpenOptions::new().create_new(true).write(true)
        .open(log).expect("exclusive directed debugger log");
    // Observe the real GetFinalPathNameByHandleW entry and its caller return.
    // Software breakpoints change memory and timing for measurement, not the
    // on-disk CLI bytes, its token or ACL. This is not product acceptance.
    // Continue only the loader breakpoint observed in the first cloud trace.
    let commands = r#"sxe -c ".if (@rip == ntdll!LdrpDoDebuggerBreak+0x35) { .echo GOGOKE_LOADER_BREAK_CONTINUE; gh }" bpe; bp KERNELBASE!GetFinalPathNameByHandleW ".printf \"GOGOKE_FINALPATH_ENTRY tid=%x flags=%x handle=%p\\n\", @$tid, @r9, @rcx; !handle @rcx f; ~.bp /1 poi(@rsp) \".printf \\\"GOGOKE_FINALPATH_RETURN tid=%x value=%x\\\\n\\\", @$tid, @rax; !gle; gc\"; gc"; .echo GOGOKE_CDB_READY; g"#;
    let mut child = Command::new(debugger).args(["-G", "-pd", "-p", &prepared.identity.pid.to_string(),
        "-c", commands]).stdin(Stdio::null()).stdout(Stdio::piped())
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
