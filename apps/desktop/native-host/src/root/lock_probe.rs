#[allow(dead_code, unused_imports)]
#[path = "mod.rs"]
mod root;

use root::{RootLock, RootLockError};
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(first) = args.next() else {
        eprintln!(
            "usage: lock_probe <absolute-root> [hold-ms] | --pin-db|--create-pin-db <root> <db> [hold-ms]"
        );
        std::process::exit(2);
    };
    if first == "--pin-db" || first == "--create-pin-db" {
        let create_new = first == "--create-pin-db";
        let (Some(root_path), Some(database_path)) = (args.next(), args.next()) else {
            eprintln!(
                "usage: lock_probe --pin-db|--create-pin-db <absolute-root> <absolute-db> [hold-ms]"
            );
            std::process::exit(2);
        };
        let hold_ms = parse_hold_ms(args.next());
        match RootLock::acquire(Path::new(&root_path)) {
            Ok(lock) => match if create_new {
                lock.create_and_pin_database(Path::new(&database_path))
            } else {
                lock.pin_existing_database(Path::new(&database_path))
            } {
                Ok(pin) => {
                    println!(
                        "DB_PINNED\t{}\t{}\tcreated_new={}\tnon_inheritable={}",
                        pin.identity().opaque(),
                        pin.path().display(),
                        pin.created_new(),
                        !pin.handle_inheritable().unwrap_or(true)
                    );
                    io::stdout().flush().expect("flush database pin evidence");
                    std::thread::sleep(Duration::from_millis(hold_ms));
                }
                Err(error) => {
                    eprintln!("ERROR\t{error}");
                    std::process::exit(1);
                }
            },
            Err(error) => {
                eprintln!("ERROR\t{error}");
                std::process::exit(1);
            }
        }
        return;
    }

    let path = first;
    let hold_ms = parse_hold_ms(args.next());

    match RootLock::acquire(Path::new(&path)) {
        Ok(lock) => {
            let canonical = lock.canonical_root();
            let non_inheritable = lock.handles_are_non_inheritable().unwrap_or(false);
            println!(
                "ACQUIRED\t{}\t{}\tnon_inheritable={non_inheritable}",
                canonical.identity.opaque(),
                canonical.canonical_path.display()
            );
            io::stdout().flush().expect("flush acquisition evidence");
            if hold_ms > 0 {
                std::thread::sleep(Duration::from_millis(hold_ms));
            }
        }
        Err(RootLockError::AlreadyLocked { identity }) => {
            println!("BUSY\t{}", identity.opaque());
            std::process::exit(23);
        }
        Err(error) => {
            eprintln!("ERROR\t{error}");
            std::process::exit(1);
        }
    }
}

fn parse_hold_ms(value: Option<std::ffi::OsString>) -> u64 {
    value
        .and_then(|value| value.to_string_lossy().parse::<u64>().ok())
        .unwrap_or(0)
}
