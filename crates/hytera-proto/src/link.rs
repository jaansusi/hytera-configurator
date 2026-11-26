//! Link layer: 20-byte framed packets with checksum, sequence numbers and acknowledgements.
//!
//! Frame header (big-endian), see `docs/protocol.md` section 2:
//! `5E 6E 01 type src dst seq16 00 00 00 01 flag 00 00 00 len16 checksum16`, then the payload.

use std::time::{Duration, Instant};

use crate::checksum::{CHECKSUM_OFFSET, internet_checksum};
use crate::transport::Transport;
use crate::{Error, Result};

pub const SYNC: u8 = 0x5E;
pub const HEADER_LEN: usize = 20;
/// Largest frame accepted when resynchronising. Real frames are at most about 1 KiB plus headers.
pub const MAX_FRAME_LEN: usize = 8 * 1024;

/// Address of the PC (CPS / this program).
pub const ADDR_PC: u8 = 0x20;
/// Address of the radio.
pub const ADDR_RADIO: u8 = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Connect,
    ConnectAck,
    Disconnect,
    DisconnectAck,
    Data,
    Ack,
    Other(u8),
}

impl FrameType {
    pub fn code(self) -> u8 {
        match self {
            FrameType::Connect => 0xFE,
            FrameType::ConnectAck => 0xFD,
            FrameType::Disconnect => 0xFB,
            FrameType::DisconnectAck => 0xFA,
            FrameType::Data => 0x00,
            FrameType::Ack => 0x10,
            FrameType::Other(c) => c,
        }
    }

    pub fn from_code(c: u8) -> Self {
        match c {
            0xFE => FrameType::Connect,
            0xFD => FrameType::ConnectAck,
            0xFB => FrameType::Disconnect,
            0xFA => FrameType::DisconnectAck,
            0x00 => FrameType::Data,
            0x10 => FrameType::Ack,
            other => FrameType::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: FrameType,
    pub src: u8,
    pub dst: u8,
    pub seq: u16,
    /// Header byte 12: 1 on data frames and on PC acks, 0 on radio acks and connect/disconnect.
    pub flag: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    /// A frame sent from the PC to the radio, with the flag value CPS uses for that frame type.
    pub fn from_pc(kind: FrameType, seq: u16, payload: Vec<u8>) -> Self {
        let flag = u8::from(matches!(kind, FrameType::Data | FrameType::Ack));
        Frame { kind, src: ADDR_PC, dst: ADDR_RADIO, seq, flag, payload }
    }

    pub fn encode(&self) -> Vec<u8> {
        let len = HEADER_LEN + self.payload.len();
        let mut b = vec![0u8; len];
        b[0] = SYNC;
        b[1] = 0x6E;
        b[2] = 0x01;
        b[3] = self.kind.code();
        b[4] = self.src;
        b[5] = self.dst;
        b[6..8].copy_from_slice(&self.seq.to_be_bytes());
        b[11] = 0x01;
        b[12] = self.flag;
        b[16..18].copy_from_slice(&(len as u16).to_be_bytes());
        b[HEADER_LEN..].copy_from_slice(&self.payload);
        let sum = internet_checksum(&b);
        b[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 2].copy_from_slice(&sum.to_be_bytes());
        b
    }

    /// Decode one complete frame (exactly `len` bytes), verifying length and checksum.
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER_LEN || b[0] != SYNC {
            return Err(Error::Malformed(format!("not a frame: {} bytes", b.len())));
        }
        let len = usize::from(u16::from_be_bytes([b[16], b[17]]));
        if len != b.len() {
            return Err(Error::Malformed(format!("length field {len} but {} bytes", b.len())));
        }
        let stored = u16::from_be_bytes([b[18], b[19]]);
        let computed = internet_checksum(b);
        if stored != computed {
            return Err(Error::Malformed(format!(
                "checksum {stored:04x}, expected {computed:04x}"
            )));
        }
        Ok(Frame {
            kind: FrameType::from_code(b[3]),
            src: b[4],
            dst: b[5],
            seq: u16::from_be_bytes([b[6], b[7]]),
            flag: b[12],
            payload: b[HEADER_LEN..].to_vec(),
        })
    }
}

/// Splits a received byte stream into frames, skipping garbage and frames with bad checksums.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    pub dropped_bytes: usize,
}

