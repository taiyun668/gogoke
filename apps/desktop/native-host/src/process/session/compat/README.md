# Fixed LPAC compatibility and Claude startup pipe mapping

The path mode is only for the native x64 Codex CLI 0.160.0 launched by H inside
its verified LPAC profile. The official npm platform archive binary is PE machine `0x8664`,
SHA-256 `fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d`,
and imports `GetFinalPathNameByHandleW` once from `kernel32.dll`. This is a
read-only archive observation, not an installed launch result; the host still performs
its own program identity checks at every launch. The original 0.149.0 failure
was flags `0` returning `0` / Win32 `5` after an LPAC-denied open of
`\??\MountPointManager`, although the exact F home metadata handle opened.

The same embedded DLL has a separate pipe mode for the exact Claude
2.1.196 x64 image, SHA-256
`180d7b279455e8b89d4353a5146447be2f80b80fb0db14bdc6dd9cb98c0aef09`.
H constructs `GOGOKE_LPAC_COMPAT_MODE=CLAUDE_PIPE_V1` only for that pinned
digest and checks the actual suspended child digest again before injection.
The DLL requires one each of `CreateNamedPipeA`, `CreateNamedPipeW`, and
`CreateFileA` main-image imports from `kernel32.dll`, then patches only those
three exact IAT slots. Codex keeps its existing path mode with no mode
variable. The host does not accept a
caller or inherited mode value and supplies no Codex path mappings to Claude.

The fixed libuv `uv__unique_pipe_name` creates a private pair name in a 64-byte
buffer as `\\?\pipe\uv\<decimal>-<pid>`. Only an exact decimal form ending in
the current process ID, with libuv's fixed server or client call parameters,
maps to `\\.\pipe\LOCAL\uv\<same suffix>`. The `CreateNamedPipeA` server
and its following `CreateFileA` client receive the same mapped name. Every
other argument and each API's return handle and LastError pass through. The
`CreateNamedPipeW` wrapper remains observation-only; other pipe names and
calls remain unmapped. No retry, wait, CLI byte change, token/Job change, pipe
ACL change, or additional capability is introduced.
[Microsoft's IPC guidance](https://learn.microsoft.com/en-us/windows/apps/develop/communication/interprocess-communication)
states that `LOCAL` limits the name to the login session; the name is not an
authorization check. Both mapped ends are in the same verified LPAC process,
not a User helper.

On the first failed matching `uv` `CreateNamedPipeA` or W call per process,
the wrapper emits one stderr line with the API, Win32 error and prefix class.
It never emits the pipe name, payload, credential or private path. Stderr
writing cannot replace the API's LastError. Subsequent failures are not logged.
Loading the host-owned embedded DLL for Claude uses the existing exact-file
LPAC read/execute grant and verification. That grant writes the DLL ACL for
the Claude profile; no CLI, worktree, or pipe object rights are added. The
existing module custody retains the file across child lifetime. The existing
module grant cleanup is a residual outside this observation repair; no
profile-specific RX withdrawal at stop or release is claimed.

The target follows the fixed-image PE metadata and the pinned
[oven-sh/libuv Windows pipe implementation](https://github.com/oven-sh/libuv/blob/4dcfac4780d394e0dc2d3fb30335ca01b553eb46/src/win/pipe.c),
whose `uv__pipe_server` retries `ERROR_PIPE_BUSY` and `ERROR_ACCESS_DENIED`
after `CreateNamedPipeA`, then opens the same pair name with `CreateFileA`.
The original installed 0.1.38 attempts had no stdout, stderr or debug file;
sampled syscall PCs were not returned error codes. The installed 0.1.49
attempt recorded a first matching A failure with Win32 `5`, followed by an
initialize frame deadline. That first error does not establish the return
of every later retry or exclude a first-instance name collision. The mapping
follows [Microsoft's documented AppContainer `\\.\pipe\LOCAL\` syntax](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea);
installed same-byte startup must still establish the outcome.

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
`GetFinalPathNameByHandleW` in Codex path mode, or the three named imports above
in Claude pipe mode, with no duplicate or foreign-DLL match. It patches only
those main-image IAT slots, preserving original pointers; it does not patch
system DLL code or scan other modules. It creates no thread, loads no library,
and waits for nothing from `DllMain`. The host uses
`DetourUpdateProcessWithDll` on the verified suspended image, so the injected
DLL is a startup import. A missing slot or partial IAT patch makes `DllMain`
return `FALSE`: loader initialization fails and the CLI cannot continue. This
startup failure is the fail-closed condition; no runtime patch rollback is
claimed.

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
the same-source fixture also checks Claude argument/return/LastError
passthrough, identical mapped A server/client names, unrelated calls, bounded
stderr text and exact A/W/`CreateFileA` import shape.
The installed CLI remains the definitive behavior check.

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
