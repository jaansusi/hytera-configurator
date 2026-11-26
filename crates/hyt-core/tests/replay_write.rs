//! Replays the recorded CPS writes of an unchanged configuration through the write path:
//! - BP515 R23N141382, RCDB V1.5.24.000 (captures/write-unchanged_2026-11-18.jsonl, 55 files);
//! - older BP515 R225105080, firmware V1.5.13, RCDB V1.5.08.000
//!   (captures/write-unchanged-old-bp515_2026-11-26.jsonl, 54 files; CPS read it just before, in
//!   captures/cps-read-old-bp515_2026-11-26.jsonl, byte-identical to `hyt read`).
//!
//! Passing proves that `hyt` sends byte-for-byte what CPS sent when given CPS's files, and that
//! preparing our own read reproduces CPS's files except for the known CPS edits.
//! Skipped when the private captures or the CPS install are missing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hyt_core::{load_config_default, plan_write, write_radio};
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_models::BP515;
use hytera_proto::Session;
use hytera_proto::transport::ReplayTransport;

struct Case {
    rcdb_version: &'static str,
    capture: &'static str,
    written: &'static str,
    our_read: &'static str,
    serial: &'static str,
    /// CPS's transaction counter at the start of the recording (it never resets).
    first_txn: u32,
    files: usize,
    writes: usize,
    /// Files CPS changes in an unchanged write: 18 = programming date; 80 = a few bytes from offset
    /// 260 (first RX group record), different each time (inferred: leftover memory).
    cps_edits: &'static [u16],
}

const CASES: [Case; 2] = [
    Case {
        rcdb_version: "V1.5.24.000",
        capture: "captures/write-unchanged_2026-11-18.jsonl",
        written: "captures/write-unchanged_2026-11-18_files",
        our_read: "captures/hyt-read_2026-11-18",
        serial: "R23N141382",
        first_txn: 0xA1,
        files: 55,
        writes: 312,
        cps_edits: &[18, 80],
    },
    Case {
        rcdb_version: "V1.5.08.000",
        capture: "captures/write-unchanged-old-bp515_2026-11-26.jsonl",
        written: "captures/write-unchanged-old-bp515_2026-11-26_files",
        our_read: "captures/old-bp515-read_2026-11-26",
        serial: "R225105080",
        first_txn: 0x1E3,
        files: 54,
        writes: 301,
        cps_edits: &[18, 80],
    },
];

struct Fixtures {
    capture: PathBuf,
    cps_written: BTreeMap<u16, Vec<u8>>,
    our_read: PathBuf,
    rcdb: RcdbRoot,
}

fn fixtures(case: &Case) -> Option<Fixtures> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().ok()?;
    let capture = root.join(case.capture);
    let written_dir = root.join(case.written);
    let our_read = root.join(case.our_read);
    let rcdb_dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !capture.is_file() || !written_dir.is_dir() || !our_read.is_dir() || !rcdb_dir.is_dir() {
        eprintln!("skipping {}: captures or CPS RCDB data not present", case.capture);
        return None;
    }
    let mut cps_written = BTreeMap::new();
    for entry in std::fs::read_dir(&written_dir).ok()? {
        let path = entry.ok()?.path();
        let id: u16 = path.file_name()?.to_str()?.get(..3)?.parse().ok()?;
        cps_written.insert(id, std::fs::read(&path).ok()?);
    }
    Some(Fixtures { capture, cps_written, our_read, rcdb: RcdbRoot::new(rcdb_dir) })
}

#[test]
fn replayed_cps_write_matches_recording() {
    for case in &CASES {
        let Some(fx) = fixtures(case) else { continue };
        let list = fx.rcdb.file_list(case.rcdb_version).unwrap();
        let links = fx.rcdb.table_links().unwrap();
        let plan = plan_write(&fx.cps_written, &list, &links, &BP515).unwrap();
        assert_eq!(plan.len(), case.files, "{}", case.capture);
        for (entry, bytes) in &plan {
            assert_eq!(bytes, &fx.cps_written[&entry.id], "{}: preparing CPS's file {} changed it", case.capture, entry.id);
        }

        let mut session = Session::new(ReplayTransport::from_jsonl_file(&fx.capture).unwrap());
        session.set_next_txn(case.first_txn);
        let info = session.identify().unwrap();
        assert_eq!(info.serial_number().as_deref(), Some(case.serial));
        assert_eq!(info.rcdb_version().as_deref(), Some(case.rcdb_version));
        write_radio(&mut session, &plan, BP515.chunk_size, |_| {}).unwrap();
        let replay = session.into_transport();
        assert!(replay.finished(), "{}: not every recorded write was reproduced", case.capture);
        assert_eq!(replay.writes_matched(), case.writes, "{}", case.capture);
    }
}

#[test]
fn preparing_our_read_reproduces_cps_files() {
    for case in &CASES {
        let Some(fx) = fixtures(case) else { continue };
        let saved = load_config_default(&fx.our_read).unwrap();
        let list = fx.rcdb.file_list(case.rcdb_version).unwrap();
        let links = fx.rcdb.table_links().unwrap();
        let plan = plan_write(&saved.files, &list, &links, &BP515).unwrap();
        let ids: BTreeSet<u16> = plan.iter().map(|(e, _)| e.id).collect();
        assert_eq!(ids, fx.cps_written.keys().copied().collect(), "{}: other files than CPS's", case.capture);
        let differing: BTreeSet<u16> =
            plan.iter().filter(|(e, b)| b != &fx.cps_written[&e.id]).map(|(e, _)| e.id).collect();
        assert_eq!(differing, case.cps_edits.iter().copied().collect(), "{}", case.capture);
    }
}

#[test]
fn stamping_the_programming_date_reproduces_cps_file_18() {
    for case in &CASES {
        let Some(fx) = fixtures(case) else { continue };
        let saved = load_config_default(&fx.our_read).unwrap();
        let layout = fx.rcdb.layout(case.rcdb_version).unwrap();
        let cps18 = hytera_codeplug::decode::decode(&fx.cps_written[&18], "Comm_Basic", &layout).unwrap();
        let date = cps18
            .blocks
            .iter()
            .flat_map(|b| &b.records)
            .filter_map(|r| r.fields.as_ref())
            .find_map(|f| f.0.get("Programming Time").and_then(|v| v.as_str()).map(str::to_string))
            .unwrap();
        let mut files = saved.files.clone();
        assert!(hyt_core::stamp_programming_date(&saved, &mut files, &layout, &date).unwrap());
        let list = fx.rcdb.file_list(case.rcdb_version).unwrap();
        let links = fx.rcdb.table_links().unwrap();
        let plan = plan_write(&files, &list, &links, &BP515).unwrap();
        let file18 = &plan.iter().find(|(e, _)| e.id == 18).unwrap().1;
        assert!(file18 == &fx.cps_written[&18], "{}: stamped file 18 differs from CPS's (date {date})", case.capture);
    }
}