impl FrameReader {
    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn next_frame(&mut self) -> Option<Frame> {
        loop {
            match self.buf.iter().position(|&b| b == SYNC) {
                Some(0) => {}
                Some(i) => self.drop_front(i),
                None => {
                    let n = self.buf.len();
                    self.drop_front(n);
                    return None;
                }
            }
            if self.buf.len() < HEADER_LEN {
                return None;
            }
            let len = usize::from(u16::from_be_bytes([self.buf[16], self.buf[17]]));
            if !(HEADER_LEN..=MAX_FRAME_LEN).contains(&len) {
                self.drop_front(1);
                continue;
            }
            if self.buf.len() < len {
                return None;
            }
            match Frame::decode(&self.buf[..len]) {
                Ok(frame) => {
                    self.buf.drain(..len);
                    return Some(frame);
                }
                Err(e) => {
                    tracing::warn!("dropping bad frame: {e}");
                    self.drop_front(1);
                }
            }
        }
    }

    fn drop_front(&mut self, n: usize) {
        if n > 0 {
            tracing::debug!("resync: dropping {n} bytes");
            self.dropped_bytes += n;
            self.buf.drain(..n);
        }
    }
}

/// Reliable request/response exchange over a [`Transport`].
///
/// Timing follows CPS: while writing, the radio sometimes ignores a request entirely (no ACK);
/// CPS then resends it about every 920 ms until it is acknowledged (seen up to 2 resends).
pub struct Link<T> {
    transport: T,
    reader: FrameReader,
    seq: u16,
    /// How long to wait for an acknowledgement (or connect/disconnect answer) before resending.
    pub ack_timeout: Duration,
    /// How long to wait for the response once the request has been acknowledged.
    pub response_timeout: Duration,
    /// How often to resend a request the radio has not acknowledged.
    pub retries: u32,
}

/// Frames whose length is a multiple of this are followed by one `00` byte (see [`Link::send_frame`]).
pub const USB_PACKET_SIZE: usize = 64;

impl<T: Transport> Link<T> {
    pub fn new(transport: T) -> Self {
        Link {
            transport,
            reader: FrameReader::default(),
            seq: 0,
            ack_timeout: Duration::from_millis(900),
            response_timeout: Duration::from_secs(5),
            retries: 5,
        }
    }

