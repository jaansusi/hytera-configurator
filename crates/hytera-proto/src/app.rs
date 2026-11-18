//! Application layer: `0x0A0B` packets carried in link DATA frames (big-endian).
//!
//! `0A 0B opcode16 sub16 txn32 len16 data[len]`. A response repeats the sub-command and transaction
//! ID and sets [`RESPONSE_BIT`] in the opcode.

use crate::{Error, Result};

pub const MAGIC: u16 = 0x0A0B;
pub const RESPONSE_BIT: u16 = 0x1000;
pub const HEADER_LEN: usize = 12;

/// Opcodes observed from CPS.
pub mod opcode {
    /// Radio information queries.
    pub const INFO: u16 = 0x0003;
    /// Session control and opening files.
    pub const CONTROL: u16 = 0x01F0;
    /// File data transfer.
    pub const TRANSFER: u16 = 0x41F8;
}

/// Sub-commands observed from CPS.
pub mod sub {
    /// `INFO`: read one information item (`u16 item`).
    pub const INFO_ITEM: u16 = 0x0002;
    /// `INFO`: unknown query CPS sends once during identification (data `00 00`, answer `00 00 00 01`).
    pub const INFO_UNKNOWN_1B: u16 = 0x001B;
    /// `CONTROL`: enter programming mode (`u8 mode`: 0 = read, 1 = write).
    pub const ENTER_PROGRAMMING: u16 = 0x0000;
    /// `CONTROL`: leave programming mode (`u8 mode, 00`).
    pub const LEAVE_PROGRAMMING: u16 = 0x0001;
    /// `CONTROL`: open a file for reading (`u16 file id, u16 chunk size`).
    pub const OPEN_READ: u16 = 0x0006;
    /// `CONTROL`: open a file for writing (`u16 file id, u32 size, u16 chunk size`).
    pub const OPEN_WRITE: u16 = 0x000B;
    /// `TRANSFER`: write one chunk (`u16 chunk index` (1-based), chunk bytes).
    pub const WRITE_CHUNK: u16 = 0x0005;
    /// `TRANSFER`: read one chunk (`u16 chunk index`, 1-based).
    pub const READ_CHUNK: u16 = 0x0008;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub opcode: u16,
    pub sub: u16,
    pub txn: u32,
    pub data: Vec<u8>,
}

impl Packet {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(HEADER_LEN + self.data.len());
        b.extend_from_slice(&MAGIC.to_be_bytes());
        b.extend_from_slice(&self.opcode.to_be_bytes());
        b.extend_from_slice(&self.sub.to_be_bytes());
        b.extend_from_slice(&self.txn.to_be_bytes());
        b.extend_from_slice(&(self.data.len() as u16).to_be_bytes());
        b.extend_from_slice(&self.data);
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER_LEN {
            return Err(Error::Malformed(format!("application packet of {} bytes", b.len())));
        }
        let magic = u16::from_be_bytes([b[0], b[1]]);
        if magic != MAGIC {
            return Err(Error::Malformed(format!("application magic {magic:04x}")));
        }
        let len = usize::from(u16::from_be_bytes([b[10], b[11]]));
        if b.len() != HEADER_LEN + len {
            return Err(Error::Malformed(format!(
                "application length field {len} but {} data bytes",
                b.len() - HEADER_LEN
            )));
        }
        Ok(Packet {
            opcode: u16::from_be_bytes([b[2], b[3]]),
            sub: u16::from_be_bytes([b[4], b[5]]),
            txn: u32::from_be_bytes([b[6], b[7], b[8], b[9]]),
            data: b[HEADER_LEN..].to_vec(),
        })
    }
}

/// Reads big-endian fields from response data with clear errors.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], what: &'static str) -> Self {
        Reader { data, pos: 0, what }
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos + n;
        let s = self.data.get(self.pos..end).ok_or_else(|| {
            Error::Malformed(format!("{}: need {end} bytes, have {}", self.what, self.data.len()))
        })?;
        self.pos = end;
        Ok(s)
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.data[self.pos..];
        self.pos = self.data.len();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_recorded_request() {
        let p = Packet { opcode: opcode::CONTROL, sub: sub::OPEN_READ, txn: 0x16, data: vec![0, 0x12, 4, 0] };
        let enc = p.encode();
        let hex: String = enc.iter().map(|x| format!("{x:02x}")).collect();
        assert_eq!(hex, "0a0b01f0000600000016000400120400");
        assert_eq!(Packet::decode(&enc).unwrap(), p);
    }
}
