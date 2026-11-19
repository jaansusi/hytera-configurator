//! Model-number ranges (CPS's `ModelNumber*` rules, exported to `models.json`): for the BP515 U1
//! (model number `55P10B00-1L0BAPU1U1…`, band U1 = 400–470 MHz) frequencies outside the band are
//! refused by `Workspace::set_value` and `apply_edits`, like CPS does, while the generic catalogue
//! would allow 350–527 MHz. Uses captures/unattended_2026-11-19/start; skipped when the private
//! captures, the CPS install or the exported catalogue/rules are missing.

use std::path::{Path, PathBuf};

use hyt_core::edit::{apply_edits, def_for, is_text};
use hyt_core::workspace::{ChannelKind, FieldRef, Workspace};
use hyt_core::{ReadManifest, decode_read, load_layout, load_options, load_options_for_read, load_read, model_number};
use hytera_codeplug::models::ModelRules;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use serde_json::{Value, json};

const VERSION: &str = "V1.5.24.000";
/// Layout ids of RX/TX Frequency in the digital (7, 8) and analog (84, 85) channel lists.
const FREQUENCIES: [u32; 4] = [7, 8, 84, 85];

fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../captures/unattended_2026-11-19/start")
}

fn rcdb() -> Option<RcdbRoot> {
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    dir.is_dir().then(|| RcdbRoot::new(dir))
}

fn present() -> Option<RcdbRoot> {
    let have_data = Options::default_path(VERSION).is_some_and(|p| p.is_file())
        && ModelRules::default_path(VERSION).is_some_and(|p| p.is_file());
    let rcdb = rcdb();
    if !folder().is_dir() || !have_data || rcdb.is_none() {
        eprintln!("skipping: captures, CPS RCDB data or exported options.json/models.json not present");
        return None;
    }
    rcdb
}

/// The RX or TX Frequency field of the first record of channel list `file_name`.
fn frequency(ws: &Workspace, file_name: &str, key: &str) -> FieldRef {
    let file = ws.view().iter().find(|f| f.name == file_name).unwrap();
    let block = file
        .blocks
        .iter()
        .find(|b| b.records.first().and_then(|r| r.fields.as_ref()).is_some_and(|f| f.0.contains_key(key)))
        .unwrap();
    FieldRef { file: file.id, block: block.id, record: 0, member: None, key: key.into() }
}

#[test]
fn bp515_u1_is_narrowed_to_its_band() {
    if present().is_none() {
        return;
    }
    let saved = load_read(&folder()).unwrap();
    assert!(model_number(&saved.manifest).unwrap().starts_with("55P10B00-1L0BAPU1U1"));
    let generic = load_options(None, VERSION).unwrap();
    let (options, note) = load_options_for_read(None, &saved.manifest).unwrap();
    assert_eq!(note, None);
    assert!(options.model().is_some());
    for id in FREQUENCIES {
        assert_eq!(generic.allows(id, 500_000_000), Some(true), "CPS's generic range is 350-527 MHz");
        assert_eq!(options.allows(id, 444_725_000), Some(true));
        assert_eq!(options.allows(id, 400_000_000), Some(true));
        assert_eq!(options.allows(id, 470_000_000), Some(true));
        assert_eq!(options.allows(id, 399_995_000), Some(false));
        assert_eq!(options.allows(id, 470_005_000), Some(false));
        assert_eq!(options.allows(id, 380_000_000), Some(false));
        assert_eq!(options.describe_allowed(id), "400..470 MHz");
        // CPS's default for a new channel on this model (observed: the channel template has it too).
        assert_eq!(options.model_default(id), Some("429075000"));
    }
    // Comm_Basic "Frequency Range" (6163) defaults to the band text.
    assert_eq!(options.model_default(6163), Some("400-470"));
    // "In Call TX Admit" (38): CPS offers no "Always Allow" for this model.
    assert_eq!(generic.allows(38, 0), Some(true));
    assert_eq!(options.allows(38, 0), Some(false));
    assert_eq!(options.allows(38, 1), Some(true));
}

