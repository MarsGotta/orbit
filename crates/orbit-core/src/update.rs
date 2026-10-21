//! Pure parsing and decisions for a published release and a local install.
use crate::refusal::{Refusal, UpdateCause};
use semver::Version;
use std::collections::BTreeMap;

pub type Fields = BTreeMap<String, String>;

pub fn parse_fields(body: &str) -> Fields {
    body.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// A filename from an untrusted manifest. The 100-byte limit bounds a package
/// basename, not the download or any filesystem allocation; existing package
/// names are under 100 bytes and the installer does the final path validation.
pub fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn safe_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseManifest {
    pub version: Version,
    pub packages: BTreeMap<String, Package>,
    pub fields: Fields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionVerdict {
    Newer {
        published: String,
    },
    Same,
    NotNewer {
        installed: String,
        published: String,
    },
}

pub fn compare_running_version(
    published: &ReleaseManifest,
    running: &str,
) -> Result<VersionVerdict, Refusal> {
    let installed = Version::parse(running).map_err(|_| Refusal::Update {
        cause: UpdateCause::InvalidVersion {
            value: running.to_owned(),
        },
    })?;
    if published.version > installed {
        Ok(VersionVerdict::Newer {
            published: published.version.to_string(),
        })
    } else if published.version == installed {
        Ok(VersionVerdict::Same)
    } else {
        Ok(VersionVerdict::NotNewer {
            installed: installed.to_string(),
            published: published.version.to_string(),
        })
    }
}

impl ReleaseManifest {
    pub fn parse(body: &str) -> Result<Self, Refusal> {
        let fields = parse_fields(body);
        let value = fields
            .get("version")
            .ok_or_else(|| missing("version", &fields))?;
        let version = Version::parse(value).map_err(|_| Refusal::Update {
            cause: UpdateCause::InvalidVersion {
                value: value.clone(),
            },
        })?;
        let mut packages = BTreeMap::new();
        for architecture in ["arm64", "x86_64"] {
            if fields.contains_key(&format!("{architecture}_file"))
                && fields.contains_key(&format!("{architecture}_sha256"))
            {
                packages.insert(
                    architecture.to_owned(),
                    parse_package(&fields, architecture)?,
                );
            }
        }
        Ok(Self {
            version,
            packages,
            fields,
        })
    }

    pub fn package(&self, architecture: &str) -> Result<Package, Refusal> {
        match self.packages.get(architecture) {
            Some(package) => Ok(package.clone()),
            None => parse_package(&self.fields, architecture),
        }
    }
}

fn parse_package(fields: &Fields, architecture: &str) -> Result<Package, Refusal> {
    let file_key = format!("{architecture}_file");
    let digest_key = format!("{architecture}_sha256");
    let file = fields
        .get(&file_key)
        .ok_or_else(|| missing(&file_key, fields))?;
    let sha256 = fields
        .get(&digest_key)
        .ok_or_else(|| missing(&digest_key, fields))?;
    if !safe_name(file) {
        return Err(Refusal::Update {
            cause: UpdateCause::InvalidFile {
                key: file_key,
                value: file.clone(),
            },
        });
    }
    if !safe_digest(sha256) {
        return Err(Refusal::Update {
            cause: UpdateCause::InvalidDigest {
                key: digest_key,
                value: sha256.clone(),
            },
        });
    }
    Ok(Package {
        file: file.clone(),
        sha256: sha256.clone(),
    })
}

fn missing(key: &str, fields: &Fields) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::MissingKey {
            key: key.to_owned(),
            available_keys: fields.keys().cloned().collect(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRecord {
    pub version: Option<Version>,
    pub sha256: Option<String>,
    pub fields: Fields,
}

impl InstallRecord {
    pub fn parse(body: &str) -> Self {
        let fields = parse_fields(body);
        let version = fields
            .get("version")
            .and_then(|value| Version::parse(value).ok());
        let sha256 = fields
            .get("sha256")
            .filter(|value| safe_digest(value))
            .cloned();
        Self {
            version,
            sha256,
            fields,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // Fixture assertions only.
    use super::*;
    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn release_manifest_keeps_package_digest_and_unknown_fields() {
        let manifest = ReleaseManifest::parse(&format!(
            "version=0.3.14\narm64_file=Orbit.zip\narm64_sha256={A}\nfuture=value"
        ))
        .unwrap();
        assert_eq!(manifest.fields["future"], "value");
        assert_eq!(manifest.package("arm64").unwrap().sha256, A);
    }

    #[test]
    fn install_record_validates_digest_independently_of_version() {
        for (body, version, digest) in [
            (format!("sha256={A}"), None, Some(A)),
            (
                format!("version=0.3.13\nsha256={A}"),
                Some("0.3.13"),
                Some(A),
            ),
            ("version=0.3.13\nsha256=bad".into(), Some("0.3.13"), None),
        ] {
            let record = InstallRecord::parse(&body);
            assert_eq!(
                record.version.map(|value| value.to_string()).as_deref(),
                version
            );
            assert_eq!(record.sha256.as_deref(), digest);
        }
    }

    #[test]
    fn missing_and_invalid_fields_preserve_causes() {
        let missing_version = ReleaseManifest::parse("arm64_file=Orbit.zip").unwrap_err();
        assert!(
            matches!(missing_version, Refusal::Update { cause: UpdateCause::MissingKey { key, .. } } if key == "version")
        );
        let invalid = ReleaseManifest::parse("version=release-x").unwrap_err();
        assert!(
            matches!(invalid, Refusal::Update { cause: UpdateCause::InvalidVersion { value } } if value == "release-x")
        );
        let manifest = ReleaseManifest::parse("version=0.3.13\narm64_file=Orbit.zip").unwrap();
        assert!(
            matches!(manifest.package("arm64"), Err(Refusal::Update { cause: UpdateCause::MissingKey { key, available_keys } }) if key == "arm64_sha256" && available_keys.contains(&"arm64_file".to_owned()))
        );
    }

    #[test]
    fn installed_0312_record_keeps_all_five_fields() {
        let body = format!(
            "version=0.3.12\nsha256={A}\narchitecture=arm64\ninstalled=2026-09-25T07:36:53Z\norigin=https://updates.example.test\n"
        );
        let record = InstallRecord::parse(&body);
        assert_eq!(record.version.unwrap().to_string(), "0.3.12");
        assert_eq!(record.sha256.as_deref(), Some(A));
        assert_eq!(record.fields.len(), 5);
        assert_eq!(record.fields["architecture"], "arm64");
        assert_eq!(record.fields["origin"], "https://updates.example.test");
    }
}
