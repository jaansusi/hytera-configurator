//! Block structure inside a radio file (little-endian). See `docs/file-format.md` section 4.
//!
//! ```text
//! 0   20-byte file header (container.rs)
//! 20  8 zero bytes
//! 28  u32 n: total length of the block bodies
//! 32  directory: k × (u16 block id, u32 offset from the end of the directory)
//! 32+6k  block bodies (n bytes); anything after that is a stale tail
//! ```
//! Each block body:
//! ```text
//! +0  u16 block id, u16 capacity (bit 15 set for two-level lists), u16 unknown, u16 record count
//! +8  8-byte ASCII short name, +16 u32 record size
//! +20 index: capacity × (u16 slot, u32 slot × record size)
//! then `count` records of `record size` bytes (slots 0..count)
//! ```
//! A record of a two-level block holds its own member list:
//! `u16 unknown, u16 capacity, u16 member count, u16 member size, u16 unknown`, an index of
//! capacity × 6 bytes, then capacity member slots of `member size` bytes (the first `count` used).

use std::ops::Range;

use crate::{Error, Result};

const DIRECTORY_OFFSET: usize = 32;
const BODY_LENGTH_OFFSET: usize = 28;
const BLOCK_HEADER_LEN: usize = 20;
const INDEX_ENTRY_LEN: usize = 6;
const SUBLIST_HEADER_LEN: usize = 10;
/// Directories seen so far have at most 11 entries; anything far beyond is not a directory.
const MAX_DIRECTORY_ENTRIES: usize = 64;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// One block of a file, with byte ranges into the file.
#[derive(Debug, Clone)]
pub struct Block {
    pub id: u16,
    pub two_dim: bool,
    pub capacity: u16,
    /// Header field +4; equals the record count in every file seen so far except empty lists (0).
    pub unknown: u16,
    pub count: u16,
    pub short_name: String,
    pub record_size: usize,
    /// File range of the block header and slot index (everything before the records).
    pub head: Range<usize>,
    /// File range of each present record.
    pub records: Vec<Range<usize>>,
}

/// The parsed layout of one file.
#[derive(Debug, Clone)]
pub struct FileStructure {
    pub blocks: Vec<Block>,
    /// End of the real content (directory end + n). Bytes after it are a stale tail.
    pub content_end: usize,
}

/// Parse the directory and block headers of `file`.
pub fn parse(file: &[u8]) -> Result<FileStructure> {
    let bad = |msg: String| Error::Invalid(format!("file structure: {msg}"));
    let n = u32_at(file, BODY_LENGTH_OFFSET).ok_or_else(|| bad("file too short".into()))? as usize;

    // Directory entries continue until the first block body, which starts with the id of the
    // first directory entry.
    let first_id = u16_at(file, DIRECTORY_OFFSET).ok_or_else(|| bad("no directory".into()))?;
    let mut entries = Vec::new();
    let mut pos = DIRECTORY_OFFSET;
    loop {
        let id = u16_at(file, pos).ok_or_else(|| bad("directory runs past the end".into()))?;
        let off = u32_at(file, pos + 2).ok_or_else(|| bad("directory runs past the end".into()))?;
        entries.push((id, off as usize));
        pos += INDEX_ENTRY_LEN;
        if u16_at(file, pos) == Some(first_id) {
            break;
        }
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            return Err(bad("no end of directory found".into()));
        }
    }
    let base = pos;
    let content_end = base + n;
    if content_end > file.len() {
        return Err(bad(format!("content ends at {content_end}, file has {} bytes", file.len())));
    }

    let mut blocks = Vec::new();
    for (i, &(id, off)) in entries.iter().enumerate() {
        let h = base + off;
        let next = entries.get(i + 1).map_or(content_end, |&(_, o)| base + o);
        let field = |o: usize| u16_at(file, h + o).ok_or_else(|| bad(format!("block {id} header cut off")));
        if field(0)? != id {
            return Err(bad(format!("directory says block {id} at {h}, header says {}", field(0)?)));
        }
        let cap_raw = field(2)?;
        let capacity = cap_raw & 0x7FFF;
        let count = field(6)?;
        let record_size = u32_at(file, h + 16).ok_or_else(|| bad(format!("block {id} header cut off")))? as usize;
        let records_start = h + BLOCK_HEADER_LEN + usize::from(capacity) * INDEX_ENTRY_LEN;
        let records_end = records_start + usize::from(count) * record_size;
        if records_end != next {
            return Err(bad(format!("block {id}: records end at {records_end}, next block starts at {next}")));
        }
        let short_name = String::from_utf8_lossy(&file[h + 8..h + 16]).trim_end_matches('\0').to_string();
        blocks.push(Block {
            id,
            two_dim: cap_raw & 0x8000 != 0,
            capacity,
            unknown: field(4)?,
            count,
            short_name,
            record_size,
            head: h..records_start,
            records: (0..usize::from(count))
                .map(|r| records_start + r * record_size..records_start + (r + 1) * record_size)
                .collect(),
        });
    }
    Ok(FileStructure { blocks, content_end })
}

