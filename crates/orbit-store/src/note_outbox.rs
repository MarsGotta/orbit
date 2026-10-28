//! The notes outbox (`note_outbox` table): the orders the user gave about a
//! meeting's note and about the library, each kept until the hub took it
//! (`orbit_core::note_outbox`).
//!
//! One row per lane and target. A newer order on the same target replaces
//! the row, merged as `QueuedOrder::superseded_by` decides, and takes a new
//! sequence: the sequence orders a lane and is the version a delivery names,
//! so a row that a newer order replaced while the older one was on its way
//! stays to be sent.
//!
//! Each row is AEAD-sealed with its lane and target as associated data, so a
//! row moved to another lane or target does not authenticate. Only the store
//! actor writes.
use crate::{
    Error, MAX_PAYLOAD_BYTES, Result, Store,
    crypto::OVERHEAD,
    validation::{bounded_blob, bounded_json, context},
};
use orbit_core::note_outbox::{MAX_WAITING_ORDERS, QueuedOrder};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

const KIND: &str = "note_outbox_order";
/// Longest target: a template id or a calendar series id with its prefix.
/// Both are bounded on the hub far below this; it bounds the row key, not
/// what the order says.
const MAX_TARGET_BYTES: usize = 512;

/// One order waiting for the hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingOrder {
    pub target: String,
    /// The order it was given in, and the version a delivery names.
    pub sequence: i64,
    pub order: QueuedOrder,
}

fn sealed_context(lane: &str, target: &str) -> Vec<u8> {
    context(KIND, &format!("{lane}|{target}"))
}

fn check_target(target: &str) -> Result<()> {
    if target.is_empty() || target.len() > MAX_TARGET_BYTES {
        return Err(Error::Invalid("note outbox target"));
    }
    Ok(())
}

impl Store {
    /// Keeps `order` until the hub takes it, replacing the order on the same
    /// target of its lane. Returns what waits now, with its new sequence.
    pub fn queue_note_order(
        &mut self,
        target: &str,
        order: QueuedOrder,
        now_ms: i64,
    ) -> Result<WaitingOrder> {
        check_target(target)?;
        let lane = order.lane().key();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let older = transaction
            .query_row(
                "SELECT length(payload),payload FROM note_outbox WHERE lane=?1 AND target=?2",
                params![lane, target],
                |row| bounded_blob(row, 1, MAX_PAYLOAD_BYTES + OVERHEAD, "A note command"),
            )
            .optional()?;
        let order = match older {
            Some(sealed) => {
                let clear = self
                    .cipher
                    .decrypt(&sealed_context(&lane, target), &sealed)?;
                serde_json::from_slice::<QueuedOrder>(&clear)?.superseded_by(order)
            }
            None => order,
        };
        let sequence: i64 = transaction.query_row(
            "SELECT coalesce(max(sequence),0)+1 FROM note_outbox",
            [],
            |row| row.get(0),
        )?;
        let bytes = bounded_json(&order, MAX_PAYLOAD_BYTES)?;
        let payload = self
            .cipher
            .encrypt(&sealed_context(&lane, target), &bytes)?;
        transaction.execute(
            "INSERT INTO note_outbox(lane,target,sequence,created_at,payload) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(lane,target) DO UPDATE SET sequence=excluded.sequence,
                 created_at=excluded.created_at, payload=excluded.payload",
            params![lane, target, sequence, now_ms, payload],
        )?;
        transaction.commit()?;
        Ok(WaitingOrder {
            target: target.to_owned(),
            sequence,
            order,
        })
    }

    /// Every order waiting, oldest first, at most `MAX_WAITING_ORDERS`.
    pub fn note_orders(&self) -> Result<Vec<WaitingOrder>> {
        let mut statement = self.connection.prepare(
            "SELECT lane,target,sequence,length(payload),payload FROM note_outbox
             ORDER BY sequence LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![MAX_WAITING_ORDERS as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    bounded_blob(row, 4, MAX_PAYLOAD_BYTES + OVERHEAD, "A note command")?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(lane, target, sequence, sealed)| {
                let clear = self
                    .cipher
                    .decrypt(&sealed_context(&lane, &target), &sealed)?;
                let order: QueuedOrder = serde_json::from_slice(&clear)?;
                let named = order.target().is_none_or(|named| named == target);
                if order.lane().key() != lane || !named {
                    return Err(Error::Authentication);
                }
                Ok(WaitingOrder {
                    target,
                    sequence,
                    order,
                })
            })
            .collect()
    }

    /// The hub answered the order of `sequence` (taken, or refused for
    /// good): the row leaves the Mac, unless a newer order replaced it
    /// meanwhile, which stays to be sent. Returns whether it left.
    pub fn remove_note_order(&mut self, lane: &str, target: &str, sequence: i64) -> Result<bool> {
        let removed = self.connection.execute(
            "DELETE FROM note_outbox WHERE lane=?1 AND target=?2 AND sequence=?3",
            params![lane, target, sequence],
        )?;
        Ok(removed > 0)
    }
}

#[cfg(test)]
mod tests;
