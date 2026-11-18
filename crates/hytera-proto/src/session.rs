//! Radio commands and the CPS-compatible read flows.

use std::collections::BTreeMap;

use crate::app::{Packet, RESPONSE_BIT, Reader, opcode, sub};
use crate::link::Link;
use crate::transport::Transport;
use crate::{Error, Result};

/// Chunk size CPS requests when opening a file.
pub const DEFAULT_CHUNK_SIZE: u16 = 0x0400;

/// Information items, in the order CPS queries them during identification. `None` marks the
/// unknown `0x001B` query CPS sends between items `0xFFF6` and `0xFFF4`.
pub const CPS_IDENTIFY_SEQUENCE: &[Option<u16>] = &[
    Some(0x0002),
    Some(0x0000),
    Some(0x0011),
    Some(0x0001),
    Some(0x0003),
    Some(0x000A),
    Some(0x000B),
    Some(0xFFF0),
    Some(0x0009),
    Some(0xFFF6),
    None,
    Some(0xFFF4),
    Some(0x0008),
    Some(0x0007),
    Some(0x0100),
    Some(0x0045),
    Some(0x0006),
    Some(0x0200),
    Some(0x0065),
    Some(0x0071),
];

/// Well-known information items (meanings partly inferred; see `docs/protocol.md`).
pub mod item {
    pub const MODEL_CODE: u16 = 0x0000;
    pub const FIRMWARE_VERSION: u16 = 0x0001;
    pub const RCDB_VERSION: u16 = 0x0002;
    pub const HARDWARE_VERSION: u16 = 0x0006;
    pub const SERIAL_NUMBER: u16 = 0x000B;
    pub const MODEL_NAME: u16 = 0x0011;
    pub const PRODUCT_TYPE: u16 = 0xFFF0;
}

/// One information item as returned by the radio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoItem {
    pub item: u16,
    /// 0 = OK; 1 = the radio does not support this item.
    pub status: u16,
    pub value: Vec<u8>,
}

impl InfoItem {
    /// The value as text (UTF-16BE, trailing NULs removed), if it decodes cleanly.
    pub fn text(&self) -> Option<String> {
        let (pairs, rest) = self.value.as_chunks::<2>();
        if !rest.is_empty() {
            return None;
        }
        let units: Vec<u16> = pairs.iter().map(|&p| u16::from_be_bytes(p)).collect();
        let s = String::from_utf16(&units).ok()?;
        let s = s.trim_end_matches('\0').to_string();
        s.chars().all(|c| !c.is_control() && !matches!(c, '\u{FFFE}' | '\u{FFFF}')).then_some(s)
    }
}

/// Everything learned during identification.
#[derive(Debug, Clone, Default)]
pub struct RadioInfo {
    pub items: BTreeMap<u16, InfoItem>,
    /// Raw answer to the unknown `0x001B` query.
    pub unknown_1b: Option<Vec<u8>>,
}

impl RadioInfo {
    pub fn text(&self, item: u16) -> Option<String> {
        self.items.get(&item).filter(|i| i.status == 0).and_then(InfoItem::text)
    }
    pub fn model_name(&self) -> Option<String> {
        self.text(item::MODEL_NAME)
    }
    pub fn rcdb_version(&self) -> Option<String> {
        self.text(item::RCDB_VERSION)
    }
    pub fn firmware_version(&self) -> Option<String> {
        self.text(item::FIRMWARE_VERSION)
    }
    pub fn serial_number(&self) -> Option<String> {
        self.text(item::SERIAL_NUMBER)
    }
}

/// Programming mode, sent when entering and leaving programming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Read = 0,
    Write = 1,
}

/// A conversation with one radio. Transaction IDs keep counting across link sessions, as in CPS.
pub struct Session<T> {
    link: Link<T>,
    next_txn: u32,
}

impl<T: Transport> Session<T> {
    pub fn new(transport: T) -> Self {
        Session { link: Link::new(transport), next_txn: 1 }
    }

    /// Set the next transaction ID. CPS counts from 1 when it starts and never resets, so
    /// replaying a later CPS session needs its starting value.
    pub fn set_next_txn(&mut self, txn: u32) {
        self.next_txn = txn;
    }

    pub fn link_mut(&mut self) -> &mut Link<T> {
        &mut self.link
    }

    pub fn into_transport(self) -> T {
        self.link.into_transport()
    }

