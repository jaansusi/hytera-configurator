//! An older BP515 U1 (serial R225105080, firmware V1.5.13.000.01, RCDB V1.5.08.000; read
//! 2026-11-26 into captures/old-bp515-read_2026-11-26): 57 files, its own layout, labels from the
//! newest exported CPS data. Writing to it was verified on 2026-11-26.
//! Skipped when the private captures, the CPS install or the option catalogue are missing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hyt_core::tables::table;
use hyt_core::workspace::{ChannelKind, Workspace};
use hyt_core::write_flow::{WriteJob, WriteSettings, write_config};
use hyt_core::{load_config_default, sim_radio};
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_proto::Session;

const OLD: &str = "captures/old-bp515-read_2026-11-26";

fn setup() -> Option<(RcdbRoot, PathBuf)> {
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(OLD);
    if !dir.is_dir() || !folder.join("manifest.json").is_file() || !Options::default_path("V1.5.24.000")?.is_file() {
        eprintln!("skipping: captures, CPS RCDB data or option catalogue not present");
        return None;
    }
    Some((RcdbRoot::new(dir), folder))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hyt-test-old-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn older_radio_opens_with_labels_and_its_own_layout() {
    let Some((rcdb, folder)) = setup() else { return };
    let mut ws = Workspace::open(&folder, &rcdb, None).unwrap();
    assert_eq!(ws.manifest().rcdb_version, "V1.5.08.000");
    assert_eq!(ws.files().len(), 57);
    assert!(ws.options.is_some(), "labels: {:?}", ws.options_note);
    let note = ws.model_note.clone().unwrap_or_default();
    assert!(note.contains("RCDB V1.5.08.000") && note.contains("V1.5.24.000"), "{note}");

    let t = table(&ws, "digital").unwrap();
    assert_eq!(t.rows.len(), 1);
    let alias = t.columns.iter().position(|c| c.key == "Channel Alias").unwrap();
    assert_eq!(t.rows[0].cells[alias].text, "CH D1");
    let power = t.columns.iter().position(|c| c.key == "Power Level").unwrap();
    assert_eq!(t.rows[0].cells[power].text, "High");

    // Adding a channel copies this radio's own template record (its V1.5.08 layout).
    assert_eq!(ws.add_channel(ChannelKind::Digital).unwrap(), 1);
    ws.add_zone_member(0, ChannelKind::Digital, 2).unwrap();
    assert_eq!(table(&ws, "digital").unwrap().rows.len(), 2);
    let out = temp("saved");
    ws.save_as(&out).unwrap();
    let saved = load_config_default(&out).unwrap();
    assert_eq!(saved.manifest.rcdb_version, "V1.5.08.000");
    let layout = rcdb.layout("V1.5.08.000").unwrap();
    let decoded = hyt_core::decode_read(&saved, &layout).unwrap();
    let ch = decoded.iter().find(|f| f.name == "QEU_C_DChannel").unwrap();
    assert_eq!(ch.blocks[0].records.len(), 2);
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn writing_to_the_older_radio_needs_no_override() {
    // Verified 2026-11-26 (CPS recording replayed, then a live write); the gate itself is tested in
    // hytera-models (`writing_is_verified_per_rcdb_version`).
    let Some((rcdb, folder)) = setup() else { return };
    let saved = load_config_default(&folder).unwrap();
    let backup = temp("backup");
    let job = WriteJob { saved: &saved, source: OLD, backup: &backup, port: "sim" };
    let settings = WriteSettings {
        allow_unverified: false,
        restart_pause: Duration::ZERO,
        restart_attempts: 3,
        programming_date: Some("2030-01-02".into()),
    };
    let mut session = Session::new(sim_radio(&saved));
    let report = write_config(&mut session, &rcdb, &job, &settings, |_| {}).unwrap();
    // 54 files: CPS never writes 242, 245 and 253. Only file 18 (programming date) changes.
    assert_eq!((report.written, report.differing), (54, 1));
    assert_eq!(session.into_transport().written.len(), 54);
    std::fs::remove_dir_all(&backup).unwrap();
}
