//! Removing a channel, checked against CPS's recording of deleting digital channel 3 (`CH D1`,
//! a Zone 1 member) while `HYT Test 4` was channel 4 (captures/delete-channel-*_2026-11-18).
//! Skipped when the private captures, the CPS install or the option catalogue are missing.

use std::path::{Path, PathBuf};

use hyt_core::edit::{REMOVED, apply_edits};
use hyt_core::{SavedRead, decode_read, load_read};
use hytera_codeplug::decode::{DecodedFile, Fields, decode};
use hytera_codeplug::layout::Layout;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot, TableLink};
use serde_json::json;

struct Fx {
    before: SavedRead,
    cps_written: PathBuf,
    layout: Layout,
    options: Options,
    links: Vec<TableLink>,
}

fn fixture() -> Option<Fx> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rcdb_dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let before = root.join("captures/delete-channel-read_2026-11-18_files");
    let cps_written = root.join("captures/delete-channel-write_2026-11-18_files");
    let options_path = Options::default_path("V1.5.24.000")?;
    if !rcdb_dir.is_dir() || !before.join("manifest.json").is_file() || !cps_written.is_dir() || !options_path.is_file() {
        eprintln!("skipping: captures, CPS RCDB data or option catalogue not present");
        return None;
    }
    let rcdb = RcdbRoot::new(rcdb_dir);
    let before = load_read(&before).ok()?;
    Some(Fx {
        layout: rcdb.layout(&before.manifest.rcdb_version).ok()?,
        links: rcdb.table_links().ok()?,
        options: Options::load(&options_path).ok()?,
        before,
        cps_written,
    })
}

fn decoded(fx: &Fx, ids: &[u16]) -> Vec<DecodedFile> {
    let mut d = decode_read(&fx.before, &fx.layout).unwrap();
    d.retain(|f| ids.contains(&f.id));
    d
}

fn file(files: &mut [DecodedFile], id: u16) -> &mut DecodedFile {
    files.iter_mut().find(|f| f.id == id).unwrap()
}

#[test]
fn deleting_a_channel_reproduces_cps() {
    let Some(fx) = fixture() else { return };
    // Only the channel file is edited: the zone is updated automatically.
    let mut edits = decoded(&fx, &[3]);
    let removed = file(&mut edits, 3).blocks[0].records.remove(2);
    assert_eq!(removed.fields.unwrap().0["Channel Alias"], json!("CH D1"));

    let (files, changes) = apply_edits(&fx.before, &fx.layout, Some(&fx.options), &fx.links, edits).unwrap();
    let cps = |name: &str| std::fs::read(fx.cps_written.join(name)).unwrap();
    assert!(files[&3] == cps("003_QEU_C_DChannel.bin"), "digital channel file differs from CPS's");
    assert!(files[&6] == cps("006_QEU_C_Zone.bin"), "zone file differs from CPS's");
    for (id, bytes) in &files {
        if *id != 3 && *id != 6 {
            assert_eq!(bytes, &fx.before.files[id], "file {id} changed");
        }
    }
    // Reported as: channel 3 removed; zone member Digital 3 removed, Digital 4 renumbered to 3.
    let summary: Vec<(u16, String, String)> = changes.iter().map(|c| (c.file_id, c.position.clone(), c.key.clone())).collect();
    assert!(summary.contains(&(3, "[3]".into(), REMOVED.into())), "{summary:?}");
    let zone = decode(&files[&6], "QEU_C_Zone", &fx.layout).unwrap();
    let members: Vec<(i64, i64)> = zone.blocks[0].records[0]
        .members
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| (m.0["Channel List"].as_i64().unwrap(), m.0["Channel Type"].as_i64().unwrap()))
        .collect();
    assert_eq!(members, vec![(1, 0), (1, 1), (2, 1), (3, 0)]);
}

#[test]
fn deleting_channels_with_unknown_references_is_refused() {
    let Some(fx) = fixture() else { return };
    // Channel 1 is the scan list's designated channel and the stun channel on this radio.
    let mut edits = decoded(&fx, &[3]);
    file(&mut edits, 3).blocks[0].records.remove(0);
    let err = apply_edits(&fx.before, &fx.layout, Some(&fx.options), &fx.links, edits).unwrap_err();
    assert!(format!("{err:#}").contains("channel 1 is used as"), "{err:#}");

    // A scan list using digital channel 4 blocks removing channel 3 (4 would be renumbered);
    // its existing "Selected" member (65535) does not.
    let mut edits = decoded(&fx, &[3, 84]);
    file(&mut edits, 3).blocks[0].records.remove(2);
    file(&mut edits, 84).blocks[0].records[0]
        .members
        .as_mut()
        .unwrap()
        .push(Fields(json!({"Channel List": 4, "Channel Type": 0}).as_object().unwrap().clone()));
    let err = apply_edits(&fx.before, &fx.layout, Some(&fx.options), &fx.links, edits).unwrap_err();
    assert!(format!("{err:#}").contains("scan list"), "{err:#}");
}
