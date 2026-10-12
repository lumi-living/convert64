//! Standard base64 (RFC 4648 §4, with `=` padding) for sources and masks embedded in a
//! project file. Small enough to keep here rather than add a dependency.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` as padded base64.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> shift & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn value(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes padded or unpadded base64; ASCII whitespace (line breaks) is ignored.
pub(crate) fn decode(text: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let body = digits
        .iter()
        .position(|&c| c == b'=')
        .map_or(&digits[..], |p| &digits[..p]);
    let padding = digits.len() - body.len();
    if digits[body.len()..].iter().any(|&c| c != b'=') || padding > 2 {
        return Err("base64: '=' may only end the data".to_owned());
    }
    if body.len() % 4 == 1 || (padding > 0 && !digits.len().is_multiple_of(4)) {
        return Err("base64: truncated data".to_owned());
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    for chunk in body.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = value(c).ok_or_else(|| format!("base64: invalid character {:?}", c as char))?;
            n |= v << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};

    #[test]
    fn rfc_4648_test_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), coded);
            assert_eq!(decode(coded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn every_byte_round_trips() {
        let all: Vec<u8> = (0..=255).chain((0..=255).rev()).collect();
        for len in 0..all.len() {
            assert_eq!(decode(&encode(&all[..len])).unwrap(), &all[..len]);
        }
        assert_eq!(decode("Zm9v\nYmFy\n").unwrap(), b"foobar");
        assert_eq!(decode("Zm8").unwrap(), b"fo");
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["Zm9v!", "Z", "Zg=a", "Zg===", "Zm9=v"] {
            assert!(decode(bad).is_err(), "{bad}");
        }
    }
}
