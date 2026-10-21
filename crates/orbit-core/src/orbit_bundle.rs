//! The shipped macOS bundle's identity and paths. All decisions use supplied
//! paths; callers resolve symlinks before asking and own any filesystem access.
//!
//! SIM-25: the Cargo feature `pruebas` builds «Orbit Pruebas», the copy the
//! real test on the Mac installs beside Mars's Orbit. It differs only in what
//! would make the two collide — the bundle name, the data directory and the
//! Keychain service — and in reaching its test hub over plain HTTP on the
//! tailnet, and in exposing `orbit.sock` to the real-test harness.
//! `scripts/bundle-macos.sh` turns it on with `ORBIT_PRUEBAS=1` and
//! gives the bundle its own identifier; nothing else ever does.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

#[cfg(not(feature = "pruebas"))]
pub const NAME: &str = "Orbit.app";
#[cfg(feature = "pruebas")]
pub const NAME: &str = "Orbit Pruebas.app";
#[cfg(not(feature = "pruebas"))]
pub const IDENTIFIER: &str = "app.orbit.companion";
#[cfg(feature = "pruebas")]
pub const IDENTIFIER: &str = "app.orbit.companion.pruebas";
#[cfg(not(feature = "pruebas"))]
pub const PARKED_PREFIX: &str = "Orbit.app.anterior-";
#[cfg(feature = "pruebas")]
pub const PARKED_PREFIX: &str = "Orbit Pruebas.app.anterior-";
/// `directories::ProjectDirs::from` arguments: qualifier, organization and
/// application of the data directory (on macOS `~/Library/Application
/// Support/com.MarsGotta.Orbit`).
#[cfg(not(feature = "pruebas"))]
pub const DATA_DIRECTORY: [&str; 3] = ["com", "MarsGotta", "Orbit"];
#[cfg(feature = "pruebas")]
pub const DATA_DIRECTORY: [&str; 3] = ["com", "MarsGotta", "OrbitPruebas"];
/// The Keychain service of the device token and the storage key.
#[cfg(not(feature = "pruebas"))]
pub const KEYCHAIN_SERVICE: &str = "com.marsgotta.orbit";
#[cfg(feature = "pruebas")]
pub const KEYCHAIN_SERVICE: &str = "com.marsgotta.orbit-pruebas";
pub const USER_APPLICATIONS: &str = "Applications";
pub const SYSTEM_APPLICATIONS: &str = "/Applications";
pub const INSTALL_DATA_DIRECTORY: &str = "Library/Application Support/com.MarsGotta.Orbit";
pub const ENGINE_RELATIVE_PATH: &str = "Helpers/Orbit Engine.app/Contents/MacOS/cua-driver";
pub const INSTALL_RECORD: &str = ".orbit-instalado";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleKind {
    Installed,
    Parked,
}

/// Renamed bundles are not Orbit. A parked copy has a nonempty suffix made by
/// the installer and remains Orbit while its executables are running.
pub fn kind(path: &Path) -> Option<BundleKind> {
    let name = path.file_name()?.to_str()?;
    if name == NAME {
        Some(BundleKind::Installed)
    } else if name.starts_with(PARKED_PREFIX) && name.len() > PARKED_PREFIX.len() {
        Some(BundleKind::Parked)
    } else {
        None
    }
}

/// A retirement decision over an entry in the two installation directories.
#[derive(Debug, PartialEq, Eq)]
pub enum Retirement {
    Unrelated,
    Keep(&'static str),
    Remove,
}

pub fn retirement(
    candidate: &Path,
    current: &Path,
    user_applications: &Path,
    ready: bool,
    listing: Option<&str>,
) -> Retirement {
    if candidate.parent() != current.parent() && candidate.parent() != Some(user_applications) {
        return Retirement::Unrelated;
    }
    let Some(bundle_kind) = kind(candidate) else {
        return Retirement::Unrelated;
    };
    if candidate == current {
        return Retirement::Keep("app actual");
    }
    if bundle_kind == BundleKind::Installed && candidate.parent() != Some(user_applications) {
        return Retirement::Unrelated;
    }
    if !ready {
        return Retirement::Keep("arranque sin confirmar");
    }
    let Some(listing) = listing.filter(|listing| !listing.trim().is_empty()) else {
        return Retirement::Keep("listado de procesos desconocido");
    };
    // The former install.sh parked_running rule: exact app/helper paths.
    // An unreadable or malformed census is unknown, never evidence of closure.
    let mut running = false;
    for line in listing.lines().filter(|line| !line.trim().is_empty()) {
        let Some((pid, rest)) = line.trim().split_once(char::is_whitespace) else {
            return Retirement::Keep("listado de procesos desconocido");
        };
        let Some((parent, executable)) = rest.trim_start().split_once(char::is_whitespace) else {
            return Retirement::Keep("listado de procesos desconocido");
        };
        if pid.parse::<u32>().is_err()
            || parent.parse::<u32>().is_err()
            || executable.trim().is_empty()
        {
            return Retirement::Keep("listado de procesos desconocido");
        }
        running |= [Member::App, Member::Helper].into_iter().any(|member| {
            member_path(candidate, member).as_deref() == Some(Path::new(executable.trim()))
        });
    }
    if running {
        Retirement::Keep("copia en marcha")
    } else {
        Retirement::Remove
    }
}

/// The executables of the bundle. The daemon runs inside the app since SIM-51:
/// `Contents/Resources/orbit`, its executable until 0.5.x, is not shipped and
/// is not a member; only `scripts/install.sh` still knows its path, to wait for
/// a 0.5.x daemon before replacing its bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Member {
    App,
    Helper,
    Engine,
}

