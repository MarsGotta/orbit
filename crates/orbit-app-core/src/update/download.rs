//! The only download and installer adapter: the updater (`super`) downloads
//! through it.
use orbit_core::{
    refusal::{Refusal, UpdateCause, UpdateNetworkCause, UpdateSizeSource},
    update::ReleaseManifest,
};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    time::{Duration, Instant},
};

/// Protects parsing a key=value manifest from substituted pages. The real
/// published manifest is 164 B; 8,192 B leaves roughly fiftyfold room. This
/// is not authentication or a bound on the installer.
pub const MANIFEST_CEILING_BYTES: usize = 8 * 1024;

const INSTALLER_BASELINE_BYTES: usize = 11_171;
const INSTALLER_HEADROOM_PERCENT: usize = 50;
/// Protects bash from a substituted page passed as an installer. The 11,171 B
/// script measured when this ceiling was set, plus 50% headroom, gives 16,756 B. This is not a
/// signature check. `scripts/release/publish-macos.sh` refuses to publish an installer
/// larger than this, because the updater of every Mac already installed
/// downloads at most this many bytes.
pub const INSTALLER_CEILING_BYTES: usize =
    INSTALLER_BASELINE_BYTES + INSTALLER_BASELINE_BYTES * INSTALLER_HEADROOM_PERCENT / 100;

/// One GET gets 60 seconds, matching the existing CLI's network budget. It
/// bounds an unavailable origin, not the installer process or user consent.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(DOWNLOAD_TIMEOUT_SECONDS);
/// In seconds, as the interface reads it for the update orders' ceilings
/// (`apps/desktop/src/state/ceilings.ts`, `contracts/constants.json`).
pub const DOWNLOAD_TIMEOUT_SECONDS: u64 = 60;

/// Bounds redirect loops and request amplification. Ten preserves reqwest's
/// previous default; the published origin was measured at zero redirects on
/// 2026-09-29. This is not an origin or authenticity check: each hop is also
/// restricted to the same scheme and host. A port change on that host is
/// allowed: TLS authenticates the host name, not the port, and the origin that
/// redirects already serves the installer.
const MAX_REDIRECTS: usize = 10;

#[derive(Debug, Clone, Copy)]
pub struct Download {
    pub file: &'static str,
    pub ceiling: usize,
}

pub const MANIFEST: Download = Download {
    file: "latest.txt",
    ceiling: MANIFEST_CEILING_BYTES,
};
pub const INSTALLER: Download = Download {
    file: "install.sh",
    ceiling: INSTALLER_CEILING_BYTES,
};

pub struct Updater {
    origin: String,
    client: reqwest::Client,
    timeout: Duration,
}

impl Updater {
    pub fn new(origin: impl Into<String>) -> Result<Self, Refusal> {
        Self::with_timeout(origin, DOWNLOAD_TIMEOUT)
    }

    /// Explicit timeout is also useful for local synthetic origins in tests.
    pub fn with_timeout(origin: impl Into<String>, timeout: Duration) -> Result<Self, Refusal> {
        let origin = origin.into().trim_end_matches('/').to_owned();
        let parsed = reqwest::Url::parse(&origin).map_err(|_| invalid_origin(&origin))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(invalid_origin(&origin));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            // fetch validates each Location before making the next request.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(client_build_error)?;
        Ok(Self {
            origin,
            client,
            timeout,
        })
    }

    pub async fn fetch(&self, download: &Download) -> Result<String, Refusal> {
        let origin = reqwest::Url::parse(&self.origin).map_err(|_| invalid_origin(&self.origin))?;
        let url = reqwest::Url::parse(&format!("{}/{}", self.origin, download.file))
            .map_err(|_| invalid_origin(&self.origin))?;
        let started = Instant::now();
        let (response, url) = self.follow_redirects(&origin, url, started).await?;
        self.read_body(download, response, &url, started).await
    }