    pub fn connect(&mut self) -> Result<()> {
        self.link.connect()
    }

    pub fn disconnect(&mut self) -> Result<()> {
        self.link.disconnect()
    }

    /// Send one application request and return the matching response.
    pub fn call(&mut self, op: u16, subcommand: u16, data: Vec<u8>) -> Result<Packet> {
        let txn = self.next_txn;
        self.next_txn = self.next_txn.wrapping_add(1);
        let request = Packet { opcode: op, sub: subcommand, txn, data };
        let response = Packet::decode(&self.link.exchange(&request.encode())?)?;
        if response.opcode != op | RESPONSE_BIT || response.sub != subcommand || response.txn != txn {
            return Err(Error::Unexpected(format!(
                "asked op {op:04x}/{subcommand:04x} txn {txn}, got op {:04x}/{:04x} txn {}",
                response.opcode, response.sub, response.txn
            )));
        }
        Ok(response)
    }

    pub fn get_info(&mut self, item: u16) -> Result<InfoItem> {
        let resp = self.call(opcode::INFO, sub::INFO_ITEM, item.to_be_bytes().to_vec())?;
        let mut r = Reader::new(&resp.data, "info response");
        let status = r.u16()?;
        let got_item = r.u16()?;
        let len = usize::from(r.u16()?);
        let value = r.bytes(len)?.to_vec();
        if got_item != item {
            return Err(Error::Unexpected(format!("asked info item {item:#06x}, got {got_item:#06x}")));
        }
        Ok(InfoItem { item, status, value })
    }

    /// Identification exactly as CPS does it: connect, the 20 queries, disconnect.
    pub fn identify(&mut self) -> Result<RadioInfo> {
        self.connect()?;
        let mut info = RadioInfo::default();
        for step in CPS_IDENTIFY_SEQUENCE {
            match step {
                Some(item) => {
                    let value = self.get_info(*item)?;
                    info.items.insert(*item, value);
                }
                None => {
                    let resp = self.call(opcode::INFO, sub::INFO_UNKNOWN_1B, vec![0, 0])?;
                    info.unknown_1b = Some(resp.data);
                }
            }
        }
        self.disconnect()?;
        Ok(info)
    }

    pub fn enter_programming(&mut self, mode: Mode) -> Result<()> {
        let resp = self.call(opcode::CONTROL, sub::ENTER_PROGRAMMING, vec![mode as u8])?;
        expect_ok_status(&resp.data, "enter programming mode")
    }

    pub fn leave_programming(&mut self, mode: Mode) -> Result<()> {
        let resp = self.call(opcode::CONTROL, sub::LEAVE_PROGRAMMING, vec![mode as u8, 0])?;
        expect_ok_status(&resp.data, "leave programming mode")
    }

    /// Open `file_id` for writing `size` bytes.
    pub fn open_write(&mut self, file_id: u16, size: u32, chunk_size: u16) -> Result<()> {
        let mut data = file_id.to_be_bytes().to_vec();
        data.extend_from_slice(&size.to_be_bytes());
        data.extend_from_slice(&chunk_size.to_be_bytes());
        let resp = self.call(opcode::CONTROL, sub::OPEN_WRITE, data)?;
        expect_ok_status(&resp.data, &format!("opening file {file_id} for writing"))
    }

    /// Write chunk `index` (1-based) of the file currently open for writing.
    pub fn write_chunk(&mut self, index: u16, bytes: &[u8]) -> Result<()> {
        let mut data = index.to_be_bytes().to_vec();
        data.extend_from_slice(bytes);
        let resp = self.call(opcode::TRANSFER, sub::WRITE_CHUNK, data)?;
        let got = Reader::new(&resp.data, "write-chunk response").u32()?;
        if got != u32::from(index) {
            return Err(Error::Unexpected(format!("wrote chunk {index}, radio confirmed {got}")));
        }
        Ok(())
    }

    /// Write a whole file in `chunk_size` pieces.
    pub fn write_file(&mut self, file_id: u16, data: &[u8], chunk_size: u16) -> Result<()> {
        let size = u32::try_from(data.len())
            .map_err(|_| Error::Unexpected(format!("file {file_id} is too large")))?;
        self.open_write(file_id, size, chunk_size)?;
        for (i, chunk) in data.chunks(usize::from(chunk_size)).enumerate() {
            let index = u16::try_from(i + 1)
                .map_err(|_| Error::Unexpected(format!("file {file_id} needs too many chunks")))?;
            self.write_chunk(index, chunk)?;
        }
        Ok(())
    }

