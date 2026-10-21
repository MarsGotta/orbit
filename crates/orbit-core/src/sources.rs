//! Immutable local sources. These contracts grant neither inference nor publication.
//! Time values are supplied by adapters; this module never reads a clock or device.
mod desktop;
mod png_validation;
mod selected_text;

pub use desktop::{
    DesktopFitError, DesktopScale, FittedDesktop, MAX_DESKTOP_HEIGHT, MAX_DESKTOP_PNG_BYTES,
    MAX_DESKTOP_WIDTH, fit_desktop_png, is_desktop_action, map_desktop_action,
};
pub use png_validation::validate_png;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_SOURCE_BYTES: usize = 700_000;
pub const MAX_SELECTED_TEXT_BYTES: usize = 16_384;
pub const MAX_SELECTED_TEXT_UTF16_UNITS: u64 = 4096;
pub const MAX_WINDOW_WIDTH: u32 = 1280;
pub const MAX_WINDOW_HEIGHT: u32 = 1024;
pub const MAX_WINDOW_CANDIDATES: usize = 200;
pub const INVENTORY_TTL_NS: u64 = 30_000_000_000;
pub const CAPTURE_TIMEOUT_NS: u64 = 10_000_000_000;
pub const SELECTED_TEXT_TIMEOUT_NS: u64 = 5_000_000_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SourceError {
    #[error("invalid source identity or selection binding")]
    Identity,
    #[error("invalid or expired source time interval")]
    Interval,
    #[error("source exceeds its size or descriptor limits")]
    Bounds,
    #[error("source metadata does not match the authorized selection")]
    Scope,
    #[error("invalid, unsupported or damaged PNG source")]
    Image,
    #[error("invalid or inconsistent UTF-8 selected text")]
    Text,
}

