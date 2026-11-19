//! Failures while talking to the radio, on a simulated radio holding the BP515's configuration
//! (captures/gui-write_2026-11-19/after): radio off, cable pulled during a read or a write, a file
//! refused, the restart after a write, a radio that never comes back; and saves that fail halfway.
//! Skipped when the private captures or the CPS install are missing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hyt_core::workspace::{FieldRef, Workspace};
use hyt_core::write_flow::{WriteEvent, WriteJob, WriteSettings, write_config};
use hyt_core::{SavedRead, load_read, read_radio, save_edited, sim_radio};
use hytera_codeplug::container::prepare_for_write;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_proto::Session;
use hytera_proto::sim::SimRadio;

const FOLDER: &str = "captures/gui-write_2026-11-19/after";

struct Fx {
    rcdb: RcdbRoot,
    saved: SavedRead,
    folder: PathBuf,
}

fn fixture() -> Option<Fx> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(FOLDER);
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !dir.is_dir() || !folder.join("manifest.json").is_file() {
        eprintln!("skipping: captures or CPS RCDB data not present");
        return None;
    }
    Some(Fx { rcdb: RcdbRoot::new(dir), saved: load_read(&folder).ok()?, folder })
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hyt-robust-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(dir.with_file_name(format!("hyt-robust-{name}-{}.partial", std::process::id())));
    dir
}

fn fast() -> WriteSettings {
    WriteSettings { allow_unverified: false, restart_pause: Duration::ZERO, restart_attempts: 5, programming_date: Some("2030-01-02".into()) }
}

/// Frames a full read takes, so faults can be placed after the backup.
fn frames_for_read(fx: &Fx) -> usize {
    let mut s = Session::new(sim_radio(&fx.saved));
    read_radio(&mut s, &fx.rcdb, |_, _, _, _| {}).unwrap();
    s.into_transport().frames_received
}

/// The current configuration with digital channel 2 renamed, saved to a temporary folder.
fn edited(fx: &Fx, name: &str) -> SavedRead {
    let mut ws = Workspace::open(&fx.folder, &fx.rcdb, None).unwrap();
    let file = ws.view().iter().find(|f| f.name == "QEU_C_DChannel").unwrap();
    let at = FieldRef { file: file.id, block: file.blocks[0].id, record: 1, member: None, key: "Channel Alias".into() };
    ws.set_value(&at, "Sim Test 2").unwrap();
    let out = temp(name);
    ws.save_as(&out).unwrap();
    load_read(&out).unwrap()
}

fn write(fx: &Fx, radio: SimRadio, saved: &SavedRead, backup: &Path, settings: &WriteSettings) -> (anyhow::Result<usize>, SimRadio, Vec<String>) {
    let mut events = Vec::new();
    let mut session = Session::new(radio);
    let job = WriteJob { saved, source: "test", backup, port: "sim" };
    let r = write_config(&mut session, &fx.rcdb, &job, settings, |e| match e {
        WriteEvent::NotReadyYet { attempt } => events.push(format!("not ready {attempt}")),
        WriteEvent::Verified { files } => events.push(format!("verified {files}")),
        _ => {}
    })
    .map(|r| r.written);
    (r, session.into_transport(), events)
}

#[test]
fn a_write_to_the_simulated_radio_is_verified_after_its_restart() {
    let Some(fx) = fixture() else { return };
    let saved = edited(&fx, "ok-edit");
    let mut radio = sim_radio(&fx.saved);
    radio.faults.restart_requests = 3;
    let backup = temp("ok-backup");
    let (r, radio, events) = write(&fx, radio, &saved, &backup, &fast());
    assert_eq!(r.unwrap(), 55);
    assert!(events.iter().any(|e| e.starts_with("not ready")), "the restart was waited for: {events:?}");
    assert_eq!(events.last().unwrap(), "verified 55");
    assert_eq!(radio.written.len(), 55);
    assert_eq!(Some(&radio.files[&3]), prepare_for_write(&saved.files[&3]).as_ref());
    // The programming date is stamped like CPS does; the saved folder itself is unchanged.
    let layout = fx.rcdb.layout("V1.5.24.000").unwrap();
    let basic = hytera_codeplug::decode::decode(&radio.files[&18], "Comm_Basic", &layout).unwrap();
    let stamped = basic.blocks.iter().flat_map(|b| &b.records).filter_map(|r| r.fields.as_ref()).find_map(|f| f.0.get("Programming Time").cloned());
    assert_eq!(stamped, Some(serde_json::json!("2030-01-02")));
    assert_eq!(load_read(&saved_dir("ok-edit")).unwrap().files[&18], saved.files[&18]);
    // The backup is the radio's configuration before the write.
    assert_eq!(load_read(&backup).unwrap().files, fx.saved.files);
    for d in [&backup, &saved_dir("ok-edit")] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

fn saved_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("hyt-robust-{name}-{}", std::process::id()))
}

