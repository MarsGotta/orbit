/// Validate the limited mailto representation used by the native reader. This
/// preserves case and UTF-8, does not repair malformed escapes, and does not
/// validate deliverability or map the candidate to a vault person.
pub(super) fn candidate(value: &str) -> Option<String> {
    if value.len() > 4096
        || !value.get(..7)?.eq_ignore_ascii_case("mailto:")
        || super::validation::ascii_control(value)
    {
        return None;
    }
    let path = value.get(7..)?;
    if path.starts_with("//") || path.contains(['?', '#']) {
        return None;
    }
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = hexadecimal(bytes.next()?)?;
            let low = hexadecimal(bytes.next()?)?;
            decoded.push(high * 16 + low);
        } else {
            decoded.push(byte);
        }
    }
    let address = String::from_utf8(decoded).ok()?;
    if address.is_empty()
        || address.len() > 320
        || address.chars().any(|ch| {
            ch.is_whitespace() || ch < ' ' || ch == '\u{7f}' || "<>,;:\"\\[]".contains(ch)
        })
    {
        return None;
    }
    let (local, domain) = address.split_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return None;
    }
    Some(address)
}

fn hexadecimal(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
