//! Shared by the tests that need a second BP515: the BP515's configuration
//! (captures/gui-write_2026-11-19/after) and made-up other radios derived from it. Each test file
//! uses some of these.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use hyt_core::workspace::{Workspace, single_field, value_at};
use hyt_core::{SavedRead, load_config_default, save_config};
use hytera_codeplug::container::prepare_for_write;
use hytera_codeplug::decode::decode;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use serde_json::{Value, json};

pub const FOLDER: &str = "captures/gui-write_2026-11-19/after";

pub struct Fx {
    pub rcdb: RcdbRoot,
    pub config: SavedRead,
    pub folder: PathBuf,
}

pub fn fixture() -> Option<Fx> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(FOLDER);
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !dir.is_dir() || !folder.join("manifest.json").is_file() {
        eprintln!("skipping: captures or CPS RCDB data not present");
        return None;
    }
    Some(Fx { rcdb: RcdbRoot::new(dir), config: load_config_default(&folder).ok()?, folder })
}

pub fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hyt-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Value of the single-record field `key` of file `file` in `saved`.
pub fn field(fx: &Fx, saved: &SavedRead, file: &str, key: &str) -> Value {
    let layout = fx.rcdb.layout(&saved.manifest.rcdb_version).unwrap();
    let decoded = hyt_core::decode_read(saved, &layout).unwrap();
    let at = single_field(&decoded, file, key).unwrap_or_else(|| panic!("no {file} {key}"));
    value_at(&decoded, &at).unwrap().clone()
}

pub fn channel_1_alias(fx: &Fx, files: &std::collections::BTreeMap<u16, Vec<u8>>) -> Value {
    let layout = fx.rcdb.layout("V1.5.24.000").unwrap();
    let ch = decode(&files[&3], "QEU_C_DChannel", &layout).unwrap();
    ch.blocks[0].records[0].fields.as_ref().unwrap().0["Channel Alias"].clone()
}

/// Another BP515: the fixture with serial number `serial` (manifest, info item and Comm_Basic's
/// radio data), Radio ID `id`, alias `alias` and channel 1 renamed, saved as a new folder.
pub fn other_radio(fx: &Fx, name: &str, serial: &str, id: &str, alias: &str) -> (SavedRead, PathBuf) {
    let mut ws = Workspace::open(&fx.folder, &fx.rcdb, None).unwrap();
    for (file, key, text) in [("QEU_C_Config", "Radio ID", id), ("Comm_Basic", "Radio Alias", alias)] {
        let at = single_field(ws.view(), file, key).unwrap();
        ws.set_value(&at, text).unwrap();
    }
    let ch = ws.view().iter().find(|f| f.name == "QEU_C_DChannel").unwrap();
    let at = hyt_core::workspace::FieldRef { file: ch.id, block: ch.blocks[0].id, record: 0, member: None, key: "Channel Alias".into() };
    ws.set_value(&at, "Target CH1").unwrap();
    let step = temp(&format!("{name}-step"));
    ws.save_as(&step).unwrap();
    let mut saved = load_config_default(&step).unwrap();
    std::fs::remove_dir_all(&step).unwrap();

    let layout = fx.rcdb.layout("V1.5.24.000").unwrap();
    let mut basic = decode(&saved.files[&18], "Comm_Basic", &layout).unwrap();
    for b in &mut basic.blocks {
        for r in &mut b.records {
            if let Some(f) = r.fields.as_mut().filter(|f| f.0.contains_key("Serial Number")) {
                f.0.insert("Serial Number".into(), json!(serial_number(serial)));
            }
        }
    }
    let rebuilt = hytera_codeplug::decode::apply(&saved.files[&18], &basic, &layout).unwrap();
    saved.files.insert(18, prepare_for_write(&rebuilt).unwrap());
    saved.manifest.serial_number = Some(serial.into());
    let item = saved.manifest.info.iter_mut().find(|e| e.item == "0x000B").unwrap();
    let len = item.hex.len() / 4;
    let mut units: Vec<u16> = serial.encode_utf16().collect();
    units.resize(len.max(units.len()), 0);
    item.hex = units.iter().map(|u| format!("{u:04X}")).collect();
    item.text = Some(serial.into());
    let out = temp(name);
    // Saved as that radio's own read: a configuration folder takes its radio from `radio/`.
    saved.radio = None;
    save_config(&saved, &saved.files, &out, "test: another BP515", &hyt_core::load_layout(&saved, &fx.rcdb).unwrap()).unwrap();
    (load_config_default(&out).unwrap(), out)
}


/// Comm_Basic's Serial Number field for a serial like `R23N999999`: its digits (`23999999`).
fn serial_number(serial: &str) -> u32 {
    serial.chars().filter(char::is_ascii_digit).collect::<String>().parse::<u32>().unwrap_or(987654321) % 1_000_000_000
}
