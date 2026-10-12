pub fn decode(mime: &str, encoded: &str) -> Option<(&'static str, Vec<u8>)> {
    if encoded.len() > 350_000 || !encoded.len().is_multiple_of(4) {
        return None;
    }
    let mut data = Vec::with_capacity(encoded.len() / 4 * 3);
    for (index, chunk) in encoded.as_bytes().as_chunks::<4>().0.iter().enumerate() {
        let value = |byte| match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            b'=' => Some(0),
            _ => None,
        };
        let a = value(chunk[0])?;
        let b = value(chunk[1])?;
        let c = value(chunk[2])?;
        let d = value(chunk[3])?;
        let last = index + 1 == encoded.len() / 4;
        if (!last && (chunk[2] == b'=' || chunk[3] == b'=')) || chunk[2] == b'=' && chunk[3] != b'='
        {
            return None;
        }
        data.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            data.push((b << 4) | (c >> 2));
        }
        if chunk[3] != b'=' {
            data.push((c << 6) | d);
        }
        if data.len() > 256 * 1024 {
            return None;
        }
    }

    let mime = mime.split(';').next()?.trim().to_ascii_lowercase();
    let ext = match mime.as_str() {
        "image/png" if data.starts_with(b"\x89PNG\r\n\x1a\n") => "png",
        "image/jpeg" if data.starts_with(&[0xff, 0xd8, 0xff]) => "jpg",
        "image/gif" if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") => "gif",
        "image/webp" if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") => "webp",
        "image/svg+xml"
            if std::str::from_utf8(&data)
                .ok()?
                .trim_start_matches('\u{feff}')
                .trim_start()
                .to_ascii_lowercase()
                .contains("<svg") =>
        {
            "svg"
        }
        "image/x-icon" | "image/vnd.microsoft.icon"
            if data.starts_with(&[0, 0, 1, 0]) && data.len() >= 6 =>
        {
            "ico"
        }
        _ => return None,
    };
    Some((ext, data))
}

pub fn host_key(host: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in host.to_ascii_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn accepts_png_and_rejects_mismatched_magic() {
        let png = b"\x89PNG\r\n\x1a\n";
        let mut encoded = String::new();
        for chunk in png.chunks(3) {
            let a = chunk[0];
            let b = *chunk.get(1).unwrap_or(&0);
            let c = *chunk.get(2).unwrap_or(&0);
            let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            encoded.push(chars[(a >> 2) as usize] as char);
            encoded.push(chars[(((a & 3) << 4) | (b >> 4)) as usize] as char);
            encoded.push(if chunk.len() > 1 {
                chars[(((b & 15) << 2) | (c >> 6)) as usize] as char
            } else {
                '='
            });
            encoded.push(if chunk.len() > 2 {
                chars[(c & 63) as usize] as char
            } else {
                '='
            });
        }
        assert_eq!(decode("image/png", &encoded).unwrap().0, "png");
        assert!(decode("image/jpeg", &encoded).is_none());
    }
}
