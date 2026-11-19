//! Replays the recorded CPS write of an unchanged configuration
//! (captures/write-unchanged_2026-11-18.jsonl) through the write path.
//!
//! Passing proves that `hyt` sends byte-for-byte what CPS sent when given CPS's files, and that
//! preparing our own read reproduces CPS's files except for the two known CPS edits.
//! Skipped when the private captures or the CPS install are missing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hyt_core::{load_read, plan_write, write_radio};
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_models::BP515;
use hytera_proto::Session;
use hytera_proto::transport::ReplayTransport;

const RCDB_VERSION: &str = "V1.5.24.000";

struct Fixtures {
    capture: PathBuf,
    cps_written: BTreeMap<u16, Vec<u8>>,
    our_read: PathBuf,
    rcdb: RcdbRoot,
}

fn fixtures() -> Option<Fixtures> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().ok()?;
    let capture = root.join("captures/write-unchanged_2026-11-18.jsonl");
    let written_dir = root.join("captures/write-unchanged_2026-11-18_files");
    let our_read = root.join("captures/hyt-read_2026-11-18");
    let rcdb_dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !capture.is_file() || !written_dir.is_dir() || !our_read.is_dir() || !rcdb_dir.is_dir() {
        eprintln!("skipping: captures or CPS RCDB data not present");
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
    let Some(fx) = fixtures() else { return };
    let list = fx.rcdb.file_list(RCDB_VERSION).unwrap();
    let links = fx.rcdb.table_links().unwrap();
    let plan = plan_write(&fx.cps_written, &list, &links, &BP515).unwrap();
    assert_eq!(plan.len(), 55);
    for (entry, bytes) in &plan {
        assert_eq!(bytes, &fx.cps_written[&entry.id], "preparing CPS's file {} changed it", entry.id);
    }

    let mut session = Session::new(ReplayTransport::from_jsonl_file(&fx.capture).unwrap());
    session.set_next_txn(0xA1); // CPS's counter continued from earlier sessions
    let info = session.identify().unwrap();
    assert_eq!(info.serial_number().as_deref(), Some("R23N141382"));
    write_radio(&mut session, &plan, BP515.chunk_size, |_| {}).unwrap();
    let replay = session.into_transport();
    assert!(replay.finished(), "not every recorded write was reproduced");
    assert_eq!(replay.writes_matched(), 312);
}

#[test]
fn preparing_our_read_reproduces_cps_files() {
    let Some(fx) = fixtures() else { return };
    let saved = load_read(&fx.our_read).unwrap();
    let list = fx.rcdb.file_list(RCDB_VERSION).unwrap();
    let links = fx.rcdb.table_links().unwrap();
    let plan = plan_write(&saved.files, &list, &links, &BP515).unwrap();
    let differing: BTreeSet<u16> =
        plan.iter().filter(|(e, b)| b != &fx.cps_written[&e.id]).map(|(e, _)| e.id).collect();
    // 18: CPS stamps the programming date; 80: CPS rewrites 5 bytes of the first RX group record.
    assert_eq!(differing, BTreeSet::from([18, 80]));
}

#[test]
fn stamping_the_programming_date_reproduces_cps_file_18() {
    let Some(fx) = fixtures() else { return };
    let saved = load_read(&fx.our_read).unwrap();
    let layout = fx.rcdb.layout(RCDB_VERSION).unwrap();
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
    let list = fx.rcdb.file_list(RCDB_VERSION).unwrap();
    let links = fx.rcdb.table_links().unwrap();
    let plan = plan_write(&files, &list, &links, &BP515).unwrap();
    let file18 = &plan.iter().find(|(e, _)| e.id == 18).unwrap().1;
    assert!(file18 == &fx.cps_written[&18], "stamped file 18 differs from CPS's (date {date})");
}
