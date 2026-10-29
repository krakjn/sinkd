use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home().join(".config"), PathBuf::from).join("sinkd")
}

pub fn state_dir(app: &str) -> PathBuf {
    if let Some(path) = std::env::var_os("SINKD_STATE_DIR") {
        return PathBuf::from(path);
    }
    std::env::var_os("XDG_STATE_HOME")
        .map_or_else(|| home().join(".local/state"), PathBuf::from)
        .join(app)
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

pub fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home().join(rest);
    }
    if path == "~" {
        return home();
    }
    PathBuf::from(path)
}

pub fn user_name() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".to_string())
}

pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return "host".to_string();
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

pub fn sanitize(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "x".to_string()
    } else {
        cleaned
    }
}

#[must_use]
pub fn anchor_id(path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[must_use]
pub fn anchor_key(user: &str, id: &str, share: bool) -> String {
    if share {
        format!("share/{id}")
    } else {
        format!("{}/{id}", sanitize(user))
    }
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    let mut file = std::fs::File::open("/dev/urandom").map_err(|e| e.to_string())?;
    std::io::Read::read_exact(&mut file, &mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

pub fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("odd hex length".into());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_val(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(format!("bad hex byte {byte}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_id_is_stable() {
        assert_eq!(anchor_id("/data/notes"), anchor_id("/data/notes"));
        assert_ne!(anchor_id("/data/notes"), anchor_id("/data/other"));
    }

    #[test]
    fn hex_roundtrip() {
        let bytes = [0u8, 255, 16, 10];
        let text = hex_encode(&bytes);
        assert_eq!(hex_decode(&text).expect("decode"), bytes);
    }
}
