# gogoke

This repository contains the gogoke desktop source and is the official release channel. The source is licensed under MIT; bundled third-party code retains the licenses and notices listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Official Windows releases are published through this repository's [Releases](https://github.com/taiyun668/gogoke/releases) page. The installer is currently not code signed. Windows Smart App Control may block an unsigned installer or a child executable; do not disable system security to install gogoke. Release integrity is checked with an Owner offline-signed `SHA256SUMS.windows` manifest, as described in [the release integrity documentation](apps/desktop/docs/release-integrity.md).

Native code is built and tested in cloud CI. The build and platform policy is in [docs/governance/gogoke-build-and-release.md](docs/governance/gogoke-build-and-release.md). S1-R4 construction gates and live model activation retain their separate authorization requirements.
Installer artifacts must be built by the cloud desktop workflow: it generates the locked production dependency notices, bundles them, and checks the installed copies. A local source checkout alone is not a complete installer build input.
