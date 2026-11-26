//! Editing values in place, on the real BP515 read. Skipped when the private captures, the CPS
//! install or the option catalogue are missing.

use std::path::{Path, PathBuf};

use hyt_core::edit::{apply_edits, changes, labelize};
use hyt_core::{SavedRead, decode_read, load_config_default, plan_write, save_config};
use hytera_codeplug::container::{CrcCheck, check_crc};
use hytera_codeplug::decode::{DecodedFile, decode};
use hytera_codeplug::layout::Layout;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_models::BP515;
use serde_json::{Value, json};

struct Fx {
    saved: SavedRead,
    layout: Layout,
    options: Options,
    rcdb: RcdbRoot,
}

fn fixture() -> Option<Fx> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rcdb_dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let folder = root.join("captures/hyt-read-after-write_2026-11-18");
    let options_path = Options::default_path("V1.5.24.000")?;
    if !rcdb_dir.is_dir() || !folder.is_dir() || !options_path.is_file() {
        eprintln!("skipping: captures, CPS RCDB data or option catalogue not present");
        return None;
    }
    let rcdb = RcdbRoot::new(rcdb_dir);
    let saved = load_config_default(&folder).ok()?;
    let layout = rcdb.layout(&saved.manifest.rcdb_version).ok()?;
    let options = Options::load(&options_path).ok()?;
    Some(Fx { saved, layout, options, rcdb })
}

fn labelled(fx: &Fx) -> Vec<DecodedFile> {
    let mut d = decode_read(&fx.saved, &fx.layout).unwrap();
    labelize(&mut d, &fx.layout, &fx.options);
    // Through JSON text, as a user would edit it.
    serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap()
}

fn record(files: &mut [DecodedFile], file: u16, index: usize) -> &mut serde_json::Map<String, Value> {
    let f = files.iter_mut().find(|f| f.id == file).unwrap();
    &mut f.blocks[0].records[index].fields.as_mut().unwrap().0
}

#[test]
fn unchanged_labelled_json_applies_without_changes() {
    let Some(fx) = fixture() else { return };
    let edits = labelled(&fx);
    let labelled_values =
        serde_json::to_string(&edits).unwrap().matches("\": \"").count() + serde_json::to_string(&edits).unwrap().matches("\":\"").count();
    assert!(labelled_values > 900, "expected many labelled values, got {labelled_values}");
    let (files, changes) = apply_edits(&fx.saved, &fx.layout, Some(&fx.options), &fx.rcdb.table_links().unwrap(), edits).unwrap();
    assert!(changes.is_empty(), "unexpected changes: {changes:?}");
    assert_eq!(files, fx.saved.files);
}

#[test]
fn edits_change_exactly_the_requested_fields() {
    let Some(fx) = fixture() else { return };
    let mut edits = labelled(&fx);
    let ch1 = record(&mut edits, 3, 0);
    ch1.insert("Channel Alias".into(), json!("Home DMR"));
    ch1.insert("RX Frequency".into(), json!("445.0125 MHz"));
    ch1.insert("TX Frequency".into(), json!(445_012_500));
    ch1.insert("Power Level".into(), json!("Low"));
    let ach2 = record(&mut edits, 9, 1);
    ach2.insert("RX CTCorCDC Value".into(), json!("88.5"));
    ach2.insert("Channel Spacing".into(), json!("25"));
    // Only send the two edited files, as a user may.
    edits.retain(|f| f.id == 3 || f.id == 9);

    let (files, changes) = apply_edits(&fx.saved, &fx.layout, Some(&fx.options), &fx.rcdb.table_links().unwrap(), edits).unwrap();
    let mut got: Vec<(u16, &str, String)> = changes.iter().map(|c| (c.file_id, c.key.as_str(), c.new.to_string())).collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            (3, "Channel Alias", "\"Home DMR\"".into()),
            (3, "Power Level", "2".into()),
            (3, "RX Frequency", "445012500".into()),
            (3, "TX Frequency", "445012500".into()),
            (9, "Channel Spacing", "2".into()),
            (9, "RX CTCorCDC Value", "885".into()),
        ]
    );
    // Untouched files are byte-identical; edited files have valid structure and CRC.
    for (id, bytes) in &files {
        if *id == 3 || *id == 9 {
            assert_eq!(check_crc(bytes), CrcCheck::Body, "file {id}");
            let name = fx.saved.manifest.files.iter().find(|f| f.id == *id).unwrap().name.clone();
            let before = decode(&fx.saved.files[id], &name, &fx.layout).unwrap();
            let after = decode(bytes, &name, &fx.layout).unwrap();
            assert_eq!(changes_count(&before, &after), if *id == 3 { 4 } else { 2 });
        } else {
            assert_eq!(bytes, &fx.saved.files[id], "file {id} changed");
        }
    }

    // The saved folder is a valid source for `hyt write`.
    let out = std::env::temp_dir().join(format!("hyt-edit-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    save_config(&fx.saved, &files, &out, "test", &hyt_core::load_layout(&fx.saved, &fx.rcdb).unwrap()).unwrap();
    let reloaded = load_config_default(&out).unwrap();
    assert_eq!(reloaded.manifest.serial_number, fx.saved.manifest.serial_number);
    let list = fx.rcdb.file_list(&reloaded.manifest.rcdb_version).unwrap();
    let links = fx.rcdb.table_links().unwrap();
    assert_eq!(plan_write(&reloaded.files, &list, &links, &BP515).unwrap().len(), 55);
    std::fs::remove_dir_all(&out).unwrap();
}

fn changes_count(a: &DecodedFile, b: &DecodedFile) -> usize {
    changes(a, b).unwrap().len()
}

#[test]
fn invalid_values_are_refused() {
    let Some(fx) = fixture() else { return };
    for (key, value, expect) in [
        ("Power Level", json!("Medium"), "not a valid value"),
        ("RX Frequency", json!("600 MHz"), "not a valid value"),
        ("RX Frequency", json!(445_012_501), "not allowed"),
        ("Channel Alias", json!("This name is far too long"), "longer than 16"),
    ] {
        let mut edits = labelled(&fx);
        record(&mut edits, 3, 0).insert(key.into(), value.clone());
        edits.retain(|f| f.id == 3);
        let err = apply_edits(&fx.saved, &fx.layout, Some(&fx.options), &fx.rcdb.table_links().unwrap(), edits).unwrap_err();
        assert!(format!("{err:#}").contains(expect), "{key} = {value}: {err:#}");
    }
}

#[test]
fn duplicate_contacts_are_refused_like_cps() {
    let Some(fx) = fixture() else { return };
    let links = fx.rcdb.table_links().unwrap();
    let with_extra = |call_id: i64| {
        let mut contacts: Vec<DecodedFile> = decode_read(&fx.saved, &fx.layout).unwrap().into_iter().filter(|f| f.name == "QEU_C_DContact").collect();
        let first = contacts[0].blocks[0].records[0].fields.clone().unwrap();
        let mut f = first.0.clone();
        f.insert("Call Alias".into(), json!("Twin"));
        f.insert("Call ID".into(), json!(call_id));
        contacts[0].blocks[0].records.push(hytera_codeplug::decode::Record { index: None, fields: Some(hytera_codeplug::decode::Fields(f)), members: None });
        apply_edits(&fx.saved, &fx.layout, Some(&fx.options), &links, contacts)
    };
    let err = with_extra(1).unwrap_err();
    assert!(format!("{err:#}").contains("same call ID 1"), "{err:#}");
    assert!(with_extra(2).is_ok());
}
