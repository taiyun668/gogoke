# gogoke 0.1.2 Windows runtime verification — 2026-09-15

## Release and artifact

- Source candidate: `50a1c4f45bdd9d20e5e5c649a25e3ea6d148f10f`.
- GitHub Actions run `35017346806`: browser tests, Windows build, updater tests,
  full update/readiness/rollback smoke, and formal artifact upload all passed.
- Published release: `https://github.com/taiyun668/gogoke/releases/tag/v0.1.2`.
- Anonymous latest-release API returned `v0.1.2`, non-draft, non-prerelease,
  with exactly the installer, portable ZIP, checksum manifest, and detached
  manifest signature.
- Installer SHA-256:
  `ad3ae37c4a936b5f7344114c3a0bbed4b32f53ceff49b2f0b5e7f61fec9a8a05`.
- Portable ZIP SHA-256:
  `5562bdfb3e3f4309475a64b818b94873f9068b7f0ec14497b7a3a0253766755e`.
- Installer Authenticode status: `NotSigned`. The release manifest was signed
  outside the repository and verified against the public key embedded in gogoke.

## Bootstrap installation

The installed 0.1.0 client could read the GitHub release feed but rejected the
asset redirect with `release request returned HTTP 302 Found`. The observed
redirect host was `release-assets.githubusercontent.com`; 0.1.0 did not include
that exact official host in its allowlist. Candidate 0.1.2 adds that exact HTTPS
host while retaining exact-host matching and the five-redirect limit.

Because 0.1.0 could not download the updater that fixes its own allowlist, the
Owner authorized one manual bootstrap installation. The verified 0.1.2 NSIS
installer replaced the running application at the existing path:
`%USERPROFILE%\AppData\Local\Programs\gogoke\gogoke.exe`.

## Runtime behavior

- About view reported version `0.1.2`, branch `codex/gogoke-shell`, and commit
  `50a1c4f45bdd`.
- The interface was switched to Simplified Chinese and rendered Chinese text in
  the sidebar, home, Settings, Display & Sound, and About views.
- The actual process path matched the installed path above.
- The desktop shortcut targets the same installed executable.
- The uninstaller exists and the HKCU uninstall registration reports version
  `0.1.2` with the expected uninstall path.
- DWM attribute 38 returned `systemBackdrop=3`, HRESULT 0; the glass background
  remained visibly present.
- Installed executable Authenticode status is `NotSigned`.
- No `gogoke.update-backup*` directory or `update-failure*.log` remained.

The installed NSIS executable and portable executable have equal length but
different hashes in exactly three bytes. The differing embedded marker is
`__TAURI_BUNDLE_TYPE` (`UNK` in portable, `NSS` in the NSIS-installed binary),
so portable-byte equality is not used as an installation-integrity substitute.

Language persistence across a second manual restart was not independently
verified because concurrent user interaction interrupted the close action.
The live installed application remained in Simplified Chinese.

## Boundaries

This verifies the 0.1.2 release artifact, manual bootstrap installation, runtime
identity, localization, glass behavior, and installed registration. It does not
claim that 0.1.0 performed an in-app installation; that path was impossible due
to its missing redirect host. Future higher-version releases can exercise the
corrected in-app download path from 0.1.2.
