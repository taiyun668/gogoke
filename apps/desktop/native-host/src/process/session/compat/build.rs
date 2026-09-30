use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const VENDOR: &str = "vendor/detours-4.0.1";
const SOURCE_FILES: &[&str] = &[
    "detours.cpp", "modules.cpp", "disasm.cpp", "image.cpp", "creatwth.cpp",
    "disolx86.cpp", "disolx64.cpp", "disolia64.cpp", "disolarm.cpp",
    "disolarm64.cpp",
];

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap_or_else(|e|
        panic!("read {}: {e}", path.display()))))
}

fn verify_vendor() {
    assert_eq!(sha256(Path::new("VENDOR_SHA256SUMS")),
        "ff5cf70c47ace5cf45966648f1dab9bd24b50cc7d6853f46ab5575893c43881f",
        "fixed Detours source list changed");
    let manifest = fs::read_to_string("VENDOR_SHA256SUMS")
        .expect("fixed Detours source hash list");
    for line in manifest.lines() {
        let (expected, relative) = line.split_once("  ").expect("vendor hash list format");
        let path = Path::new(relative);
        assert!(relative.starts_with("vendor/detours-4.0.1/") &&
            !relative.contains("..") && path.is_file(), "invalid vendor hash path");
        assert_eq!(sha256(path), expected, "vendored source changed: {relative}");
        println!("cargo:rerun-if-changed={relative}");
    }
    println!("cargo:rerun-if-changed=VENDOR_SHA256SUMS");
}

fn main() {
    assert_eq!(std::env::var("TARGET").unwrap(), "x86_64-pc-windows-msvc",
        "LPAC compatibility is fixed to native Windows x64");
    verify_vendor();
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let vendor_src = Path::new(VENDOR).join("src");
    let mut detours = cc::Build::new();
    detours.cpp(true).static_crt(true).opt_level(2).debug(false)
        .include(&vendor_src).define("WIN32_LEAN_AND_MEAN", None)
        .define("_WIN32_WINNT", "0x0A00")
        .flag("/Brepro").flag("/Gy").warnings(false)
        .cargo_metadata(false);
    for source in SOURCE_FILES { detours.file(vendor_src.join(source)); }
    detours.compile("gogoke_detours");

    // Build the DLL from the same fixed Detours library. The host embeds these
    // bytes; no build-time or run-time download, caller DLL path, or shim swap.
    let module = out.join("gogoke_lpac_path_compat.dll");
    let import_lib = out.join("gogoke_lpac_path_compat.lib");
    let object = out.join("gogoke_lpac_path_compat.obj");
    let vendor_lib = out.join("gogoke_detours.lib");
    assert!(vendor_lib.is_file(), "Detours static library missing");
    let mut compiler = cc::Build::new();
    compiler.cpp(true).static_crt(true).opt_level(2).debug(false)
        .include(&vendor_src).define("_WIN32_WINNT", "0x0A00")
        .flag("/Brepro").flag("/Gy").warnings(false);
    let tool = compiler.get_compiler();
    let mut command = tool.to_command();
    command.arg("/nologo").arg("/LD").arg("/Brepro")
        .arg("/EHsc").arg("/MT").arg("/O2")
        .arg(format!("/I{}", vendor_src.display()))
        .arg(format!("/Fo{}", object.display()))
        .arg("native/shim.cpp")
        .arg("/link").arg("/Brepro").arg("/INCREMENTAL:NO")
        .arg("/DEBUG:NONE").arg("/OPT:REF").arg("/OPT:ICF")
        .arg(format!("/OUT:{}", module.display()))
        .arg(format!("/IMPLIB:{}", import_lib.display()))
        .arg("/DEF:native/shim.def")
        .arg(&vendor_lib).arg("kernel32.lib").arg("user32.lib")
        .arg("advapi32.lib").arg("imagehlp.lib");
    let status = command.status().expect("launch MSVC cloud compiler for fixed DLL");
    assert!(status.success(), "fixed LPAC path DLL build failed: {status}");
    assert!(module.is_file(), "fixed LPAC path DLL absent after build");
    // Cargo build scripts do not receive a dependable test cfg. Build the
    // same-source cloud test executable into OUT_DIR on every profile; only
    // crate tests execute it. It is never linked or embedded in the product.
    let test_exe = out.join("gogoke_lpac_path_shim_test.exe");
    let test_object = out.join("gogoke_lpac_path_shim_test.obj");
    let mut tests = tool.to_command();
    tests.arg("/nologo").arg("/Brepro").arg("/EHsc").arg("/MT").arg("/O2")
        .arg(format!("/I{}", vendor_src.display()))
        .arg(format!("/Fo{}", test_object.display()))
        .arg("native/shim_test.cpp")
        .arg("/link").arg("/Brepro").arg("/INCREMENTAL:NO")
        .arg("/DEBUG:NONE").arg(format!("/OUT:{}", test_exe.display()))
        .arg("kernel32.lib");
    let status = tests.status().expect("launch MSVC cloud compiler for shim test");
    assert!(status.success(), "fixed LPAC path shim test build failed: {status}");
    assert!(test_exe.is_file(), "fixed LPAC path shim test absent after build");
    let digest = sha256(&module);
    println!("cargo:rustc-env=GOGOKE_LPAC_PATH_SHIM_SHA256={}",
        sha256(Path::new("native/shim.cpp")));
    println!("cargo:rustc-env=GOGOKE_LPAC_PATH_MODULE={}", module.display());
    println!("cargo:rustc-env=GOGOKE_LPAC_PATH_MODULE_SHA256={digest}");
    println!("cargo:rustc-env=GOGOKE_LPAC_PATH_TEST_EXE={}", test_exe.display());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=gogoke_detours");
    println!("cargo:rerun-if-changed=native/shim.cpp");
    println!("cargo:rerun-if-changed=native/shim.def");
    println!("cargo:rerun-if-changed=native/shim_test.cpp");
}
