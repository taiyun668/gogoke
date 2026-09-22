use gogoke_native_host::ipc::PrivatePipeListener;
use gogoke_native_host::root::{RootLock, RootLockError};
use gogoke_native_host::store::product_database::ProductDatabase;
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (root, database) = explicit_arguments()?;
    let lock = RootLock::acquire(&root)?;
    let canonical = lock.canonical_root();
    let non_inheritable = lock.handles_are_non_inheritable()?;
    println!(
        "LOCKED\t{}\t{}\t{}\tnon_inheritable={non_inheritable}",
        canonical.requested_path.display(),
        canonical.canonical_path.display(),
        canonical.identity.opaque(),
    );
    io::stdout().flush()?;
    let mut product = ProductDatabase::open(&lock, &database)?;
    let endpoint = format!(
        "store.{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let listener = PrivatePipeListener::bind(&endpoint)?;
    let service_capability = service_capability()?;
    println!("PIPE\t{}", listener.path());
    println!("CAPABILITY\t{service_capability}");
    io::stdout().flush()?;
    let pipe = listener.accept_current_user()?;
    product.serve_authenticated_pipe(&pipe, &service_capability)?;
    product.close_checked().ok();
    drop(lock);
    Ok(())
}

#[cfg(windows)]
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8, length: u32, flags: u32) -> i32;
}

#[cfg(windows)]
fn service_capability() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    // System-preferred OS RNG only; failure has no predictable fallback.
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), bytes.len() as u32, 2) };
    if status != 0 {
        return Err(io::Error::new(io::ErrorKind::Other, "service capability RNG failed"));
    }
    let mut value = String::with_capacity(64);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(value)
}

#[cfg(not(windows))]
fn service_capability() -> io::Result<String> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "native service capability requires Windows"))
}

fn explicit_arguments() -> Result<(PathBuf, PathBuf), RootLockError> {
    let mut args = std::env::args_os().skip(1);
    match (
        args.next(),
        args.next(),
        args.next(),
        args.next(),
        args.next(),
    ) {
        (Some(root_flag), Some(root), Some(db_flag), Some(database), None)
            if root_flag == "--root" && db_flag == "--database" =>
        {
            let root = PathBuf::from(root);
            let database = PathBuf::from(database);
            if !database.starts_with(&root) {
                return Err(RootLockError::MissingRoot);
            }
            Ok((root, database))
        }
        (Some(flag), Some(path), None, None, None) if flag == "--root" => {
            let root = PathBuf::from(path);
            let database = root.join("state.sqlite");
            Ok((root, database))
        }
        _ => Err(RootLockError::MissingRoot),
    }
}

#[allow(dead_code)]
fn database_is_direct_child(root: &Path, database: &Path) -> bool {
    database.parent() == Some(root)
}