#[test]
fn the_radio_configuration_fits_its_model() {
    let Some(rcdb) = present() else { return };
    let saved = load_read(&folder()).unwrap();
    let layout = load_layout(&saved, &rcdb).unwrap();
    let (options, _) = load_options_for_read(None, &saved.manifest).unwrap();
    let mut outside = Vec::new();
    for file in decode_read(&saved, &layout).unwrap() {
        for block in &file.blocks {
            let maps = block
                .records
                .iter()
                .flat_map(|r| r.fields.iter().map(|f| &f.0).chain(r.members.iter().flatten().map(|m| &m.0)));
            for map in maps {
                for (key, v) in map {
                    let Some(def) = def_for(&layout, file.id, block.id, key) else { continue };
                    if is_text(def) || options.model().is_none_or(|m| !m.ranges.contains_key(&def.id)) {
                        continue;
                    }
                    if let Some(raw) = v.as_i64()
                        && options.allows(def.id, raw) == Some(false)
                    {
                        outside.push(format!("{} {key} = {raw}", file.name));
                    }
                }
            }
        }
    }
    assert!(outside.is_empty(), "values outside the model's ranges: {outside:?}");
}

#[test]
fn out_of_band_frequencies_are_refused() {
    let Some(rcdb) = present() else { return };
    let mut ws = Workspace::open(&folder(), &rcdb, None).unwrap();
    assert_eq!(ws.model_note, None);
    for (file, key) in [
        ("QEU_C_DChannel", "RX Frequency"),
        ("QEU_C_DChannel", "TX Frequency"),
        ("QEU_C_AChannel", "RX Frequency"),
        ("QEU_C_AChannel", "TX Frequency"),
    ] {
        let at = frequency(&ws, file, key);
        for bad in ["480", "399.995", "350 MHz", "527"] {
            let err = ws.set_value(&at, bad).unwrap_err().to_string();
            assert!(err.contains("400..470 MHz"), "{file} {key} {bad}: {err}");
        }
        ws.set_value(&at, "444.725").unwrap();
        ws.set_value(&at, "445.1").unwrap();
        ws.revert(&at).unwrap();
    }
    assert!(!ws.has_changes());

    // A channel added from the template keeps a valid configuration.
    ws.add_channel(ChannelKind::Digital).unwrap();
    ws.add_channel(ChannelKind::Analog).unwrap();
    assert!(ws.has_changes());
}

#[test]
fn apply_edits_refuses_raw_out_of_band_values() {
    let Some(rcdb) = present() else { return };
    let saved = load_read(&folder()).unwrap();
    let layout = load_layout(&saved, &rcdb).unwrap();
    let links = rcdb.table_links().unwrap();
    let (options, _) = load_options_for_read(None, &saved.manifest).unwrap();
    let decoded = decode_read(&saved, &layout).unwrap();
    let set_rx = |raw: i64| {
        let mut files = decoded.clone();
        let file = files.iter_mut().find(|f| f.name == "QEU_C_DChannel").unwrap();
        let record = file.blocks.iter_mut().flat_map(|b| b.records.iter_mut()).find_map(|r| {
            r.fields.as_mut().filter(|f| f.0.contains_key("RX Frequency"))
        });
        record.unwrap().0.insert("RX Frequency".into(), json!(raw));
        files.retain(|f| f.name == "QEU_C_DChannel");
        files
    };
    let err = apply_edits(&saved, &layout, Some(&options), &links, set_rx(480_000_000)).unwrap_err().to_string();
    assert!(err.contains("not allowed") && err.contains("400..470 MHz"), "{err}");
    let (_, changes) = apply_edits(&saved, &layout, Some(&options), &links, set_rx(445_100_000)).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].new, Value::from(445_100_000));
}

#[test]
fn without_a_model_number_the_generic_ranges_apply() {
    if present().is_none() {
        return;
    }
    let saved = load_read(&folder()).unwrap();
    let mut manifest: ReadManifest = serde_json::from_value(serde_json::to_value(&saved.manifest).unwrap()).unwrap();
    manifest.info.retain(|i| i.item != "0x0000");
    let (options, note) = load_options_for_read(None, &manifest).unwrap();
    assert!(note.unwrap().contains("no model number"));
    assert_eq!(options.allows(7, 500_000_000), Some(true));
}