impl Member {
    pub const ALL: [Self; 3] = [Self::App, Self::Helper, Self::Engine];

    pub const fn relative_path(self) -> &'static str {
        match self {
            Self::App => "MacOS/Orbit",
            Self::Helper => "MacOS/OrbitNative",
            Self::Engine => ENGINE_RELATIVE_PATH,
        }
    }
}

pub fn member_path(bundle: &Path, member: Member) -> Option<PathBuf> {
    contents_path(bundle).map(|contents| contents.join(member.relative_path()))
}

pub fn contents_path(bundle: &Path) -> Option<PathBuf> {
    kind(bundle).map(|_| bundle.join("Contents"))
}

/// Identify an exact member in an installed or parked bundle at any location.
pub fn member_of(executable: &Path) -> Option<(PathBuf, Member)> {
    let bundle = executable.ancestors().find(|path| {
        kind(path).is_some()
            && Member::ALL
                .into_iter()
                .any(|member| member_path(path, member).as_deref() == Some(executable))
    })?;
    let member = Member::ALL
        .into_iter()
        .find(|member| member_path(bundle, *member).as_deref() == Some(executable))?;
    Some((bundle.to_path_buf(), member))
}

pub fn bundle_of(executable: &Path, member: Member) -> Option<PathBuf> {
    member_of(executable).and_then(|(bundle, found)| (found == member).then_some(bundle))
}

/// Only Orbit Pruebas exposes the test harness's Unix socket. The standalone
/// development daemon keeps its socket independently of this bundle decision.
pub const LOCAL_SOCKET: bool = cfg!(feature = "pruebas");

