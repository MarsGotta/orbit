use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

use super::*;

/// Swift omits absent optionals. A present null is not that wire form and must
/// not silently become an omitted category, field, failure or scope.
pub(super) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CalendarError> {
    if bytes.len() > MAX_NATIVE_PAGE_BYTES {
        return Err(CalendarError::LimitExceeded);
    }
    serde_json::from_slice(bytes).map_err(|_| CalendarError::InvalidWire)
}

pub(super) fn size(value: &impl Serialize) -> Result<(), CalendarError> {
    // All strings are bounded before this encoding. JSON escaping still counts
    // toward the whole message; no truncation is performed.
    if serde_json::to_vec(value)
        .map_err(|_| CalendarError::InvalidWire)?
        .len()
        > MAX_NATIVE_PAGE_BYTES
    {
        return Err(CalendarError::LimitExceeded);
    }
    Ok(())
}

pub fn decode_status(bytes: &[u8]) -> Result<CalendarStatus, CalendarError> {
    let value: CalendarStatus = decode(bytes)?;
    value.validate()?;
    Ok(value)
}

/// Structural validation is not authority. Call `validate_response` with the
/// actor's current facts and exact query before admitting a decoded page.
pub fn decode_read_page(bytes: &[u8]) -> Result<CalendarReadPage, CalendarError> {
    let value: CalendarReadPage = decode(bytes)?;
    value.validate()?;
    Ok(value)
}

pub fn decode_inventory_page(bytes: &[u8]) -> Result<CalendarInventoryPage, CalendarError> {
    let value: CalendarInventoryPage = decode(bytes)?;
    value.validate()?;
    Ok(value)
}
