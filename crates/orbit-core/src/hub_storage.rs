//! The hub's storage as the Mac says it: one notice when the hub becomes
//! nearly full, with what it uses, its limit and what is left.
//!
//! The hub decides what nearly full is (`STORAGE_WARNING_PERCENT` in
//! `server/orbit_hub/storage.py`) and says it in every `ready` and in
//! `storage.status`; the Mac only reads it, so the share lives in one place.
//! Pure: the daemon passes what it said before.
use crate::notice::{Notice, NoticeCode, NoticeKey};
use crate::wire::hub::HubStorage;

/// The notice for `storage` when the hub has just become nearly full:
/// `previous` is whether it was, as this Mac last heard it (`None` before
/// any word). A reconnection to a hub that stays nearly full says nothing
/// again; a hub that frees room and fills again does.
pub fn storage_notice(previous: Option<bool>, storage: &HubStorage) -> Option<Notice> {
    (storage.nearly_full && previous != Some(true)).then(|| {
        Notice::new(NoticeCode::HubStorageNearlyFull)
            .with(NoticeKey::Bytes, storage.used_bytes)
            .with(NoticeKey::MaxBytes, storage.max_bytes)
            .with(NoticeKey::Remaining, storage.remaining_bytes)
            .with_token(NoticeKey::Cause, &storage.limit)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notice::{DataValue, Token};

    fn storage(nearly_full: bool) -> HubStorage {
        HubStorage {
            used_bytes: 82_000_000_000,
            max_bytes: 100_000_000_000,
            remaining_bytes: 18_000_000_000,
            limit: "disk".into(),
            nearly_full,
        }
    }

    #[test]
    fn a_hub_that_becomes_nearly_full_is_said_with_its_numbers_and_its_limit() {
        let Some(notice) = storage_notice(None, &storage(true)) else {
            panic!("a nearly full hub must be said");
        };
        assert_eq!(notice.code, NoticeCode::HubStorageNearlyFull);
        assert_eq!(
            notice.data.get(&NoticeKey::Bytes),
            Some(&DataValue::Int(82_000_000_000))
        );
        assert_eq!(
            notice.data.get(&NoticeKey::MaxBytes),
            Some(&DataValue::Int(100_000_000_000))
        );
        assert_eq!(
            notice.data.get(&NoticeKey::Remaining),
            Some(&DataValue::Int(18_000_000_000))
        );
        assert_eq!(
            notice.data.get(&NoticeKey::Cause),
            Token::new("disk").map(DataValue::Token).as_ref()
        );
        assert_eq!(storage_notice(Some(false), &storage(true)), Some(notice));
    }

    #[test]
    fn a_hub_that_stays_nearly_full_is_not_said_again_and_room_is_not_said() {
        assert_eq!(storage_notice(Some(true), &storage(true)), None);
        assert_eq!(storage_notice(None, &storage(false)), None);
        assert_eq!(storage_notice(Some(true), &storage(false)), None);
    }

    #[test]
    fn a_limit_that_is_not_a_token_is_kept_as_unrecognized_never_as_text() {
        let mut odd = storage(true);
        odd.limit = "Disco de Valeria".into();
        let Some(notice) = storage_notice(None, &odd) else {
            panic!("a nearly full hub must be said");
        };
        assert_eq!(
            notice.data.get(&NoticeKey::Cause),
            Some(&DataValue::Token(Token::unrecognized()))
        );
    }
}
