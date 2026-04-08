/// Shared helpers used across the parser, layout, and renderer.

/// Decode a standard Base64 string without pulling in an extra dependency.
pub(crate) fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let table = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };

    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;

    while i < bytes.len() {
        let remaining = bytes.len() - i;
        if remaining < 2 {
            break;
        }

        let a = table(bytes[i])?;
        let b = table(bytes[i + 1])?;
        result.push((a << 2) | (b >> 4));

        if i + 2 < bytes.len() && bytes[i + 2] != b'=' {
            let c = table(bytes[i + 2])?;
            result.push((b << 4) | (c >> 2));

            if i + 3 < bytes.len() && bytes[i + 3] != b'=' {
                let d = table(bytes[i + 3])?;
                result.push((c << 6) | d);
            }
        }

        i += 4;
    }

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::decode_base64;

    #[test]
    fn decode_base64_basic() {
        assert_eq!(
            decode_base64("SGVsbG8=").as_deref(),
            Some(b"Hello".as_ref())
        );
    }

    #[test]
    fn decode_base64_with_whitespace() {
        assert_eq!(
            decode_base64("SGVs\nbG8=").as_deref(),
            Some(b"Hello".as_ref())
        );
    }
}
