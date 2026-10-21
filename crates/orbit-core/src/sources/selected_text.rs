use super::{MAX_SELECTED_TEXT_BYTES, SelectedTextRange, SourceError};

pub(super) fn validate(bytes: &[u8], range: &SelectedTextRange) -> Result<(), SourceError> {
    if bytes.len() > MAX_SELECTED_TEXT_BYTES {
        return Err(SourceError::Bounds);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SourceError::Text)?;
    if text.contains('\0') || text.encode_utf16().count() as u64 != range.length {
        return Err(SourceError::Text);
    }
    Ok(())
}