/// Observed owner/window binding, not proof of an immutable OS window incarnation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WindowBinding {
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub inventory_id: String,
    pub selection_token: String,
    pub window_id: u32,
    pub application_pid: i32,
    pub application_instance_id: String,
    pub binding_quality: String,
    pub descriptor_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WindowCandidate {
    pub binding: Option<WindowBinding>,
    pub window_id: u32,
    pub application_pid: i32,
    pub application_name: String,
    pub bundle_id: String,
    pub title: String,
    pub title_truncated: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WindowInventory {
    pub native_instance_id: String,
    pub privacy_generation: u64,
    pub inventory_id: String,
    pub clock_domain: String,
    pub observed_monotonic_ns: u64,
    pub expires_monotonic_ns: u64,
    /// Observed accessibility capability, not a guarantee that a window has a selection.
    pub selected_text_available: bool,
    pub windows: Vec<WindowCandidate>,
    pub omitted_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceTarget {
    NativeWindow {
        binding: WindowBinding,
        application_name: String,
        bundle_id: String,
        title: String,
        title_truncated: bool,
        observed_monotonic_ns: u64,
        expires_monotonic_ns: u64,
    },
}

/// The exact material authorized by the intent; adapters must not substitute formats.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    #[default]
    Png,
    SelectedText,
}

impl SourceFormat {
    pub fn is_png(&self) -> bool {
        *self == Self::Png
    }

    /// Maximum plaintext reservation, also enforced when validating a receipt.
    pub fn max_bytes(&self) -> usize {
        match self {
            Self::Png => MAX_SOURCE_BYTES,
            Self::SelectedText => MAX_SELECTED_TEXT_BYTES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceIntent {
    pub id: String,
    pub device_id: Option<String>,
    pub session_id: Option<String>,
    pub generation: u64,
    /// Wall time is provenance only, never authority to reuse a capture token.
    pub requested_at_ms: i64,
    pub expires_at_ms: i64,
    pub target: SourceTarget,
    pub annotation: String,
    /// Omitted for PNG so previously authenticated receipt serialization is unchanged.
    #[serde(default, skip_serializing_if = "SourceFormat::is_png")]
    pub format: SourceFormat,
}

/// An application-provided range, measured in UTF-16 code units, not UTF-8 bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectedTextRange {
    pub location: u64,
    pub length: u64,
    pub unit: String,
}

impl SelectedTextRange {
    pub fn validate(&self) -> Result<(), SourceError> {
        if self.unit != "utf16_code_units" {
            return Err(SourceError::Scope);
        }
        if self.length == 0
            || self.length > MAX_SELECTED_TEXT_UTF16_UNITS
            || self
                .location
                .checked_add(self.length)
                .is_none_or(|end| end > i64::MAX as u64)
        {
            return Err(SourceError::Bounds);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceMedia {
    Png {
        width: u32,
        height: u32,
    },
    SelectedText {
        range: SelectedTextRange,
        ax_binding_quality: String,
        selection_scope: String,
        sensitivity: String,
    },
}

impl SourceMedia {
    pub fn format(&self) -> SourceFormat {
        match self {
            Self::Png { .. } => SourceFormat::Png,
            Self::SelectedText { .. } => SourceFormat::SelectedText,
        }
    }

    fn validate(&self) -> Result<(), SourceError> {
        match self {
            Self::Png { width, height } => validate_dimensions(*width, *height),
            Self::SelectedText {
                range,
                ax_binding_quality,
                selection_scope,
                sensitivity,
            } => {
                range.validate()?;
                if ax_binding_quality != "unique_owner_title_bounds"
                    || selection_scope != "application_focused_element"
                    || sensitivity != "not_certified"
                {
                    return Err(SourceError::Scope);
                }
                Ok(())
            }
        }
    }
}

/// Fully validate original bytes without normalizing, truncating or repairing them.
/// Text controls other than NUL remain data; displaying them requires sanitization.
pub fn validate_source_bytes(bytes: &[u8], media: &SourceMedia) -> Result<(), SourceError> {
    media.validate()?;
    match media {
        SourceMedia::Png { width, height } => validate_png(bytes, *width, *height),
        SourceMedia::SelectedText { range, .. } => selected_text::validate(bytes, range),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    pub origin: SourceTarget,
    pub media: SourceMedia,
    pub clock_domain: String,
    /// These delimit native observation and validation, not an exact application event.
    pub capture_started_monotonic_ns: u64,
    pub capture_completed_monotonic_ns: u64,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceReceipt {
    pub intent: SourceIntent,
    pub metadata: SourceMetadata,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceStage {
    Listing,
    Available,
    Capturing,
    Validated,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceProgress {
    pub id: String,
    pub stage: SourceStage,
    pub detail: String,
}

impl WindowBinding {
    pub fn validate(&self) -> Result<(), SourceError> {
        if [
            &self.native_instance_id,
            &self.inventory_id,
            &self.selection_token,
            &self.application_instance_id,
        ]
        .iter()
        .any(|id| !crate::wire::canonical_uuid(id))
            || self.window_id == 0
            || self.application_pid <= 0
            || self.binding_quality != "observed_owner_and_window"
            || !valid_sha256(&self.descriptor_hash)
        {
            return Err(SourceError::Identity);
        }
        Ok(())
    }
}

impl WindowInventory {
    pub fn validate(&self) -> Result<(), SourceError> {
        if !crate::wire::canonical_uuid(&self.native_instance_id)
            || !crate::wire::canonical_uuid(&self.inventory_id)
            || self.clock_domain != "mach_absolute_time"
        {
            return Err(SourceError::Identity);
        }
        validate_inventory_interval(self.observed_monotonic_ns, self.expires_monotonic_ns)?;
        if self.windows.len() > MAX_WINDOW_CANDIDATES {
            return Err(SourceError::Bounds);
        }
        let mut tokens = BTreeSet::new();
        let mut windows = BTreeSet::new();
        for window in &self.windows {
            validate_labels(&window.application_name, &window.bundle_id, &window.title)?;
            if !windows.insert(window.window_id)
                || window.window_id == 0
                || window
                    .unavailable_reason
                    .as_ref()
                    .is_some_and(|reason| reason.len() > 128)
            {
                return Err(SourceError::Identity);
            }
            if let Some(binding) = &window.binding {
                binding.validate()?;
                if binding.native_instance_id != self.native_instance_id
                    || binding.inventory_id != self.inventory_id
                    || binding.privacy_generation != self.privacy_generation
                    || binding.window_id != window.window_id
                    || binding.application_pid != window.application_pid
                    || window.unavailable_reason.is_some()
                    || !tokens.insert(&binding.selection_token)
                {
                    return Err(SourceError::Scope);
                }
            } else if window
                .unavailable_reason
                .as_ref()
                .is_none_or(String::is_empty)
            {
                return Err(SourceError::Identity);
            }
        }
        Ok(())
    }

    pub fn select(&self, selection_token: &str) -> Result<SourceTarget, SourceError> {
        self.validate()?;
        let window = self
            .windows
            .iter()
            .find(|window| {
                window
                    .binding
                    .as_ref()
                    .is_some_and(|binding| binding.selection_token == selection_token)
            })
            .ok_or(SourceError::Identity)?;
        let binding = window.binding.clone().ok_or(SourceError::Identity)?;
        Ok(SourceTarget::NativeWindow {
            binding,
            application_name: window.application_name.clone(),
            bundle_id: window.bundle_id.clone(),
            title: window.title.clone(),
            title_truncated: window.title_truncated,
            observed_monotonic_ns: self.observed_monotonic_ns,
            expires_monotonic_ns: self.expires_monotonic_ns,
        })
    }
}

impl SourceTarget {
    pub fn binding(&self) -> &WindowBinding {
        match self {
            Self::NativeWindow { binding, .. } => binding,
        }
    }

    pub fn validate(&self) -> Result<(), SourceError> {
        match self {
            Self::NativeWindow {
                binding,
                application_name,
                bundle_id,
                title,
                observed_monotonic_ns,
                expires_monotonic_ns,
                ..
            } => {
                binding.validate()?;
                validate_labels(application_name, bundle_id, title)?;
                validate_inventory_interval(*observed_monotonic_ns, *expires_monotonic_ns)
            }
        }
    }
}

impl SourceIntent {
    pub fn validate(&self) -> Result<(), SourceError> {
        if !crate::wire::canonical_uuid(&self.id)
            || [&self.device_id, &self.session_id].iter().any(|id| {
                id.as_ref().is_some_and(|id| {
                    id.is_empty() || id.len() > 128 || id.chars().any(char::is_control)
                })
            })
        {
            return Err(SourceError::Identity);
        }
        if self.annotation.len() > 4096 {
            return Err(SourceError::Bounds);
        }
        if self.requested_at_ms < 0
            || self.expires_at_ms <= self.requested_at_ms
            || self
                .expires_at_ms
                .checked_sub(self.requested_at_ms)
                .is_none_or(|ms| ms > 45_000)
        {
            return Err(SourceError::Interval);
        }
        self.target.validate()
    }
}

impl SourceMetadata {
    pub fn validate_for(&self, intent: &SourceIntent) -> Result<(), SourceError> {
        intent.validate()?;
        if self.origin != intent.target
            || self.clock_domain != "mach_absolute_time"
            || self.media.format() != intent.format
        {
            return Err(SourceError::Scope);
        }
        let SourceTarget::NativeWindow {
            observed_monotonic_ns,
            expires_monotonic_ns,
            ..
        } = &self.origin;
        if self.received_at_ms < 0
            || self.capture_started_monotonic_ns < *observed_monotonic_ns
            || self.capture_started_monotonic_ns >= *expires_monotonic_ns
            || self.capture_completed_monotonic_ns >= *expires_monotonic_ns
            || self.capture_completed_monotonic_ns < self.capture_started_monotonic_ns
            || self
                .capture_completed_monotonic_ns
                .saturating_sub(self.capture_started_monotonic_ns)
                >= match intent.format {
                    SourceFormat::Png => CAPTURE_TIMEOUT_NS,
                    SourceFormat::SelectedText => SELECTED_TEXT_TIMEOUT_NS,
                }
        {
            return Err(SourceError::Interval);
        }
        self.media.validate()
    }
}

impl SourceReceipt {
    pub fn validate(&self) -> Result<(), SourceError> {
        self.metadata.validate_for(&self.intent)?;
        if self.bytes == 0 || self.bytes > self.intent.format.max_bytes() as u64 {
            return Err(SourceError::Bounds);
        }
        if !valid_sha256(&self.sha256) {
            return Err(SourceError::Identity);
        }
        Ok(())
    }
}

fn validate_inventory_interval(observed: u64, expires: u64) -> Result<(), SourceError> {
    if observed == 0 || expires <= observed || expires - observed > INVENTORY_TTL_NS {
        return Err(SourceError::Interval);
    }
    Ok(())
}

fn validate_labels(application: &str, bundle: &str, title: &str) -> Result<(), SourceError> {
    if application.len() > 128 || bundle.len() > 256 || title.len() > 256 {
        return Err(SourceError::Bounds);
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    crate::wire::sha256_hex(value)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), SourceError> {
    if width == 0 || height == 0 || width > MAX_WINDOW_WIDTH || height > MAX_WINDOW_HEIGHT {
        return Err(SourceError::Bounds);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
