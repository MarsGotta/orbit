//! What a notice is made of: the op of its action, the closed keys of its
//! data and the values they admit, none of which can carry free text.
use super::{NoticeClass, NoticeCode};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

/// The identifier of one action from end to end: the order in the
/// interface, the daemon, the request to the hub, the logs and the notice.
/// Shared contract with the hub: 8 to 64 characters of
/// `[A-Za-z0-9-]`, so a UUID fits and nothing else rides along.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Op(String);

impl Op {
    pub const MIN_LEN: usize = 8;
    pub const MAX_LEN: usize = 64;

    pub fn parse(raw: &str) -> Option<Self> {
        let valid = (Self::MIN_LEN..=Self::MAX_LEN).contains(&raw.len())
            && raw
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        valid.then(|| Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The code the generic failure shows: the first six characters, in
    /// capitals so it reads the same aloud as in the log.
    pub fn short(&self) -> String {
        self.0
            .chars()
            .take(6)
            .collect::<String>()
            .to_ascii_uppercase()
    }
}

impl From<uuid::Uuid> for Op {
    /// Every UUID is a valid op: 36 characters of hex digits and hyphens.
    fn from(value: uuid::Uuid) -> Self {
        Self(value.hyphenated().to_string())
    }
}

impl std::fmt::Display for Op {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Op {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| serde::de::Error::custom("op must be 8-64 [A-Za-z0-9-]"))
    }
}

macro_rules! notice_keys {
    ($($variant:ident,)*) => {
        /// The names the data of a notice can have: a closed set, so a key
        /// cannot smuggle text either.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[cfg_attr(
            feature = "ts",
            derive(ts_rs::TS),
            ts(export_to = "../../apps/desktop/src/bridge/generated/")
        )]
        #[serde(rename_all = "snake_case")]
        pub enum NoticeKey {
            $($variant,)*
        }

        impl NoticeKey {
            pub const ALL: &'static [NoticeKey] = &[$(Self::$variant,)*];

            /// Whether this key's token is a closed enumeration: a code some
            /// component already speaks (the daemon, the helper, the hub),
            /// never a name or a word somebody said. Only these tokens may
            /// leave the Mac in an error report; any other key's
            /// token stays in the local log.
            pub const fn closed(self) -> bool {
                matches!(self, Self::Cause | Self::Code | Self::Origin | Self::Step)
            }
        }
    };
}

notice_keys! {
    Cause,
    Count,
    Minutes,
    Fragments,
    Recording,
    Call,
    VoiceNote,
    Closing,
    Origin,
    Attributed,
    Code,
    Step,
    Recovered,
    Invalid,
    Missing,
    Incomplete,
    Interrupted,
    Stage,
    Attempt,
    Max,
    Remaining,
    Stored,
    Total,
    Pages,
    Bytes,
    Status,
    RetryAtMs,
    Failed,
    Omitted,
    OmittedCount,
    Returned,
    MaxBytes,
    AudioUnsaved,
    ReserveExhausted,
    ElapsedMs,
    SilentMs,
    AtMs,
    Turns,
    DurationMs,
    MinimumMs,
    // The numbers and closed tokens of a turn that ended without an answer
    // (hub-v1: generic providers and official assistant tools).
    ProviderStatus,
    RetryAfterMs,
    TimeoutMs,
    HistoryChars,
    Tokens,
    MaxTokens,
    MaxRequests,
    AcpCode,
    RpcCode,
    ExitStatus,
    StopReason,
    ResetsAtMs,
}

/// A short machine value: a code a component already speaks (`privacy_stopped`,
/// `microphone_permission_required`), never a sentence or a name. 1 to 64
/// characters of `[a-z0-9_.-]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Token(String);

impl Token {
    pub const MAX_LEN: usize = 64;

    pub fn new(raw: &str) -> Option<Self> {
        let valid = (1..=Self::MAX_LEN).contains(&raw.len())
            && raw.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'.' | b'-')
            });
        valid.then(|| Self(raw.to_owned()))
    }

    /// What a value that is not a token becomes: the notice keeps the fact
    /// that there was a value, never the value.
    pub fn unrecognized() -> Self {
        Self("unrecognized".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Token {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::new(&raw).ok_or_else(|| serde::de::Error::custom("token must be 1-64 [a-z0-9_.-]"))
    }
}

/// One value of a notice's data. There is deliberately no conversion from
/// `&str` or `String`, so free text does not compile:
/// ```compile_fail
/// use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
/// let _ = Notice::new(NoticeCode::CaptureCut).with(NoticeKey::Cause, "Alguien dijo algo privado");
/// ```
/// ```compile_fail
/// use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
/// let _ = Notice::new(NoticeCode::CaptureCut)
///     .with(NoticeKey::Cause, String::from("Alguien dijo algo privado"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
#[serde(untagged)]
pub enum DataValue {
    Bool(bool),
    Int(#[cfg_attr(feature = "ts", ts(type = "number"))] i64),
    Token(#[cfg_attr(feature = "ts", ts(type = "string"))] Token),
}

impl From<bool> for DataValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for DataValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<u64> for DataValue {
    fn from(value: u64) -> Self {
        Self::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<u32> for DataValue {
    fn from(value: u32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<u16> for DataValue {
    fn from(value: u16) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<usize> for DataValue {
    fn from(value: usize) -> Self {
        Self::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<Token> for DataValue {
    fn from(value: Token) -> Self {
        Self::Token(value)
    }
}

/// The data of a notice: closed keys to numbers, booleans and tokens.
pub type NoticeData = BTreeMap<NoticeKey, DataValue>;

/// One notice: what happened, to which action, with which numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export_to = "../../apps/desktop/src/bridge/generated/")
)]
pub struct Notice {
    pub code: NoticeCode,
    /// The producing turn's op, inherited when the notice enters its log.
    /// Unscoped domain decisions can construct a notice without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "string"))]
    pub op: Option<Op>,
    /// Assigned once on insertion; independent of code, data and op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional, type = "number"))]
    pub sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<NoticeData>", optional))]
    pub data: NoticeData,
}

impl Notice {
    pub fn new(code: NoticeCode) -> Self {
        Self {
            code,
            op: None,
            sequence: None,
            data: NoticeData::default(),
        }
    }

    pub fn with(mut self, key: NoticeKey, value: impl Into<DataValue>) -> Self {
        self.data.insert(key, value.into());
        self
    }

    /// Adds a code another component gave (a helper or hub code). A value
    /// that is not a token is kept as `unrecognized`, never as text.
    pub fn with_token(self, key: NoticeKey, raw: &str) -> Self {
        let token = Token::new(raw).unwrap_or_else(Token::unrecognized);
        self.with(key, token)
    }

    pub fn class(&self) -> NoticeClass {
        self.code.class()
    }
}

impl From<NoticeCode> for Notice {
    fn from(code: NoticeCode) -> Self {
        Self::new(code)
    }
}
