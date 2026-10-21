# Release manifest v1

`latest.txt` is UTF-8 text, one `key=value` per line, with a final LF.
`scripts/release/publish-macos.sh` writes it after copying packages and the installer.
Each file is published by rename. Older four-key manifests remain readable;
the extended format adds provenance and installer verification.

| Key | Source and meaning | Reader |
|---|---|---|
| `version` | Publisher argument, checked against `build-manifest.txt`; SemVer | App (`orbit_core::update::ReleaseManifest`) and `install.sh` |
| `published` | Publisher UTC, `YYYY-MM-DDTHH:MM:SSZ` | Informational; updaters ignore unread keys |
| `<arch>_file` | Exact verified package name `Orbit-<version>-macos-<arch>.zip` | `install.sh` downloads the selected architecture |
| `<arch>_sha256` | Publisher-measured ZIP SHA-256, 64 lowercase hex digits | `install.sh` verifies the download |
| `source_commit` | Validated 40-hex commit from `build-manifest.txt` | Operator/reviewer traceability; older clients ignore it |
| `installer_sha256` | SHA-256 of `install.sh` copied from `source_commit` by `prepare-publish.sh` and remeasured at publication | App (`crates/orbit-app-core/src/update/download.rs`) and `scripts/release/install-on-mac.sh` verify before execution; older clients ignore it |

`<arch>` is `arm64` or `x86_64`; include only packages present in the version
directory. The ZIP and installer belong to the same commit. These hashes alone
prove neither signing nor notarization.
Example: [manifest fixture](fixtures/release-manifest/latest-with-provenance.txt).
