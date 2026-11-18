//! The 20-byte header at the start of every radio file (little-endian), CRC rules, and preparing
//! files for writing. See `docs/file-format.md` section 3.

/// Length of the file header.
pub const HEADER_LEN: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    /// File ID (`Comm_RadioInfo` carries an extra high bit: `0x008000FD`).
    pub raw_file_id: u32,
    /// Version / record count; meaning not confirmed.
    pub version: u32,
    /// Logical size: equals the stored size for flat files, larger for list files.
    pub logical_size: u32,
    pub crc32: u32,
    /// Bytes 16..20. The radio returns non-zero values here for some files; CPS writes 0.
    pub reserved: u32,
}

impl FileHeader {
    pub fn parse(file: &[u8]) -> Option<Self> {
        let word = |i: usize| u32::from_le_bytes(file[i..i + 4].try_into().expect("4 bytes"));
        (file.len() >= HEADER_LEN).then(|| FileHeader {
            raw_file_id: word(0),
            version: word(4),
            logical_size: word(8),
            crc32: word(12),
            reserved: word(16),
        })
    }

    /// The file ID without flag bits.
    pub fn file_id(&self) -> u16 {
        (self.raw_file_id & 0xFFFF) as u16
    }
}

/// How the header CRC relates to the contents of a file as read from the radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrcCheck {
    /// `crc32(file[20..]) == header.crc32`.
    Body,
    /// The file is followed by stale bytes (seen on list files: one leftover record):
    /// `crc32(file[20..len]) == header.crc32` where `len` is the end of the real content
    /// ([`crate::structure::FileStructure::content_end`]). CPS drops the stale bytes on write.
    StaleTail { len: usize },
    /// No known rule matches (some multi-block files as read; see `docs/file-format.md`).
    Unknown,
}

/// Check the known CRC rules against `file`.
pub fn check_crc(file: &[u8]) -> CrcCheck {
    let Some(h) = FileHeader::parse(file) else { return CrcCheck::Unknown };
    if crc32fast::hash(&file[HEADER_LEN..]) == h.crc32 {
        return CrcCheck::Body;
    }
    if let Ok(s) = crate::structure::parse(file)
        && s.content_end < file.len()
        && crc32fast::hash(&file[HEADER_LEN..s.content_end]) == h.crc32
    {
        return CrcCheck::StaleTail { len: s.content_end };
    }
    CrcCheck::Unknown
}

/// Turn a file as read from the radio (or edited) into the form CPS writes back (observed for all
/// 55 files CPS wrote on 2026-11-18):
/// - files lose any stale trailing bytes after the end of their block area (see
///   [`crate::structure`]; for an unparseable file, the [`CrcCheck::StaleTail`] rule is used),
/// - header bytes 16..20 become 0,
/// - the header CRC becomes `crc32(file[20..])`.
///
/// Content is not changed. (CPS additionally stamps the programming date into `Comm_Basic`;
/// see `docs/file-format.md`.)
pub fn prepare_for_write(file: &[u8]) -> Option<Vec<u8>> {
    FileHeader::parse(file)?;
    // Edited files no longer match their CRC, so trim by structure, not by CRC.
    let len = match crate::structure::parse(file) {
        Ok(s) => s.content_end,
        Err(_) => match check_crc(file) {
            CrcCheck::StaleTail { len } => len,
            _ => file.len(),
        },
    };
    let mut out = file[..len].to_vec();
    out[16..20].fill(0);
    let crc = crc32fast::hash(&out[HEADER_LEN..]);
    out[12..16].copy_from_slice(&crc.to_le_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(words: [u32; 5], body: &[u8]) -> Vec<u8> {
        let mut f: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        f.extend_from_slice(body);
        f
    }

    #[test]
    fn parses_header_and_checks_crc() {
        let body = b"hello radio";
        let mut f = file([86, 1, 31, crc32fast::hash(body), 0], body);
        let h = FileHeader::parse(&f).unwrap();
        assert_eq!((h.file_id(), h.version, h.logical_size), (86, 1, 31));
        assert_eq!(check_crc(&f), CrcCheck::Body);
        f[25] ^= 1;
        assert_eq!(check_crc(&f), CrcCheck::Unknown);
    }

    #[test]
    fn file_with_stale_tail_is_trimmed_for_writing() {
        // Body: 8 zero bytes, u32 n, directory (block 9 at 0), block header with no records.
        let mut block = Vec::new();
        block.extend_from_slice(&9u16.to_le_bytes());
        block.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // capacity 0, unknown 0, count 0
        block.extend_from_slice(b"Name\0\0\0\0");
        block.extend_from_slice(&4u32.to_le_bytes());
        let mut body = vec![0u8; 8];
        body.extend_from_slice(&(block.len() as u32).to_le_bytes());
        body.extend_from_slice(&9u16.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&block);
        let content_crc = crc32fast::hash(&body);
        body.extend_from_slice(&[0x55; 6]);
        let f = file([9, 1, 64, content_crc, 0x1234], &body);
        assert_eq!(check_crc(&f), CrcCheck::StaleTail { len: 58 });

        let w = prepare_for_write(&f).unwrap();
        assert_eq!(w.len(), 58);
        assert_eq!(&w[16..20], &[0, 0, 0, 0]);
        assert_eq!(check_crc(&w), CrcCheck::Body);
        assert_eq!(prepare_for_write(&w).unwrap(), w, "idempotent");
    }
}
