//! A simulated radio for tests and offline trials: it answers the programming protocol the way the
//! BP515 does in the recordings (`docs/protocol.md`), serving files from memory and storing written
//! files. Faults can be injected: a pulled cable, a refused file, and the restart after a write
//! (the radio acknowledges requests but does not answer them for a while; observed 2026-11-18).
//!
//! It is not a full radio: it does not check file contents, and a written file reads back exactly
//! as written (the real radio may append a stale tail, which `prepare_for_write` removes).

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app::{Packet, RESPONSE_BIT, opcode, sub};
use crate::link::{ADDR_PC, ADDR_RADIO, Frame, FrameReader, FrameType};
use crate::transport::Transport;
use crate::{Error, Result};

/// One information item: (status, value).
pub type InfoValue = (u16, Vec<u8>);

enum Open {
    Read { data: Vec<u8>, chunk: usize },
    Write { id: u16, size: usize, data: Vec<u8> },
}

/// Faults to inject.
#[derive(Debug, Clone, Default)]
pub struct Faults {
    /// Stop answering for good after this many frames from the PC (cable pulled, radio off).
    pub dead_after_frames: Option<usize>,
    /// Refuse to open this file for writing (status 0x0001).
    pub refuse_write_of: Option<u16>,
    /// After a write session ends, the radio restarts: for this many frames from the PC it
    /// acknowledges data requests without answering them and ignores connect requests.
    pub restart_requests: usize,
}

/// The simulated radio's state.
pub struct SimRadio {
    info: BTreeMap<u16, InfoValue>,
    /// Current file contents by ID.
    pub files: BTreeMap<u16, Vec<u8>>,
    pub faults: Faults,
    /// Frames received from the PC so far.
    pub frames_received: usize,
    /// Files completely written, in order.
    pub written: Vec<u16>,
    from_pc: FrameReader,
    to_pc: VecDeque<u8>,
    seq: u16,
    open: Option<Open>,
    writing: bool,
    /// A write session ended; the restart begins after the disconnect.
    restart_pending: bool,
    restarting: usize,
}

impl SimRadio {
    pub fn new(info: BTreeMap<u16, InfoValue>, files: BTreeMap<u16, Vec<u8>>) -> Self {
        SimRadio {
            info,
            files,
            faults: Faults::default(),
            frames_received: 0,
            written: Vec::new(),
            from_pc: FrameReader::default(),
            to_pc: VecDeque::new(),
            seq: 0,
            open: None,
            writing: false,
            restart_pending: false,
            restarting: 0,
        }
    }

    fn send(&mut self, kind: FrameType, seq: u16, flag: u8, payload: Vec<u8>) {
        let frame = Frame { kind, src: ADDR_RADIO, dst: ADDR_PC, seq, flag, payload };
        self.to_pc.extend(frame.encode());
    }

    fn dead(&self) -> bool {
        self.faults.dead_after_frames.is_some_and(|n| self.frames_received > n)
    }

    fn handle(&mut self, frame: Frame) {
        self.frames_received += 1;
        if self.dead() {
            return;
        }
        if self.restarting > 0 {
            self.restarting -= 1;
            if frame.kind == FrameType::Data {
                self.send(FrameType::Ack, frame.seq, 0, Vec::new());
            }
            return;
        }
        match frame.kind {
            FrameType::Connect => self.send(FrameType::ConnectAck, 0, 0, Vec::new()),
            FrameType::Disconnect => {
                self.send(FrameType::DisconnectAck, 0, 0, Vec::new());
                if std::mem::take(&mut self.restart_pending) {
                    self.restarting = self.faults.restart_requests;
                }
            }
            FrameType::Data => {
                self.send(FrameType::Ack, frame.seq, 0, Vec::new());
                let Ok(request) = Packet::decode(&frame.payload) else { return };
                let data = self.answer(&request);
                let response = Packet { opcode: request.opcode | RESPONSE_BIT, sub: request.sub, txn: request.txn, data };
                self.seq = self.seq.wrapping_add(1);
                let seq = self.seq;
                self.send(FrameType::Data, seq, 1, response.encode());
            }
            _ => {}
        }
    }

