#[cfg(windows)]
use gogoke_native_host::root::RootLock;
#[cfg(windows)]
use gogoke_native_host::store::same_open::{create_new, open_ledger};
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
fn main() {
    if let Err(error) = run() {
        eprintln!("ROUTE_B_SAME_OPEN_REJECTED: {error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let Some(root_path) = args.next() else {
        eprintln!("usage: sqlite_same_open_probe <existing-absolute-scratch-root>");
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("usage: sqlite_same_open_probe <existing-absolute-scratch-root>");
        std::process::exit(2);
    }

    let root_path = PathBuf::from(root_path);
    let root_lock = RootLock::acquire(&root_path)?;
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let database = root_lock
        .canonical_root()
        .canonical_path
        .join(format!("route-b-b1-{nonce}.db"));
    let replacement = database.with_extension("replacement.db");
    let renamed = database.with_extension("renamed.db");
    let _ = std::fs::remove_file(&replacement);
    let _ = std::fs::remove_file(&renamed);

    let mut connection = create_new(&root_lock, &database)?;
    connection.execute(
        "PRAGMA foreign_keys=ON;\
         PRAGMA synchronous=FULL;\
         PRAGMA journal_mode=WAL;\
         CREATE TABLE probe(value INTEGER NOT NULL);\
         BEGIN IMMEDIATE;\
         INSERT INTO probe(value) VALUES (42);\
         COMMIT;\
         PRAGMA wal_checkpoint(TRUNCATE);",
    )?;
    println!(
        "SAME_OPEN_OK\tpath={}\tcreated_new={}\tidentity={}",
        connection.path().display(),
        connection.created_new(),
        connection.identity().opaque()
    );

    std::fs::write(&replacement, b"replacement").expect("scratch replacement file");
    let rename_denied = std::fs::rename(&database, &renamed).is_err();
    let delete_denied = std::fs::remove_file(&database).is_err();
    let replace_denied = std::fs::rename(&replacement, &database).is_err();
    println!(
        "PATH_PROTECTION\trename_denied={rename_denied}\tdelete_denied={delete_denied}\treplace_denied={replace_denied}"
    );
    if !(rename_denied && delete_denied && replace_denied) {
        return Err("live SQLite handle did not retain all path protections".into());
    }

    let ledger_before_close = open_ledger()?;
    println!(
        "OPEN_LEDGER\tadopted={}\tstock_main_open={}\trejected={}",
        ledger_before_close.adopted,
        ledger_before_close.stock_main_open,
        ledger_before_close.rejected
    );
    if ledger_before_close.stock_main_open != 0 || ledger_before_close.rejected != 0 {
        return Err(
            "Route-B strict open used a stock main open or rejected an admitted handoff".into(),
        );
    }
    let ledger_after_close = connection.close_checked()?;
    println!(
        "CLOSE_LEDGER\tbefore_adopted={}\tafter_adopted={}\tclose_success={}\tclose_unknown={}\tlast_generation={}",
        ledger_before_close.adopted,
        ledger_after_close.adopted,
        ledger_after_close.close_success,
        ledger_after_close.close_unknown,
        ledger_after_close.last_closed_generation
    );
    if ledger_after_close.adopted != ledger_before_close.adopted
        || ledger_after_close.close_success <= ledger_before_close.close_success
        || ledger_after_close.close_unknown != ledger_before_close.close_unknown
    {
        return Err("same-open or checked-close ledger did not advance".into());
    }

    std::fs::rename(&database, &renamed)?;
    std::fs::rename(&replacement, &database)?;
    std::fs::remove_file(&database)?;
    std::fs::remove_file(&renamed)?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ROUTE_B_SAME_OPEN_UNAVAILABLE: Windows VFS probe requires Windows");
    std::process::exit(77);
}
