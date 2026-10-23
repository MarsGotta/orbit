//! The notices kept in the state, readers for those saved by any version,
//! and the line each one becomes in the local report log.
use super::{Notice, NoticeClass, NoticeCode, NoticeData, Op, Token};
use serde::{Deserialize, Deserializer, Serialize};

/// Reads a notice written by any version. A notice from before notices were
/// typed was a sentence and is not carried forward; one a newer daemon wrote with data
/// this build cannot read keeps its op, as [`NoticeCode::Unknown`], so the
/// person still sees the generic failure with its code instead of nothing.
pub fn lenient_notice<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Notice>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(read_notice(value))
}

/// [`lenient_notice`] for a list: each element is read on its own.
pub fn lenient_notices<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Notice>, D::Error> {
    let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(values.into_iter().filter_map(read_notice).collect())
}

fn read_notice(value: serde_json::Value) -> Option<Notice> {
    // A sentence, or a `{text}` record, from before typed notices has no code.
    value.get("code").and_then(serde_json::Value::as_str)?;
    let op = value
        .get("op")
        .and_then(serde_json::Value::as_str)
        .and_then(Op::parse);
    let sequence = value.get("sequence").and_then(serde_json::Value::as_u64);
    serde_json::from_value(value).ok().or(Some(Notice {
        code: NoticeCode::Unknown,
        op,
        sequence,
        data: NoticeData::default(),
    }))
}

/// How many notices survive in the state. Enough for a stop, its
/// confirmation failure and the warnings that can follow one another; short
/// enough that the state stays small however long the session runs. The
/// local report log keeps every notice; this is only the recent sequence.
pub const CAPACITY: usize = 8;

/// The last notices in the order they happened, oldest dropped beyond
/// [`CAPACITY`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct NoticeLog {
    records: Vec<Notice>,
    #[serde(skip)]
    op: Option<Op>,
}

impl NoticeLog {
    pub fn record(&mut self, mut notice: Notice) -> Notice {
        notice.op = notice.op.or_else(|| self.op.clone());
        notice.sequence = Some(self.last_sequence().saturating_add(1));
        self.records.push(notice.clone());
        let excess = self.records.len().saturating_sub(CAPACITY);
        if excess > 0 {
            self.records.drain(..excess);
        }
        notice
    }

    pub fn last_sequence(&self) -> u64 {
        self.records
            .last()
            .and_then(|notice| notice.sequence)
            .unwrap_or(0)
    }

    /// The daemon passes the producing turn explicitly; pure decisions clone it.
    pub fn begin_turn(&mut self, op: Op) {
        self.op = Some(op);
    }

    pub fn records(&self) -> &[Notice] {
        &self.records
    }

    pub fn latest(&self) -> Option<&Notice> {
        self.records.last()
    }
}

// The producing context is not a published or durable change.
impl PartialEq for NoticeLog {
    fn eq(&self, other: &Self) -> bool {
        self.records == other.records
    }
}
impl Eq for NoticeLog {}

impl<'de> Deserialize<'de> for NoticeLog {
    /// Keeps every record that is a typed notice; sentences saved before
    /// notices were typed are dropped rather than refusing the whole saved state.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            records: Vec<serde_json::Value>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let records: Vec<Notice> = raw
            .records
            .into_iter()
            .filter_map(read_notice)
            .take(CAPACITY)
            .collect();
        Ok(Self { records, op: None })
    }
}

/// Who wrote a report line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSource {
    Ui,
    App,
    Daemon,
    Hub,
}

/// One line of the local report log, without its time: the writer adds it.
/// `code` is a token because the log also takes codes the interface met
/// (a refusal, a code from a newer daemon, a view that failed to paint).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub op: Op,
    pub code: Token,
    pub class: NoticeClass,
    pub source: ReportSource,
    #[serde(default)]
    pub data: NoticeData,
}

impl Report {
    /// Whether this line says something failed (only these leave for
    /// Sentry). A line the interface reports as `failed` (a view that
    /// failed to paint, an uncaught error, a code it has no entry for) is a
    /// failure; any other is one when its code is a [`NoticeCode`] that
    /// [`NoticeCode::is_failure`] says is.
    pub fn is_failure(&self) -> bool {
        self.class == NoticeClass::Failed
            || serde_json::from_value::<NoticeCode>(serde_json::Value::from(self.code.as_str()))
                .is_ok_and(NoticeCode::is_failure)
    }

    pub fn of_notice(notice: &Notice, op: Op, source: ReportSource) -> Self {
        let code = Token::new(&notice.code.name()).unwrap_or_else(Token::unrecognized);
        Self {
            op,
            code,
            class: notice.class(),
            source,
            data: notice.data.clone(),
        }
    }
}
