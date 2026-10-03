# Fixed LPAC path compatibility package

This package is only for the native x64 Codex CLI 0.160.0 launched by H inside
its verified LPAC profile. The official npm platform archive binary is PE machine `0x8664`,
SHA-256 `fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d`,
and imports `GetFinalPathNameByHandleW` once from `kernel32.dll`. This is a
read-only archive observation, not an installed launch result; the host still performs
its own program identity checks at every launch. The original 0.149.0 failure
was flags `0` returning `0` / Win32 `5` after an LPAC-denied open of
`\??\MountPointManager`, although the exact F home metadata handle opened.

The DLL is built at cloud build time and embedded in Rust as `MODULE_BYTES`.
`MODULE_SHA256`, `SHIM_SOURCE_SHA256`, and `DETOURS_COMMIT` give the host and
diagnostics the byte and source provenance. The host must materialize exactly
those bytes in its owned private directory, verify the hash and file identity,
hold a delete-blocking handle through the child lifetime, and grant that exact
file LPAC read/execute. It must not accept DLL bytes or a path from IPC/config.
Smart App Control on Owner Windows 11 may reject this new DLL independently;
the installed product test remains required, and a rejection is a launch
failure rather than a reason to change system security settings.

Root integration:

1. Add a Windows-target path dependency `gogoke-lpac-path-compat` to the
   native-host Cargo manifest. This package has its own `build.rs`; the host
   build script is not part of this change.
2. For the exact verified F home, derive NT and canonical DOS final path from
   the same held home directory handle in the unrestricted host. Put these as
   `GOGOKE_LPAC_PATH_NT_ROOT` and `GOGOKE_LPAC_PATH_DOS_ROOT` into the child's
   explicitly constructed environment. They must refer to the same verified
   object and must have no trailing separator. Do not accept values from
   process inheritance or protocol input.
3. After suspended creation, exact image/token/Job verification and before
   prepared commitment, call `module_path_ansi(&held_module_path)` and then
   `unsafe { update_suspended(process_handle, &module_path) }`. The latter
   checks same native x64 bitness and invokes only
   `DetourUpdateProcessWithDll`; it never creates, resumes or redirects a
   process. A nonrepresentable ANSI path fails closed. The host owns abort,
   preparation, durable commitment, activation and file custody.

On normal startup the injected DLL calls `DetourRestoreAfterWith` and checks
the main image PE import directory. It refuses loader initialization unless
there is exactly one `kernel32.dll` import by name for
`GetFinalPathNameByHandleW` and no import of that name from another DLL. It
patches only that main-image IAT slot, preserving the original pointer; it does
not patch system DLL code or scan other modules. It creates no thread, loads
no library, and waits for nothing from `DllMain`. Any missing import or patch
failure makes the CLI unusable rather than starting with unproved behavior.

The wrapper first calls the original API. Only when flags are `0` and the
original returns `0` with `ERROR_ACCESS_DENIED` does it query the **same
handle** with `VOLUME_NAME_NT`. It accepts an exact NT F-home root or a child
at a component boundary, then substitutes the host's canonical DOS F-home
spelling. All unrelated flags, paths and errors return the original result
and original LastError. A successful mapped call returns the UTF-16 count
excluding NUL and writes a NUL; a zero or short buffer returns the required
count including NUL and `ERROR_INSUFFICIENT_BUFFER`. The mapping is a pathname
compatibility value; the LPAC token and file ACL still decide access.
These return lengths and volume-name forms follow the
[Windows API contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew).

Required cloud validation is the original pinned CLI's empty-home `account
read`, its synthetic credential file create/replacement, small/zero buffer and
unrelated-handle/flags/error cases, missing or tampered DLL and changed IAT
shape denial, and the existing Job/image-pin/prepared-commit sequence. A
cloud build/test is not Owner Windows 11/SAC acceptance. No local native
binary build or run is part of this package.
`cargo test --manifest-path .../compat/Cargo.toml` on cloud Windows runs the
same-source C++ shim test for buffer, mapping, LastError and PE import guards;
the actual CLI tests remain the definitive behavior check.

## Dependency provenance

`vendor/detours-4.0.1/` contains the source files used by the Detours 4.0.1
static library and the upstream MIT `LICENSE.md`, copied unmodified from
Microsoft's [Detours v4.0.1 commit](https://github.com/microsoft/Detours/tree/e4bfd6b03e50de46b47abfbd1e46b384f0c5f833).
Every copied file's SHA-256 is in `VENDOR_SHA256SUMS`; `build.rs` pins that
manifest's digest and verifies every listed file before compiling. The
[official helper overview](https://github.com/microsoft/Detours/wiki/OverviewHelpers)
describes import-table DLL loading, while the
[official API declaration](https://github.com/microsoft/Detours/blob/e4bfd6b03e50de46b47abfbd1e46b384f0c5f833/src/detours.h)
defines `DetourUpdateProcessWithDll` and `DetourRestoreAfterWith`. The
[restore documentation](https://github.com/microsoft/Detours/wiki/DetourRestoreAfterWith)
specifies calling it from DLL `PROCESS_ATTACH`.