    /// Send one encoded frame. Like CPS, follow a frame whose length is an exact multiple of 64
    /// bytes with a single `00` byte (observed 3 of 3 times; probably so the USB serial adapter
    /// ends the transfer). The radio skips it while looking for the next sync byte.
    fn send_frame(&mut self, frame: &[u8]) -> Result<()> {
        tracing::trace!("tx {}", hex(frame));
        self.transport.write_all(frame)?;
        if frame.len().is_multiple_of(USB_PACKET_SIZE) {
            self.transport.write_all(&[0x00])?;
        }
        Ok(())
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    /// Open a link session. Data sequence numbers restart at 1 afterwards.
    pub fn connect(&mut self) -> Result<()> {
        self.control(FrameType::Connect, FrameType::ConnectAck, "connect acknowledgement")?;
        self.seq = 0;
        Ok(())
    }

    /// Close the link session.
    pub fn disconnect(&mut self) -> Result<()> {
        self.control(FrameType::Disconnect, FrameType::DisconnectAck, "disconnect acknowledgement")
    }

    /// Send one application payload and return the radio's response payload.
    ///
    /// Sequence: PC DATA(n) → radio ACK(n) → radio DATA → PC ACK. An unacknowledged request is
    /// resent up to `retries` times.
    pub fn exchange(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        let request = Frame::from_pc(FrameType::Data, seq, payload.to_vec()).encode();
        let mut attempts = 0;
        loop {
            self.send_frame(&request)?;
            let mut acked = false;
            loop {
                let wait = if acked { self.response_timeout } else { self.ack_timeout };
                match self.recv(wait)? {
                    Some(f) if f.kind == FrameType::Ack && f.seq == seq => acked = true,
                    Some(f) if f.kind == FrameType::Data => {
                        let ack = Frame::from_pc(FrameType::Ack, f.seq, Vec::new()).encode();
                        self.send_frame(&ack)?;
                        return Ok(f.payload);
                    }
                    Some(f) => tracing::debug!("ignoring {:?} seq {} while waiting", f.kind, f.seq),
                    None if acked => {
                        return Err(Error::Timeout {
                            waiting_for: "response after acknowledgement",
                            timeout: self.response_timeout,
                        });
                    }
                    None => break,
                }
            }
            attempts += 1;
            if attempts > self.retries {
                return Err(Error::Timeout { waiting_for: "acknowledgement", timeout: self.ack_timeout });
            }
            tracing::info!("no acknowledgement for seq {seq}, resending (attempt {attempts})");
        }
    }

    fn control(&mut self, send: FrameType, expect: FrameType, what: &'static str) -> Result<()> {
        let frame = Frame::from_pc(send, 0, Vec::new()).encode();
        self.send_frame(&frame)?;
        loop {
            match self.recv(self.ack_timeout)? {
                Some(f) if f.kind == expect => return Ok(()),
                Some(f) => tracing::debug!("ignoring {:?} while waiting for {what}", f.kind),
                None => return Err(Error::Timeout { waiting_for: what, timeout: self.ack_timeout }),
            }
        }
    }

    /// Next frame, or `None` if nothing complete arrived within `wait`.
    fn recv(&mut self, wait: Duration) -> Result<Option<Frame>> {
        let deadline = Instant::now() + wait;
        let mut buf = [0u8; 2048];
        loop {
            if let Some(frame) = self.reader.next_frame() {
                tracing::trace!("rx {:?} seq {} payload {}", frame.kind, frame.seq, hex(&frame.payload));
                return Ok(Some(frame));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let n = self.transport.read(&mut buf, deadline - now)?;
            if n == 0 {
                return Ok(None);
            }
            self.reader.push(&buf[..n]);
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn encodes_like_cps() {
        assert_eq!(
            hex(&Frame::from_pc(FrameType::Connect, 0, vec![]).encode()),
            "5e6e01fe20100000000000010000000000147f6e"
        );
        assert_eq!(
            hex(&Frame::from_pc(FrameType::Disconnect, 0, vec![]).encode()),
            "5e6e01fb20100000000000010000000000147f71"
        );
        assert_eq!(
            hex(&Frame::from_pc(FrameType::Ack, 1, vec![]).encode()),
            "5e6e011020100001000000010100000000147f5b"
        );
    }

    /// Records writes; never returns data.
    #[derive(Default)]
    struct Sink(Vec<Vec<u8>>);
    impl Transport for Sink {
        fn write_all(&mut self, data: &[u8]) -> Result<()> {
            self.0.push(data.to_vec());
            Ok(())
        }
        fn read(&mut self, _: &mut [u8], _: Duration) -> Result<usize> {
            Ok(0)
        }
    }

    #[test]
    fn pads_64_byte_multiples_and_resends_unacknowledged() {
        let mut link = Link::new(Sink::default());
        link.retries = 2;
        // 300-byte payload -> 320-byte frame, a multiple of 64.
        let err = link.exchange(&[0u8; 300]).unwrap_err();
        assert!(matches!(err, Error::Timeout { waiting_for: "acknowledgement", .. }));
        let writes = link.into_transport().0;
        assert_eq!(writes.len(), 6, "3 sends, each followed by a 00 byte");
        assert!(writes.chunks(2).all(|w| w[0].len() == 320 && w[1] == [0x00]));
        assert!(writes.chunks(2).all(|w| w[0] == writes[0][..]));
    }

    #[test]
    fn reader_resyncs_after_garbage_and_split_input() {
        let good = Frame::from_pc(FrameType::Data, 7, vec![1, 2, 3]).encode();
        let mut corrupt = good.clone();
        corrupt[21] ^= 0xFF;
        let mut r = FrameReader::default();
        r.push(&[0x00, 0x5E, 0x11]);
        r.push(&corrupt);
        r.push(&good[..10]);
        assert!(r.next_frame().is_none());
        r.push(&good[10..]);
        let f = r.next_frame().expect("frame");
        assert_eq!((f.seq, f.payload), (7, vec![1, 2, 3]));
        assert!(r.dropped_bytes > 0);
    }
}