#[test]
fn a_switched_off_radio_or_pulled_cable_during_a_read_gives_advice() {
    let Some(fx) = fixture() else { return };
    let mut off = sim_radio(&fx.saved);
    off.faults.dead_after_frames = Some(0);
    let Err(err) = read_radio(&mut Session::new(off), &fx.rcdb, |_, _, _, _| {}) else { panic!("read worked") };
    let text = format!("{err:#}");
    assert!(text.contains("identifying the radio") && text.contains("switched on"), "{text}");

    let mut pulled = sim_radio(&fx.saved);
    pulled.faults.dead_after_frames = Some(frames_for_read(&fx) / 2);
    let Err(err) = read_radio(&mut Session::new(pulled), &fx.rcdb, |_, _, _, _| {}) else { panic!("read worked") };
    let text = format!("{err:#}");
    assert!(text.contains("reading files (after ") && text.contains("of 58)") && text.contains("cable"), "{text}");
}

#[test]
fn a_cable_pulled_during_a_write_says_how_far_it_got() {
    let Some(fx) = fixture() else { return };
    let saved = edited(&fx, "pulled-edit");
    let mut radio = sim_radio(&fx.saved);
    radio.faults.dead_after_frames = Some(frames_for_read(&fx) + 120);
    let backup = temp("pulled-backup");
    let (r, radio, _) = write(&fx, radio, &saved, &backup, &fast());
    let text = format!("{:#}", r.unwrap_err());
    let n = radio.written.len();
    assert!(n > 0 && n < 55, "{n} files written");
    assert!(text.contains(&format!("writing stopped after {n} of 55 files")), "{text}");
    assert!(text.contains(&backup.display().to_string()), "names the backup: {text}");
    assert!(load_read(&backup).is_ok(), "the backup was saved before writing");
    for d in [&backup, &saved_dir("pulled-edit")] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn a_refused_first_file_and_a_radio_that_never_returns_are_reported() {
    let Some(fx) = fixture() else { return };
    let saved = edited(&fx, "refused-edit");
    let first = {
        let list = fx.rcdb.file_list("V1.5.24.000").unwrap();
        let links = fx.rcdb.table_links().unwrap();
        hyt_core::plan_write(&saved.files, &list, &links, &hytera_models::BP515).unwrap()[0].0.id
    };
    let mut radio = sim_radio(&fx.saved);
    radio.faults.refuse_write_of = Some(first);
    let backup = temp("refused-backup");
    let (r, radio, _) = write(&fx, radio, &saved, &backup, &fast());
    let text = format!("{:#}", r.unwrap_err());
    assert!(text.contains("before any file was complete") && text.contains("refused"), "{text}");
    assert!(radio.written.is_empty());
    std::fs::remove_dir_all(&backup).unwrap();

    let mut radio = sim_radio(&fx.saved);
    radio.faults.restart_requests = usize::MAX;
    let backup = temp("gone-backup");
    let settings = WriteSettings { restart_attempts: 2, ..fast() };
    let (r, radio, _) = write(&fx, radio, &saved, &backup, &settings);
    let text = format!("{:#}", r.unwrap_err());
    assert!(text.contains("could not be read back"), "{text}");
    assert_eq!(radio.written.len(), 55);
    for d in [&backup, &saved_dir("refused-edit")] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn saves_never_leave_half_written_folders() {
    let Some(fx) = fixture() else { return };
    // A file missing from the set fails the save midway: nothing is left behind.
    let mut files: BTreeMap<u16, Vec<u8>> = fx.saved.files.clone();
    files.remove(&250);
    let out = temp("half").join("nested");
    assert!(save_edited(&fx.saved, &files, &out, "test").is_err());
    assert!(!out.exists());
    assert!(!out.with_file_name("nested.partial").exists());
    // A complete save lands under its name, also into an existing empty folder.
    std::fs::create_dir_all(&out).unwrap();
    save_edited(&fx.saved, &fx.saved.files, &out, "test").unwrap();
    assert_eq!(load_read(&out).unwrap().files, fx.saved.files);
    assert!(!out.with_file_name("nested.partial").exists());
    assert!(save_edited(&fx.saved, &fx.saved.files, &out, "test").is_err(), "non-empty folders are refused");
    std::fs::remove_dir_all(out.parent().unwrap()).unwrap();
}
