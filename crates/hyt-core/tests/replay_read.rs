//! Replays the recorded CPS read (captures/read_2026-11-18.jsonl) through the full read path.
//!
//! Passing proves that `hyt read` sends byte-for-byte what CPS sent, and that the files it saves
//! equal the files CPS received. Skipped when the private capture or the CPS install is missing.

use std::path::{Path, PathBuf};

use hyt_core::{read_radio, save_read};
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_proto::Session;
use hytera_proto::transport::ReplayTransport;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn replayed_cps_read_matches_recording() {
    let root = workspace_root();
    let capture = root.join("captures/read_2026-11-18.jsonl");
    let expected_dir = root.join("captures/read_2026-11-18_files");
    let rcdb_dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !capture.is_file() || !expected_dir.is_dir() || !rcdb_dir.is_dir() {
        eprintln!("skipping: capture or CPS RCDB data not present");
        return;
    }

    let mut session = Session::new(ReplayTransport::from_jsonl_file(&capture).unwrap());
    let result = read_radio(&mut session, &RcdbRoot::new(&rcdb_dir), |_, _, _, _| {}).unwrap();
    let replay = session.into_transport();
    assert!(replay.finished(), "not every recorded write was reproduced");
    assert_eq!(replay.writes_matched(), 324);

    assert_eq!(result.info.model_name().as_deref(), Some("BP515 U1"));
    assert_eq!(result.rcdb_version, "V1.5.24.000");
    assert_eq!(result.profile.key, "bp515");
    assert_eq!(result.files.len(), 58);

    let out = std::env::temp_dir().join(format!("hyt-replay-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let manifest = save_read(&result, &out, "replay").unwrap();
    for f in &manifest.files {
        let ours = std::fs::read(out.join(&f.file)).unwrap();
        let theirs = std::fs::read(expected_dir.join(&f.file)).unwrap();
        assert_eq!(ours, theirs, "file {} differs", f.file);
    }
    let unknown: Vec<u16> = manifest.files.iter().filter(|f| f.crc == "unknown").map(|f| f.id).collect();
    assert_eq!(unknown, vec![18, 46, 5, 242, 245, 253]);
    let trimmed: Vec<u16> =
        manifest.files.iter().filter(|f| f.crc == "body-crc32-before-stale-tail").map(|f| f.id).collect();
    assert_eq!(trimmed, vec![3, 6, 7, 9]);
    std::fs::remove_dir_all(&out).unwrap();
}