/// Rebuild `original` with new record contents per block (`records[block][record]`), as CPS does
/// when it adds a record (observed 2026-11-18): records stay in slot order; the block header's
/// count (+6) and the field at +4 become the new count; the slot index, file header (including
/// the logical size) and block order are kept; the body length and directory offsets are
/// recomputed; any stale tail is dropped. The CRC is not updated (see `prepare_for_write`).
pub fn rebuild(original: &[u8], s: &FileStructure, records: &[Vec<Vec<u8>>]) -> Result<Vec<u8>> {
    if records.len() != s.blocks.len() {
        return Err(Error::Invalid("rebuild: wrong number of blocks".into()));
    }
    let mut bodies = Vec::new();
    for (b, recs) in s.blocks.iter().zip(records) {
        if recs.len() > usize::from(b.capacity) {
            return Err(Error::Invalid(format!(
                "block {} ({}) holds at most {} records, got {}",
                b.id, b.short_name, b.capacity, recs.len()
            )));
        }
        let count = u16::try_from(recs.len()).expect("checked against capacity");
        let mut body = original[b.head.clone()].to_vec();
        body[4..6].copy_from_slice(&count.to_le_bytes());
        body[6..8].copy_from_slice(&count.to_le_bytes());
        for r in recs {
            if r.len() != b.record_size {
                return Err(Error::Invalid(format!("block {}: record of {} bytes, expected {}", b.id, r.len(), b.record_size)));
            }
            body.extend_from_slice(r);
        }
        bodies.push((b.id, body));
    }
    let n: usize = bodies.iter().map(|(_, body)| body.len()).sum();
    let mut out = original[..BODY_LENGTH_OFFSET].to_vec();
    out.extend_from_slice(&(n as u32).to_le_bytes());
    let mut offset = 0usize;
    for (id, body) in &bodies {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += body.len();
    }
    for (_, body) in bodies {
        out.extend_from_slice(&body);
    }
    Ok(out)
}

/// The member list inside one record of a two-level block.
#[derive(Debug, Clone)]
pub struct SubList {
    pub capacity: u16,
    pub count: u16,
    pub member_size: usize,
    /// Ranges of the used members, relative to the record.
    pub members: Vec<Range<usize>>,
}

/// Return `record` with its member list replaced by `members` (each `member_size` bytes): the
/// count is set, members fill slots 0.., and slots that were used before but not now are zeroed.
/// The member index and capacity are kept.
pub fn rebuild_sublist(record: &[u8], members: &[Vec<u8>]) -> Result<Vec<u8>> {
    let sub = parse_sublist(record)?;
    if members.len() > usize::from(sub.capacity) {
        return Err(Error::Invalid(format!("at most {} members, got {}", sub.capacity, members.len())));
    }
    let start = SUBLIST_HEADER_LEN + usize::from(sub.capacity) * INDEX_ENTRY_LEN;
    let mut out = record.to_vec();
    out[4..6].copy_from_slice(&(members.len() as u16).to_le_bytes());
    for (i, m) in members.iter().enumerate() {
        if m.len() != sub.member_size {
            return Err(Error::Invalid(format!("member of {} bytes, expected {}", m.len(), sub.member_size)));
        }
        out[start + i * sub.member_size..start + (i + 1) * sub.member_size].copy_from_slice(m);
    }
    for i in members.len()..usize::from(sub.count) {
        out[start + i * sub.member_size..start + (i + 1) * sub.member_size].fill(0);
    }
    Ok(out)
}

/// An empty member list shaped like `like` (same capacity, member size and index), count 0 and
/// all member slots zeroed.
pub fn empty_sublist_like(like: &[u8]) -> Result<Vec<u8>> {
    let sub = parse_sublist(like)?;
    let start = SUBLIST_HEADER_LEN + usize::from(sub.capacity) * INDEX_ENTRY_LEN;
    let mut out = like.to_vec();
    out[4..6].fill(0);
    out[start..].fill(0);
    Ok(out)
}

pub fn parse_sublist(record: &[u8]) -> Result<SubList> {
    let bad = |msg: &str| Error::Invalid(format!("member list: {msg}"));
    let capacity = u16_at(record, 2).ok_or_else(|| bad("too short"))?;
    let count = u16_at(record, 4).ok_or_else(|| bad("too short"))?;
    let member_size = usize::from(u16_at(record, 6).ok_or_else(|| bad("too short"))?);
    let start = SUBLIST_HEADER_LEN + usize::from(capacity) * INDEX_ENTRY_LEN;
    let end = start + usize::from(capacity) * member_size;
    if end != record.len() || count > capacity {
        return Err(bad(&format!(
            "capacity {capacity}, count {count}, member size {member_size} do not fit {} bytes",
            record.len()
        )));
    }
    Ok(SubList {
        capacity,
        count,
        member_size,
        members: (0..usize::from(count)).map(|m| start + m * member_size..start + (m + 1) * member_size).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with one block of two 4-byte records and room for three.
    fn sample() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&7u16.to_le_bytes());
        body.extend_from_slice(&3u16.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(b"Test\0\0\0\0");
        body.extend_from_slice(&4u32.to_le_bytes());
        for slot in 0u16..3 {
            body.extend_from_slice(&slot.to_le_bytes());
            body.extend_from_slice(&(u32::from(slot) * 4).to_le_bytes());
        }
        body.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let mut f = vec![0u8; 28];
        f.extend_from_slice(&(body.len() as u32).to_le_bytes());
        f.extend_from_slice(&7u16.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&body);
        f.extend_from_slice(&[0xEE; 4]); // stale tail
        f
    }

    #[test]
    fn parses_blocks_and_records() {
        let f = sample();
        let s = parse(&f).unwrap();
        assert_eq!(s.content_end, f.len() - 4);
        let b = &s.blocks[0];
        assert_eq!((b.id, b.capacity, b.count, b.record_size, b.short_name.as_str()), (7, 3, 2, 4, "Test"));
        assert_eq!(&f[b.records[1].clone()], &[5, 6, 7, 8]);
    }
}
