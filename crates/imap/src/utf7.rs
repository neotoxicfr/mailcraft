//! IMAP folder names travel in "modified UTF-7" (RFC 3501 §5.1.3): `Envoy&AOk-s` is "Envoyés".

use base64::Engine;
use base64::alphabet::IMAP_MUTF7;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

const ENGINE: GeneralPurpose = GeneralPurpose::new(
    &IMAP_MUTF7,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

/// Folder name as shown to the user. A malformed name is shown as it is.
pub fn decode(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            return name.to_owned();
        };
        let chunk = &after[..end];
        if chunk.is_empty() {
            out.push('&');
        } else {
            let Ok(bytes) = ENGINE.decode(chunk) else {
                return name.to_owned();
            };
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_be_bytes(*p))
                .collect();
            match String::from_utf16(&units) {
                Ok(s) => out.push_str(&s),
                Err(_) => return name.to_owned(),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Name to send to the server for a folder the user typed.
pub fn encode(name: &str) -> String {
    let mut out = String::new();
    let mut pending: Vec<u16> = Vec::new();
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if !pending.is_empty() {
            let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
            out.push('&');
            out.push_str(&ENGINE.encode(bytes));
            out.push('-');
            pending.clear();
        }
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}
