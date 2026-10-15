//! NFC codec: UID <-> NFCID hex <-> CODENAME (base36) and the handshake key.
//!
//! Values are reverse-engineered from the firmware (`Utils::Base36`,
//! `WLAN::ReadSecretKey`, `Utils::printDirectory`) and the Android client
//! (`EditID.ByteToBase36` / `TCPTransfer.RevertBase36`).

const BASE36: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Number of base36 characters used for a code name before the dot (8 + 3).
pub const CODENAME_LEN: usize = 11;

/// Encode a `u64` as an uppercase base36 string (no padding).
pub fn u64_to_base36(mut value: u64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut buf = Vec::with_capacity(13);
    while value > 0 {
        buf.push(BASE36[(value % 36) as usize]);
        value /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).expect("base36 is ASCII")
}

/// Encode a `u64` card id into the device file naming: `IIIIIIII.FFF`.
///
/// Exactly mirrors Android `EditID.ByteToBase36`: pad to at least 11 chars,
/// then put a dot before the last three characters.
pub fn u64_to_codename(value: u64) -> String {
    let mut s = u64_to_base36(value);
    while s.len() < CODENAME_LEN {
        s.insert(0, '0');
    }
    if s.len() <= 3 {
        return s;
    }
    let split = s.len() - 3;
    format!("{}.{}", &s[..split], &s[split..])
}

/// Interpret UID bytes the same way the firmware union and Android do:
/// little-endian (byte 0 is the least significant).
#[allow(dead_code)]
pub fn uid_bytes_to_u64(uid: &[u8]) -> u64 {
    let mut v = 0u64;
    for (i, b) in uid.iter().take(8).enumerate() {
        v |= (*b as u64) << (8 * i as u64);
    }
    v
}

/// Decode a CODENAME (`IIIIIIII.FFF`) back into a `u64`.
pub fn codename_to_u64(codename: &str) -> Result<u64, String> {
    let cleaned: String = codename
        .trim()
        .chars()
        .filter(|c| *c != '.')
        .collect::<String>()
        .to_uppercase();
    if cleaned.is_empty() {
        return Err("empty code name".to_string());
    }
    let mut v: u128 = 0;
    for ch in cleaned.chars() {
        let d: u128 = match ch {
            '0'..='9' => (ch as u128) - ('0' as u128),
            'A'..='Z' => (ch as u128) - ('A' as u128) + 10,
            other => return Err(format!("invalid base36 character '{other}'")),
        };
        v = v
            .checked_mul(36)
            .and_then(|x| x.checked_add(d))
            .ok_or_else(|| "code name is too large".to_string())?;
        if v > u64::MAX as u128 {
            return Err("code name is out of range".to_string());
        }
    }
    Ok(v as u64)
}

/// `u64` -> uppercase hex, no leading zeros (Android `toString(16).toUpperCase()`).
pub fn u64_to_nfcid(value: u64) -> String {
    format!("{value:X}")
}

/// CODENAME -> NFCID hex string.
pub fn codename_to_nfcid(codename: &str) -> Result<String, String> {
    codename_to_u64(codename).map(u64_to_nfcid)
}

/// Parse a user-entered hex id (accepts optional `0x`, spaces, `:` and `-`).
pub fn parse_nfcid(input: &str) -> Result<u64, String> {
    let cleaned: String = input
        .trim()
        .replace("0x", "")
        .replace("0X", "")
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    if cleaned.is_empty() {
        return Err("empty id".to_string());
    }
    u64::from_str_radix(&cleaned, 16).map_err(|e| format!("invalid hex id: {e}"))
}

/// NFCID hex -> CODENAME.
pub fn nfcid_to_codename(nfcid: &str) -> Result<String, String> {
    parse_nfcid(nfcid).map(u64_to_codename)
}

/// Build the 8-byte handshake key the firmware `ReadSecretKey` validates.
///
/// `month` is passed as-is (the caller decides 0- or 1-based).
pub fn build_secret_key(year: u16, month: u8, day: u8) -> [u8; 8] {
    let checksum = (year as u32 + month as u32 + day as u32) as u16;
    [
        (checksum >> 8) as u8,
        (checksum & 0xFF) as u8,
        (year >> 8) as u8,
        (year & 0xFF) as u8,
        month,
        day,
        (checksum & 0xFF) as u8,
        (checksum >> 8) as u8,
    ]
}

/// A single `CODENAME,count` record received from the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRecord {
    pub codename: String,
    pub count: u32,
}

/// Parse one device line (`CODENAME,count`). Returns `None` for blank lines.
pub fn parse_transfer_line(line: &str) -> Option<TransferRecord> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (code, count) = line.rsplit_once(',')?;
    let code = code.trim();
    if code.is_empty() {
        return None;
    }
    let count: u32 = count.trim().parse().ok()?;
    Some(TransferRecord {
        codename: code.to_string(),
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codename_roundtrip_known_value() {
        // base36(0xBB36AB22) == "1FY0V1E" -> padded "00001FY0V1E" -> "00001FY0.V1E"
        // (the firmware's dot-before-last-3 rule produces the exact same string).
        let v = 0xBB36AB22u64;
        assert_eq!(u64_to_base36(v), "1FY0V1E");
        assert_eq!(u64_to_codename(v), "00001FY0.V1E");
        assert_eq!(codename_to_u64("00001FY0.V1E").unwrap(), v);
        assert_eq!(codename_to_nfcid("00001FY0.V1E").unwrap(), "BB36AB22");
    }

    #[test]
    fn codename_short_values_are_dotted_before_last_three() {
        assert_eq!(u64_to_codename(0), "00000000.000");
        assert_eq!(u64_to_codename(1), "00000000.001");
        assert_eq!(codename_to_u64("00000000.001").unwrap(), 1);
    }

    #[test]
    fn uid_bytes_little_endian() {
        assert_eq!(uid_bytes_to_u64(&[0x22, 0xAB, 0x36, 0xBB]), 0xBB36AB22);
    }

    #[test]
    fn secret_key_layout() {
        // year=2026, month=9 (0-based Oct), day=14 -> sum 2049 = 0x0801
        let k = build_secret_key(2026, 9, 14);
        assert_eq!(k[0], 0x08);
        assert_eq!(k[1], 0x01);
        assert_eq!(k[2], (2026u16 >> 8) as u8);
        assert_eq!(k[3], (2026 & 0xFF) as u8);
        assert_eq!(k[4], 9);
        assert_eq!(k[5], 14);
        assert_eq!(k[6], k[1]);
        assert_eq!(k[7], k[0]);
        // firmware validation
        let chk1 = ((k[0] as u16) << 8) | k[1] as u16;
        let chk2 = ((k[7] as u16) << 8) | k[6] as u16;
        assert_eq!(chk1, chk2);
        assert_eq!(2026u16 + 9 + 14, chk1);
    }

    #[test]
    fn transfer_line_parsing() {
        let r = parse_transfer_line("01FZ96AQ.000,42\r").unwrap();
        assert_eq!(r.codename, "01FZ96AQ.000");
        assert_eq!(r.count, 42);
        assert!(parse_transfer_line("   ").is_none());
        assert!(parse_transfer_line("garbage").is_none());
    }
}