    /// The response to a GET of `url` once it stops redirecting, and the URL
    /// that gave it. Every hop shares the one timeout that began at `started`.
    async fn follow_redirects(
        &self,
        origin: &reqwest::Url,
        mut url: reqwest::Url,
        started: Instant,
    ) -> Result<(reqwest::Response, reqwest::Url), Refusal> {
        let mut followed = 0;
        loop {
            let remaining = self.timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(deadline(url.as_str(), started, self.timeout));
            }
            let response = self
                .client
                .get(url.clone())
                .timeout(remaining)
                .send()
                .await
                .map_err(|error| network_error(url.as_str(), started, self.timeout, &error))?;
            if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                return Ok((response, url));
            }
            url = redirect_target(origin, &url, &response, followed)?;
            followed += 1;
        }
    }

    /// The body of the final response: refused for a status that is not a
    /// success, a size over the download's ceiling, declared or observed, and
    /// text that is not UTF-8.
    async fn read_body(
        &self,
        download: &Download,
        mut response: reqwest::Response,
        url: &reqwest::Url,
        started: Instant,
    ) -> Result<String, Refusal> {
        if !response.status().is_success() {
            return Err(Refusal::Update {
                cause: UpdateCause::HttpStatus {
                    url: url.to_string(),
                    status: response.status().as_u16(),
                },
            });
        }
        if let Some(bytes) = response.content_length()
            && bytes > download.ceiling as u64
        {
            return Err(too_large(download, bytes, UpdateSizeSource::Declared));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| network_error(url.as_str(), started, self.timeout, &error))?
        {
            let observed = body.len().saturating_add(chunk.len());
            if observed > download.ceiling {
                return Err(too_large(
                    download,
                    observed as u64,
                    UpdateSizeSource::Observed,
                ));
            }
            body.extend_from_slice(&chunk);
        }
        String::from_utf8(body).map_err(|error| Refusal::Update {
            cause: UpdateCause::InvalidUtf8 {
                file: download.file.to_owned(),
                byte: error.utf8_error().valid_up_to(),
            },
        })
    }

    pub async fn manifest(&self) -> Result<ReleaseManifest, Refusal> {
        ReleaseManifest::parse(&self.fetch(&MANIFEST).await?)
    }

    /// Fetches both published artifacts and checks the installer when the
    /// manifest carries its SHA-256. Old manifests have no installer digest.
    pub async fn installer_script(&self) -> Result<(String, bool), Refusal> {
        let manifest = self.manifest().await?;
        self.installer_script_with_manifest(&manifest).await
    }

    /// Verify against the manifest used for the caller's release decision.
    pub async fn installer_script_with_manifest(
        &self,
        manifest: &ReleaseManifest,
    ) -> Result<(String, bool), Refusal> {
        let script = self.fetch(&INSTALLER).await?;
        let Some(expected) = manifest.fields.get("installer_sha256") else {
            return Ok((script, false));
        };
        if !orbit_core::update::safe_digest(expected) {
            return Err(Refusal::Update {
                cause: UpdateCause::InvalidDigest {
                    key: "installer_sha256".to_owned(),
                    value: expected.clone(),
                },
            });
        }
        let actual = format!("{:x}", Sha256::digest(script.as_bytes()));
        if actual != *expected {
            return Err(Refusal::Update {
                cause: UpdateCause::InstallerDigestMismatch {
                    expected: expected.clone(),
                    actual,
                },
            });
        }
        Ok((script, true))
    }
}

/// Where a redirect from `url` leads, after `followed` others: refused
/// without a valid Location, with credentials in it, outside the origin's
/// scheme and host, or past `MAX_REDIRECTS`.
fn redirect_target(
    origin: &reqwest::Url,
    url: &reqwest::Url,
    response: &reqwest::Response,
    followed: usize,
) -> Result<reqwest::Url, Refusal> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| invalid_redirect(url.as_str(), "falta Location"))?
        .to_str()
        .map_err(|_| invalid_redirect(url.as_str(), "Location is not valid HTTP text"))?;
    let target = url
        .join(location)
        .map_err(|_| invalid_redirect(url.as_str(), "Location is not a valid URL"))?;
    if !target.username().is_empty() || target.password().is_some() {
        return Err(invalid_redirect(
            url.as_str(),
            "Location contiene credenciales",
        ));
    }
    let from_origin = origin.origin().ascii_serialization();
    let target_origin = target.origin().ascii_serialization();
    if origin.scheme() != target.scheme() || origin.host_str() != target.host_str() {
        return Err(Refusal::Update {
            cause: UpdateCause::RedirectOutsideOrigin {
                from_origin,
                target_origin,
            },
        });
    }
    if followed == MAX_REDIRECTS {
        return Err(Refusal::Update {
            cause: UpdateCause::RedirectLimit {
                url: url.to_string(),
                followed,
                max_redirects: MAX_REDIRECTS,
            },
        });
    }
    Ok(target)
}

fn invalid_origin(value: &str) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::InvalidOrigin { bytes: value.len() },
    }
}

fn invalid_redirect(url: &str, cause: &str) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::InvalidRedirect {
            url: url.to_owned(),
            cause: cause.to_owned(),
        },
    }
}

fn client_build_error(error: reqwest::Error) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::ClientBuild {
            cause: error.to_string(),
        },
    }
}

fn deadline(url: &str, started: Instant, timeout: Duration) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::Deadline {
            url: url.to_owned(),
            waited_ms: started.elapsed().as_millis() as u64,
            deadline_ms: timeout.as_millis() as u64,
        },
    }
}

fn too_large(download: &Download, bytes: u64, source: UpdateSizeSource) -> Refusal {
    Refusal::Update {
        cause: UpdateCause::TooLarge {
            file: download.file.to_owned(),
            bytes,
            max_bytes: download.ceiling as u64,
            source,
        },
    }
}

fn network_error(
    url: &str,
    started: Instant,
    timeout: Duration,
    error: &reqwest::Error,
) -> Refusal {
    if error.is_timeout() {
        return deadline(url, started, timeout);
    }
    let mut source = error.source();
    let mut refused = false;
    let mut dns = false;
    let mut tls = false;
    let mut detail = error.to_string();
    while let Some(current) = source {
        if let Some(io) = current.downcast_ref::<std::io::Error>() {
            refused |= io.kind() == std::io::ErrorKind::ConnectionRefused;
        }
        detail = current.to_string();
        let description = detail.to_ascii_lowercase();
        dns |= description.contains("dns")
            || description.contains("lookup")
            || description.contains("name or service");
        tls |= description.contains("tls")
            || description.contains("certificate")
            || description.contains("received corrupt message");
        source = current.source();
    }
    let cause = if refused {
        UpdateNetworkCause::ConnectionRefused
    } else if dns {
        UpdateNetworkCause::Dns
    } else if tls {
        UpdateNetworkCause::Tls
    } else {
        UpdateNetworkCause::Other { detail }
    };
    Refusal::Update {
        cause: UpdateCause::NoResponse {
            url: url.to_owned(),
            cause,
        },
    }
}

#[cfg(test)]
mod ceiling_tests;