    fn answer(&mut self, p: &Packet) -> Vec<u8> {
        let ok = vec![0, 0];
        let arg16 = |i: usize| p.data.get(i..i + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]));
        match (p.opcode, p.sub) {
            (opcode::INFO, sub::INFO_ITEM) => {
                let item = arg16(0);
                let (status, value) = self.info.get(&item).cloned().unwrap_or((1, Vec::new()));
                let mut d = status.to_be_bytes().to_vec();
                d.extend(item.to_be_bytes());
                d.extend((value.len() as u16).to_be_bytes());
                d.extend(value);
                d
            }
            (opcode::INFO, sub::INFO_UNKNOWN_1B) => vec![0, 0, 0, 1],
            (opcode::CONTROL, sub::ENTER_PROGRAMMING) => {
                self.writing = p.data.first() == Some(&1);
                ok
            }
            (opcode::CONTROL, sub::LEAVE_PROGRAMMING) => {
                if self.writing {
                    self.writing = false;
                    self.restart_pending = true;
                }
                ok
            }
            (opcode::CONTROL, sub::OPEN_READ) => {
                let id = arg16(0);
                let chunk = usize::from(arg16(2)).max(1);
                match self.files.get(&id) {
                    Some(data) => {
                        let mut d = ok.clone();
                        d.extend((data.len() as u32).to_be_bytes());
                        self.open = Some(Open::Read { data: data.clone(), chunk });
                        d
                    }
                    None => vec![0, 1],
                }
            }
            (opcode::TRANSFER, sub::READ_CHUNK) => {
                let index = usize::from(arg16(0));
                let mut d = (index as u32).to_be_bytes().to_vec();
                if let Some(Open::Read { data, chunk }) = &self.open {
                    let start = (index.saturating_sub(1) * chunk).min(data.len());
                    d.extend(&data[start..(start + chunk).min(data.len())]);
                }
                d
            }
            (opcode::CONTROL, sub::OPEN_WRITE) => {
                let id = arg16(0);
                if self.faults.refuse_write_of == Some(id) || !self.writing {
                    return vec![0, 1];
                }
                let size = p.data.get(2..6).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
                self.open = Some(Open::Write { id, size, data: Vec::new() });
                ok
            }
            (opcode::TRANSFER, sub::WRITE_CHUNK) => {
                let index = u32::from(arg16(0));
                if let Some(Open::Write { id, size, data }) = &mut self.open {
                    data.extend(p.data.get(2..).unwrap_or_default());
                    if data.len() >= *size {
                        let (id, data) = (*id, std::mem::take(data));
                        self.files.insert(id, data);
                        self.written.push(id);
                        self.open = None;
                    }
                }
                index.to_be_bytes().to_vec()
            }
            _ => vec![0, 1],
        }
    }
}

impl Transport for SimRadio {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.from_pc.push(data);
        while let Some(frame) = self.from_pc.next_frame() {
            self.handle(frame);
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8], _timeout: Duration) -> Result<usize> {
        let n = buf.len().min(self.to_pc.len());
        for (dst, src) in buf.iter_mut().zip(self.to_pc.drain(..n)) {
            *dst = src;
        }
        Ok(n)
    }
}

/// A [`SimRadio`] shared between sessions (each session owns its transport), e.g. a read after a
/// write sees the written files.
#[derive(Clone)]
pub struct SharedSim(pub Arc<Mutex<SimRadio>>);

impl SharedSim {
    pub fn new(radio: SimRadio) -> Self {
        SharedSim(Arc::new(Mutex::new(radio)))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SimRadio>> {
        self.0.lock().map_err(|_| Error::Unexpected("simulated radio state poisoned".into()))
    }
}

impl Transport for SharedSim {
    fn write_all(&mut self, data: &[u8]) -> Result<()> {
        self.lock()?.write_all(data)
    }

    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        self.lock()?.read(buf, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    fn radio() -> SimRadio {
        let name: Vec<u8> = "SIM 1".encode_utf16().flat_map(u16::to_be_bytes).collect();
        let info = BTreeMap::from([(0x0011, (0, name))]);
        let files = BTreeMap::from([(3, (0..2500u32).map(|i| i as u8).collect::<Vec<u8>>())]);
        SimRadio::new(info, files)
    }

    #[test]
    fn serves_info_and_files_and_stores_writes() {
        let mut s = Session::new(radio());
        let info = s.identify().unwrap();
        assert_eq!(info.model_name().as_deref(), Some("SIM 1"));
        let files = s.read_files(&[3], 0x400, |_, _| {}).unwrap();
        assert_eq!(files[&3].len(), 2500);
        assert_eq!(files[&3][2499], (2499u32 as u8));
        s.write_files(&[(3, &[7u8; 1500][..])], 0x400, |_| {}).unwrap();
        let back = s.read_files(&[3], 0x400, |_, _| {}).unwrap();
        assert_eq!(back[&3], vec![7u8; 1500]);
        assert_eq!(s.into_transport().written, vec![3]);
    }

    #[test]
    fn injected_faults_look_like_the_real_failures() {
        let mut r = radio();
        r.faults.dead_after_frames = Some(0);
        let err = Session::new(r).identify().unwrap_err();
        assert!(matches!(err, Error::Timeout { .. }), "{err}");

        let mut r = radio();
        r.faults.refuse_write_of = Some(3);
        let err = Session::new(r).write_files(&[(3, &[1u8; 10][..])], 0x400, |_| {}).unwrap_err();
        assert!(matches!(err, Error::Status { .. }), "{err}");

        let mut r = radio();
        r.faults.restart_requests = 2;
        let mut s = Session::new(r);
        s.write_files(&[(3, &[1u8; 10][..])], 0x400, |_| {}).unwrap();
        assert!(s.identify().is_err(), "no answer while restarting");
        assert!(s.identify().is_err(), "still restarting");
        assert!(s.identify().is_ok(), "answers again afterwards");
    }
}
