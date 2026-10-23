# Install and use Orbit on macOS

Orbit is a desktop app with a Dock icon and a menu bar icon. Its main process
contains the UI and daemon; the Swift helper and `cua-driver` engine are bundled
children. Apple Silicon and macOS 15 or later are required. There is no Intel
package. “Oye Deimos” requires macOS 26; on macOS 15 use the meeting question
button. Chat, voice and notes need a configured [hub](self-hosting.md).
Recording can continue offline.

## Build a macOS package

Install the development tools listed in the [README](../README.md#build-and-check).
Run the full repository gate before packaging, then the Swift tests on macOS.
The bundle script requires the published `cua-driver` version recorded in
`crates/orbit-daemon/src/cua.rs` (currently 0.28.2):

```sh
ORBIT_CUA_DRIVER_BINARY=/absolute/path/to/cua-driver bash scripts/macos/bundle.sh
```

The default output directory is `dist/`; the script refuses to overwrite an
existing package. `ORBIT_OUTPUT_DIR` selects another output directory.
`ORBIT_VERSION` overrides the package version and must be numeric `major.minor.patch`.

| Variable | Meaning |
|---|---|
| `ORBIT_CUA_DRIVER_BINARY` | Absolute path to the required engine binary |
| `ORBIT_SIGN_IDENTITY` | Signing identity; defaults to `-` for ad hoc development signing |
| `ORBIT_SIGN_KEYCHAIN` | Optional signing keychain |
| `ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE` | File supplying its password |
| `ORBIT_NOTARY_KEY` | App Store Connect API private key file |
| `ORBIT_NOTARY_KEY_ID` | API key ID |
| `ORBIT_NOTARY_ISSUER` | API issuer ID |
| `ORBIT_PRUEBAS=1` | Build the separate “Orbit Pruebas” acceptance app |

All three notarization variables must be supplied together. An ad hoc signature
is not a notarized distribution and does not establish Gatekeeper acceptance.
For your own distribution, choose your own signing identity and app identifiers;
review `crates/orbit-core/src/orbit_bundle.rs` and the bundle script. Keep signing
keys outside the repository.

Build-time update-origin and suggested-hub configuration is described in
[self-hosting](self-hosting.md#environment-and-configuration). A build without
an update origin must not assume a download host.

## Install a package

Obtain the ZIP and its checksum from your operator through a trusted channel.
For a package named `Orbit-<version>-macos-arm64.zip`, in its download directory:

```sh
shasum -a 256 -c Orbit-<version>-macos-arm64.zip.sha256
```

Continue only if it reports `OK`. The package's `build-manifest.txt` records the
source commit, architecture and whether the build tree was dirty. Checksums
establish byte equality with the supplied manifest; they do not establish
publisher identity, signing or notarization.

Quit Orbit, unzip the package and place `Orbit.app` in `/Applications`. Open it
and enter the hub origin and pairing code. If your operator supplies `install.sh`,
inspect it before running it and use the operator's configured origin. There is
no repository-wide download host.

The installer checks package checksums and installs in `/Applications` without
`sudo`; it refuses, naming the folder, if your user cannot write there. It sets
the replaced app aside as `Orbit.app.anterior-<date>` and refuses while an Orbit
app/helper is running, reporting the processes it found. With no options it does
not launch the app, grant permissions, capture audio, enable login launch, edit
shell files or delete data.

The set-aside copy, and an older `~/Applications/Orbit.app` from versions before
0.9.0, remain until the new app has opened its local data. Its next start then
removes them, unless one of them is still running; the reasons are written to
`logs/updates.log` in the data directory.

## Pair the Mac

Ask your hub administrator for its HTTPS origin and a one-use pairing code.
The default lifetime is five minutes. First launch asks for both;
**Ajustes › Cuenta › Emparejar de nuevo** repeats pairing. Changing hubs removes
the previous pairing: one hub's device identity is not valid at another.

The Mac keeps the device token and storage key in Keychain. Provider credentials
stay on the server. See [self-hosting](self-hosting.md#pair-and-revoke-a-mac) for
local hub administrator commands.

## Permissions and calendar

Orbit reads the macOS Calendar database through EventKit. Accounts already
configured there (such as iCloud, Google or Exchange) need no extra provider
setup in Orbit. Full Calendar permission permits reading all calendars and
editing events that are editable; Orbit adds no calendar activation switch.

Grant Microphone when needed. Screen Recording is needed for system/app audio
and screenshots; Accessibility is needed for computer control and selected text.
Use macOS System Settings to grant or withdraw these permissions. Computer
control has no additional per-action approval layer in Orbit.

## Voice and meetings

| Shortcut | Action |
|---|---|
| `⌃⌥Space` | Hold to record a voice note |
| `⌃⌥D` | Start or stop a continuous assistant call |
| `⌃⌥M` | Start or stop meeting recording |
| `⌃⌥Esc` | Privacy stop |

`⌃⌥M` records the call app when exactly one is detected; if none or several are
detected, it records the microphone only. In the app, **Grabar** offers
**Sistema**, **Solo micrófono** and **Una app…**. App capture includes the entire
app, not an individual browser tab. Record participants only with their consent.

After recording, the Mac uploads audio, the hub transcribes it and writes the
note, and the app reads it back. Recording stopped does not mean the note is
ready. Offline audio waits on the Mac. Private notes and changes to a note
wait durably until the hub confirms them.

Privacy stop («Parar todo») cuts capture, playback and transfers locally and
survives quitting. **Ajustes › Privacidad y permisos › Reactivar Orbit** clears
it locally. Reactivating does not resume recording.

## Quit, reopen and update

⌘Q, the Dock's **Quit**, and **Salir de Orbit** in the menu bar all close the
daemon in order. Closing the window only hides it; recording continues. Click
the Dock icon or open the app from Finder to show the window again. If the
daemon exits unexpectedly, the app exits too; reopen it to recover. If an
installer reports a remaining process, quit it in Activity Monitor and retry.

With an update origin configured, Orbit compares the installed version with
`latest.txt` at startup and every six hours. A newer version shows a notice with
**Actualizar** and a line in the menu bar menu; nothing installs by itself, and
installation waits while a recording or call is in progress.
**Ajustes › Actualizaciones** offers the same check and **Instalar y reiniciar**.
Installing downloads the installer and verifies its hash, then the installer waits
up to 30 seconds for the app/helper to exit, checks the package, sets the previous
app aside and reopens Orbit. Without a configured origin, update checks are
disabled.

When upgrading an old 0.5.x installation, macOS may ask twice to read Keychain
entries originally created by the separately signed daemon: one for storage,
one for the device. **Permitir siempre** permits subsequent access.

The previous app is removed once the new one has started, so restoring an older
version means installing its package again: quit Orbit, delete
`/Applications/Orbit.app` and install the older package with its own installer
(versions before 0.9.0 install in `~/Applications`). Confirm hub compatibility
with your operator before downgrading. Normally upgrade the hub before the Mac
and roll back the Mac before the hub; schema and protocol compatibility still
need review. Meeting audio already confirmed by the hub has left the Mac and
stays on the hub.

## Login launch and removal

Use **Ajustes › Arranque › Abrir Orbit al iniciar sesión** to enable login launch.

Quit Orbit and delete `/Applications/Orbit.app` to remove the app. This leaves
its local data and Keychain entries intact. The actual data directory and
Keychain service are defined in `crates/orbit-core/src/orbit_bundle.rs` and
`crates/orbit-ipc/src/paths.rs`; use those for the identity of your own build.
Delete local data only after confirming that pending audio, private notes and
note changes have reached the hub. Removing a storage key makes remaining encrypted data
unreadable. Deleting the app does not delete hub data or revoke its token; ask
the hub administrator to revoke the device when retiring it.