/// Whether a hub at `host` may be reached over plain HTTP. Never in the
/// published build. In «Orbit Pruebas», only a tailnet address (Tailscale's
/// 100.64.0.0/10): WireGuard encrypts that link, and the test hub listens
/// there and nowhere else.
pub fn plain_http_hub(host: &str) -> bool {
    cfg!(feature = "pruebas")
        && host.parse::<Ipv4Addr>().is_ok_and(|address| {
            let [first, second, ..] = address.octets();
            first == 100 && second & 0xC0 == 64
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim95_retirement_uses_exact_names_readiness_and_a_known_listing()
    -> Result<(), Box<dyn std::error::Error>> {
        let system = Path::new("/fixture/Applications");
        let user = Path::new("/fixture/home/Applications");
        let current = system.join(NAME);
        let parked = system.join(format!("{PARKED_PREFIX}old"));
        let old = user.join(NAME);
        let idle = "10 1 /usr/bin/other";
        for candidate in [&parked, &old, &user.join(format!("{PARKED_PREFIX}older"))] {
            assert_eq!(
                retirement(candidate, &current, user, false, Some(idle)),
                Retirement::Keep("arranque sin confirmar")
            );
            for listing in [None, Some(""), Some("malformed")] {
                assert_eq!(
                    retirement(candidate, &current, user, true, listing),
                    Retirement::Keep("listado de procesos desconocido")
                );
            }
            assert_eq!(
                retirement(candidate, &current, user, true, Some(idle)),
                Retirement::Remove
            );
            for member in [Member::App, Member::Helper] {
                let executable = member_path(candidate, member).ok_or("bundle member")?;
                // A match at the beginning of a large listing must survive too.
                let listing = format!(
                    "11 1 {}\n{}",
                    executable.display(),
                    format!("{idle}\n").repeat(2000)
                );
                assert_eq!(
                    retirement(candidate, &current, user, true, Some(&listing)),
                    Retirement::Keep("copia en marcha")
                );
            }
        }
        assert_eq!(
            retirement(&current, &current, user, true, Some(idle)),
            Retirement::Keep("app actual")
        );
        for directory in [system, user] {
            for name in [
                "Expo Orbit.app",
                "Orbit 2.app",
                "Orbit Pruebas.app",
                PARKED_PREFIX,
            ] {
                // The pruebas build owns its own name, never the published app.
                if name == NAME {
                    continue;
                }
                assert_eq!(
                    retirement(&directory.join(name), &current, user, true, Some(idle)),
                    Retirement::Unrelated
                );
            }
        }
        assert_eq!(
            retirement(
                &Path::new("/elsewhere").join(NAME),
                &current,
                user,
                true,
                Some(idle)
            ),
            Retirement::Unrelated
        );
        Ok(())
    }

    #[test]
    fn sim98_engine_is_a_member_only_inside_its_own_app() {
        let bundle = Path::new("/fixture/Applications").join(NAME);
        let engine = bundle.join("Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver");
        assert_eq!(member_path(&bundle, Member::Engine), Some(engine.clone()));
        assert_eq!(member_of(&engine), Some((bundle.clone(), Member::Engine)));
        assert_eq!(member_of(&bundle.join("Contents/MacOS/cua-driver")), None);
        // The installer waits for the app/helper, never for an orphan engine.
        let parked = Path::new("/fixture/Applications").join(format!("{PARKED_PREFIX}old"));
        let listing = format!(
            "11 1 {}",
            parked
                .join("Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver")
                .display()
        );
        assert_eq!(
            retirement(
                &parked,
                &bundle,
                Path::new("/fixture/home/Applications"),
                true,
                Some(&listing)
            ),
            Retirement::Remove
        );
    }

    #[test]
    fn ref501_route_table() {
        for (bundle, expected) in [
            (format!("/Applications/{NAME}"), Some(BundleKind::Installed)),
            (
                format!("/Users/sample/Applications/{PARKED_PREFIX}20260928"),
                Some(BundleKind::Parked),
            ),
            (
                format!("/Volumes/Externo/{NAME}"),
                Some(BundleKind::Installed),
            ),
            ("/Applications/Orbit 2.app".into(), None),
            (format!("/Users/sample/notes/{PARKED_PREFIX}"), None),
        ] {
            assert_eq!(kind(Path::new(&bundle)), expected, "{bundle}");
            for member in Member::ALL {
                let path = Path::new(&bundle)
                    .join("Contents")
                    .join(member.relative_path());
                assert_eq!(
                    member_of(&path),
                    expected.map(|_| (PathBuf::from(&bundle), member))
                );
            }
        }
        for path in [
            "/Applications/Editor.app/Contents/MacOS/Orbit",
            "/Users/sample/notes/Orbit.app/Contents/MacOS/OrbitNative.backup",
            "/build/target/debug/orbit",
            // SIM-51: the 0.5.x daemon's path is no longer a member.
            "/Applications/Orbit.app/Contents/Resources/orbit",
        ] {
            assert_eq!(member_of(Path::new(path)), None, "{path}");
        }
    }

    /// SIM-25: «Orbit Pruebas» is chosen at build time, and the build that is
    /// published must not move: Mars's permissions are tied to this name, her
    /// data lives in this directory and her keys in this Keychain service.
    #[cfg(not(feature = "pruebas"))]
    #[test]
    fn sim25_the_official_build_keeps_its_name_data_directory_and_keychain() {
        assert_eq!(NAME, "Orbit.app");
        assert_eq!(IDENTIFIER, "app.orbit.companion");
        assert_eq!(PARKED_PREFIX, "Orbit.app.anterior-");
        assert_eq!(DATA_DIRECTORY, ["com", "MarsGotta", "Orbit"]);
        assert_eq!(KEYCHAIN_SERVICE, "com.marsgotta.orbit");
        assert_eq!(Member::App.relative_path(), "MacOS/Orbit");
        assert_eq!(Member::Helper.relative_path(), "MacOS/OrbitNative");
        assert_eq!(
            Member::Engine.relative_path(),
            "Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
        );
        for host in ["100.64.0.2", "100.64.0.1", "100.127.255.254", "hub.example"] {
            assert!(!plain_http_hub(host), "{host}");
        }
    }

    #[cfg(feature = "pruebas")]
    #[test]
    fn sim25_orbit_pruebas_has_its_own_name_data_directory_and_keychain() {
        assert_eq!(NAME, "Orbit Pruebas.app");
        assert_eq!(IDENTIFIER, "app.orbit.companion.pruebas");
        assert_eq!(PARKED_PREFIX, "Orbit Pruebas.app.anterior-");
        assert_eq!(DATA_DIRECTORY, ["com", "MarsGotta", "OrbitPruebas"]);
        assert_eq!(KEYCHAIN_SERVICE, "com.marsgotta.orbit-pruebas");
        assert_eq!(Member::App.relative_path(), "MacOS/Orbit");
        assert_eq!(Member::Helper.relative_path(), "MacOS/OrbitNative");
        assert_eq!(
            Member::Engine.relative_path(),
            "Helpers/Orbit Engine.app/Contents/MacOS/cua-driver"
        );
        assert_eq!(kind(Path::new("/Users/m/Applications/Orbit.app")), None);
        for host in ["100.64.0.2", "100.64.0.1", "100.127.255.254"] {
            assert!(plain_http_hub(host), "{host}");
        }
        for host in [
            "100.63.255.255",
            "100.128.0.1",
            "10.0.0.1",
            "hub.example",
            "::1",
        ] {
            assert!(!plain_http_hub(host), "{host}");
        }
    }
}
