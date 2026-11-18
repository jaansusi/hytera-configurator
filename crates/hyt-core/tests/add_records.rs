//! Adding records and members, checked against CPS's own recording of adding a digital channel
//! and putting it in Zone 1 (captures/add-channel-*_2026-11-18). Skipped when the private
//! captures, the CPS install or the option catalogue are missing.

use std::path::{Path, PathBuf};

use hyt_core::edit::{ADDED, apply_edits};
use hyt_core::{SavedRead, decode_read, load_read};
use hytera_codeplug::decode::{DecodedFile, Fields, Record, decode};
use hytera_codeplug::layout::Layout;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot, TableLink};
use serde_json::{Map, json};

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
    let before = root.join("captures/add-channel-read_2026-11-18_files");
    let cps_written = root.join("captures/add-channel-write_2026-11-18_files");
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

fn fields(v: serde_json::Value) -> Fields {
    Fields(v.as_object().cloned().unwrap_or_default())
}

fn add_empty_record(f: &mut DecodedFile) {
    let records = &mut f.blocks[0].records;
    records.push(Record { index: None, fields: Some(Fields(Map::new())), members: None });
}

fn zone_members(f: &mut DecodedFile, zone: usize) -> &mut Vec<Fields> {
    f.blocks[0].records[zone].members.as_mut().unwrap()
}

fn cps(fx: &Fx, name: &str) -> Vec<u8> {
    std::fs::read(fx.cps_written.join(name)).unwrap()
}

#[test]
fn adding_a_channel_and_zone_member_reproduces_cps() {
    let Some(fx) = fixture() else { return };
    let mut edits = decoded(&fx, &[3, 6]);
    add_empty_record(file(&mut edits, 3));
    zone_members(file(&mut edits, 6), 0).push(fields(json!({"Channel List": 3, "Channel Type": "Digital"})));

    let (files, changes) = apply_edits(&fx.before, &fx.layout, Some(&fx.options), &fx.links, edits).unwrap();
    assert_eq!(changes.iter().filter(|c| c.key == ADDED).count(), 2, "{changes:?}");
    assert_eq!(changes.len(), 2);
    assert!(files[&3] == cps(&fx, "003_QEU_C_DChannel.bin"), "digital channel file differs from CPS's");
    assert!(files[&6] == cps(&fx, "006_QEU_C_Zone.bin"), "zone file differs from CPS's");
    for (id, bytes) in &files {
        if *id != 3 && *id != 6 {
            assert_eq!(bytes, &fx.before.files[id], "file {id} changed");
        }
    }
    // The new channel is CPS's template channel.
    let ch = decode(&files[&3], "QEU_C_DChannel", &fx.layout).unwrap();
    let new = &ch.blocks[0].records[2].fields.as_ref().unwrap().0;
    assert_eq!(new["Channel Alias"], json!("CH D1"));
    assert_eq!(new["RX Frequency"], json!(429_075_000));
}

#[test]
fn new_record_takes_given_values() {
    let Some(fx) = fixture() else { return };
    let mut edits = decoded(&fx, &[3]);
    let ch = file(&mut edits, 3);
    add_empty_record(ch);
    let rec = ch.blocks[0].records[2].fields.as_mut().unwrap();
    rec.0.insert("Channel Alias".into(), json!("Repeater"));
    rec.0.insert("RX Frequency".into(), json!("438.5 MHz"));
    let (files, _) = apply_edits(&fx.before, &fx.layout, Some(&fx.options), &fx.links, edits).unwrap();
    let d = decode(&files[&3], "QEU_C_DChannel", &fx.layout).unwrap();
    let new = &d.blocks[0].records[2].fields.as_ref().unwrap().0;
    assert_eq!((new["Channel Alias"].clone(), new["RX Frequency"].clone()), (json!("Repeater"), json!(438_500_000)));
    assert_eq!(new["TX Frequency"], json!(429_075_000), "fields not given come from the template");
}

#[test]
fn zones_and_aliases_are_added_together_and_members_can_be_removed() {
    let Some(fx) = fixture() else { return };
    // A second zone with one member, plus its alias.
    let mut edits = decoded(&fx, &[6, 7]);
    let zones = file(&mut edits, 6);
    zones.blocks[0].records.push(Record {
        index: None,
        fields: None,
        members: Some(vec![fields(json!({"Channel List": 2, "Channel Type": 1}))]),
    });
    zone_members(zones, 0).remove(1); // drop "Analog 1" from Zone 1
    let alias = file(&mut edits, 7);
    add_empty_record(alias);
    alias.blocks[0].records[1].fields.as_mut().unwrap().0.insert("Zone Alias".into(), json!("Zone 2"));
    let (files, _) = apply_edits(&fx.before, &fx.layout, Some(&fx.options), &fx.links, edits).unwrap();

    let z = decode(&files[&6], "QEU_C_Zone", &fx.layout).unwrap();
    let members = |i: usize| -> Vec<(i64, i64)> {
        z.blocks[0].records[i]
            .members
            .as_ref()
            .unwrap()
            .iter()
            .map(|m| (m.0["Channel List"].as_i64().unwrap(), m.0["Channel Type"].as_i64().unwrap()))
            .collect()
    };
    assert_eq!(members(0), vec![(1, 0), (2, 1)]);
    assert_eq!(members(1), vec![(2, 1)]);
    let a = decode(&files[&7], "QEU_C_ZoneAlias", &fx.layout).unwrap();
    assert_eq!(a.blocks[0].records[1].fields.as_ref().unwrap().0["Zone Alias"], json!("Zone 2"));
}

#[test]
fn inconsistent_edits_are_refused() {
    let Some(fx) = fixture() else { return };
    type Edit = Box<dyn Fn(&mut Vec<DecodedFile>)>;
    let cases: Vec<(&str, Edit)> = vec![
        ("only channels", Box::new(|e: &mut Vec<DecodedFile>| {
            file(e, 8).blocks[0].records.pop();
        })),
        ("does not exist", Box::new(|e: &mut Vec<DecodedFile>| {
            zone_members(file(e, 6), 0).push(fields(json!({"Channel List": 3, "Channel Type": 0})));
        })),
        ("add or remove both together", Box::new(|e: &mut Vec<DecodedFile>| {
            let zones = file(e, 6);
            zones.blocks[0].records.push(Record { index: None, fields: None, members: Some(vec![]) });
        })),
        ("not a valid value", Box::new(|e: &mut Vec<DecodedFile>| {
            zone_members(file(e, 6), 0).push(fields(json!({"Channel List": 1, "Channel Type": "Trunked"})));
        })),
    ];
    for (expect, edit) in cases {
        let mut edits = decoded(&fx, &[3, 6, 7, 8]);
        edit(&mut edits);
        let err = apply_edits(&fx.before, &fx.layout, Some(&fx.options), &fx.links, edits).unwrap_err();
        assert!(format!("{err:#}").contains(expect), "expected {expect:?}, got: {err:#}");
    }
}
