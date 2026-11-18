//! Link-frame checksum: RFC 1071 Internet checksum over the whole frame.

/// Byte offset of the 16-bit checksum field inside a link frame.
pub const CHECKSUM_OFFSET: usize = 18;

/// Ones'-complement sum of big-endian 16-bit words over `frame`, treating the checksum field
/// (bytes 18..20) as zero and padding an odd final byte with zero.
pub fn internet_checksum(frame: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < frame.len() {
        if i != CHECKSUM_OFFSET {
            let hi = u32::from(frame[i]);
            let lo = frame.get(i + 1).copied().map_or(0, u32::from);
            sum += (hi << 8) | lo;
        }
        i += 2;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn matches_recorded_frames() {
        // Frames copied from captures/read_2026-11-18.jsonl.
        for f in [
            "5e6e01fe20100000000000010000000000147f6e",
            "5e6e01fd10200000000000010000000000148f5f",
            "5e6e0100201000010000000101000000002275480a0b000300020000000100020002",
        ] {
            let b = hex(f);
            let stored = u16::from_be_bytes([b[18], b[19]]);
            assert_eq!(internet_checksum(&b), stored, "{f}");
        }
    }

    #[test]
    fn odd_length_is_padded() {
        let mut b = vec![0u8; 21];
        b[20] = 0x01;
        assert_eq!(internet_checksum(&b), !0x0100);
    }
}
