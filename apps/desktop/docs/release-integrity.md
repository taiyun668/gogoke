# gogoke Windows release integrity

gogoke publishes unsigned Windows binaries, while automatic updates use a
separate Owner-controlled integrity chain. Windows publisher reputation and
gogoke release authorization are different questions.

CI produces a resource pack and its byte index for every release. A `full`
release also contains the installer and portable ZIP; a `resources` release
contains only the resource pack and index. The Owner-signed
`SHA256SUMS.windows` manifest identifies the release type and version and
contains exactly the corresponding asset set:

- `full`: `gogoke-<version>-windows-x64-unsigned-setup.exe`,
  `gogoke-<version>-windows-x64-unsigned-portable.zip`,
  `gogoke-resources.windows.zip`, and `resource-index.json`
- `resources`: `gogoke-resources.windows.zip` and `resource-index.json`

Manifest headers are exactly `# gogoke-Version: <version>` and
`# gogoke-Release-Type: full|resources`; each checksum line uses a lowercase
SHA-256, two spaces, the exact asset filename, and an LF ending. Duplicate,
missing, mixed-type, and extra entries are rejected.

Before a GitHub release is published, the Owner signs the checksum manifest
offline with `tools/sign-gogoke-release-manifest.ps1`. The private P-256 key
lives at `%USERPROFILE%\.gogoke\release-key.txt` by default and must never
enter the repository, build tree, CI, or release assets. Only the public half
is compiled into the application.

`tools/publish-gogoke-release.ps1` validates the exact CI artifacts,
recomputes their hashes, verifies the resource pack against its index, signs
the manifest, and verifies that signature against the public key embedded in
the application. Use `-ReleaseType full` (the default) for a complete install
release or `-ReleaseType resources` for a resource-only release. Its default
mode stops after signing and verification. Publishing to GitHub still
requires the explicit `-Publish` switch and a release-notes file; it publishes
only the asset set selected by the signed manifest.

The application checks only `taiyun668/gogoke`. It accepts one newer,
non-draft, non-prerelease version with one exactly named installer, one checksum
manifest, and one detached signature. The signature binds the manifest to the
Owner; the manifest binds the version and installer SHA-256. The installer is
downloaded into the application cache and hashed before an update is offered.
After confirmation, the application refreshes the signed release identity,
rehashes the cached installer, starts an isolated update coordinator, and
exits. The coordinator holds an OS file lock, waits for the old process,
moves the complete previous install aside, starts the visible installer, and
launches the new executable. The React application writes a version-bound
readiness receipt after its first render. Only then is the backup removed.
Installer failure, early process exit, a wrong version, or a missing readiness
receipt restores the old install and records a bounded failure log. A portable
copy uses the same path to create a clean per-user installation.

A missing release, signature, or exact installer is not an update. Redirects
and downloads are limited to the explicit GitHub publication hosts and bounded
by size. Browser previews cannot check or install desktop updates.

Application data is stored under the independent `app.gogoke.desktop`
identity, outside the installation directory. Replacing or rolling back
program files therefore does not move or delete project data and settings.
