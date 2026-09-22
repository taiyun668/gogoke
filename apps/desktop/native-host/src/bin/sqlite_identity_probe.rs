fn main() {
    match gogoke_native_host::store::sqlite_identity::verify_sqlite_build_identity() {
        Ok(identity) => {
            println!(
                "sqlite_version={}\nsource_id={}\nthreadsafe={}\nsqlite3_c_sha256={}\nsqlite3_h_sha256={}\nsqlite3_patch_sha256={}\nsqlite3_generated_c_sha256={}",
                identity.version,
                identity.source_id,
                identity.threadsafe,
                identity.sqlite3_c_sha256,
                identity.sqlite3_h_sha256,
                identity.sqlite3_patch_sha256,
                identity.sqlite3_generated_c_sha256
            );
        }
        Err(error) => {
            eprintln!("SQLITE_IDENTITY_REJECTED: {error}");
            std::process::exit(1);
        }
    }
}