    /// Full write as CPS does it: connect, enter programming (write), write each file in the
    /// given order, leave, disconnect. `on_file(file_id)` is called after each file.
    ///
    /// This changes the radio's configuration. Callers must prepare the files
    /// (`hytera_codeplug::container::prepare_for_write`) and check the radio's identity first.
    pub fn write_files(
        &mut self,
        files: &[(u16, &[u8])],
        chunk_size: u16,
        mut on_file: impl FnMut(u16),
    ) -> Result<()> {
        self.connect()?;
        self.enter_programming(Mode::Write)?;
        for &(id, data) in files {
            self.write_file(id, data, chunk_size)?;
            on_file(id);
        }
        self.leave_programming(Mode::Write)?;
        self.disconnect()
    }

    /// Open `file_id` for reading; returns the file size in bytes.
    pub fn open_read(&mut self, file_id: u16, chunk_size: u16) -> Result<u32> {
        let mut data = file_id.to_be_bytes().to_vec();
        data.extend_from_slice(&chunk_size.to_be_bytes());
        let resp = self.call(opcode::CONTROL, sub::OPEN_READ, data)?;
        let mut r = Reader::new(&resp.data, "open-read response");
        let status = r.u16()?;
        if status != 0 {
            return Err(Error::Status { what: format!("opening file {file_id}"), status });
        }
        r.u32()
    }

    /// Read chunk `index` (1-based) of the currently open file.
    pub fn read_chunk(&mut self, index: u16) -> Result<Vec<u8>> {
        let resp = self.call(opcode::TRANSFER, sub::READ_CHUNK, index.to_be_bytes().to_vec())?;
        let mut r = Reader::new(&resp.data, "read-chunk response");
        let got = r.u32()?;
        if got != u32::from(index) {
            return Err(Error::Unexpected(format!("asked chunk {index}, got {got}")));
        }
        Ok(r.rest().to_vec())
    }

    /// Read a whole file. `progress(done_bytes, total_bytes)` is called after each chunk.
    pub fn read_file(
        &mut self,
        file_id: u16,
        chunk_size: u16,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Vec<u8>> {
        let size = self.open_read(file_id, chunk_size)? as usize;
        let chunk = usize::from(chunk_size);
        let chunks = size.div_ceil(chunk);
        let mut out = Vec::with_capacity(size);
        for index in 1..=chunks {
            let index = u16::try_from(index)
                .map_err(|_| Error::Unexpected(format!("file {file_id} needs {chunks} chunks")))?;
            out.extend(self.read_chunk(index)?);
            progress(out.len(), size);
        }
        if out.len() != size {
            return Err(Error::Unexpected(format!(
                "file {file_id}: announced {size} bytes, received {}",
                out.len()
            )));
        }
        Ok(out)
    }

    /// Full read as CPS does it: connect, enter programming, read each file, leave, disconnect.
    /// `on_file(file_id, &data)` is called after each file.
    pub fn read_files(
        &mut self,
        file_ids: &[u16],
        chunk_size: u16,
        mut on_file: impl FnMut(u16, &[u8]),
    ) -> Result<BTreeMap<u16, Vec<u8>>> {
        self.connect()?;
        self.enter_programming(Mode::Read)?;
        let mut files = BTreeMap::new();
        for &id in file_ids {
            let data = self.read_file(id, chunk_size, |_, _| {})?;
            on_file(id, &data);
            files.insert(id, data);
        }
        self.leave_programming(Mode::Read)?;
        self.disconnect()?;
        Ok(files)
    }
}

fn expect_ok_status(data: &[u8], what: &str) -> Result<()> {
    let status = Reader::new(data, "status response").u16()?;
    if status == 0 { Ok(()) } else { Err(Error::Status { what: what.into(), status }) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_text_decodes_utf16be_and_trims() {
        let v: Vec<u8> = "BP515 U1\0\0".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let i = InfoItem { item: 0x11, status: 0, value: v };
        assert_eq!(i.text().as_deref(), Some("BP515 U1"));
        let raw = InfoItem { item: 0x71, status: 0, value: vec![0xFF, 0xFF] };
        assert_eq!(raw.text(), None);
    }
}
