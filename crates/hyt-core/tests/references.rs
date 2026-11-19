//! References from CPS's own configuration (`DataValidationConfig.config`,
//! `ComplexRuleConfig.Config`) and CPS's link rules (`links.json`) on the radio's configuration of
//! 2026-11-19 (captures/unattended_2026-11-19/start). Skipped when the private captures, the CPS
//! install, the option catalogue or the exported link rules are missing.

use std::path::{Path, PathBuf};

use hyt_core::refs::Source;
use hyt_core::tables::{Editor, change_list, record_detail, table};
use hyt_core::workspace::{FieldRef, Workspace};
use hytera_codeplug::link_rules::LinkRules;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};

fn open() -> Option<Workspace> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../captures/unattended_2026-11-19/start");
    let rcdb_dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let options = Options::default_path("V1.5.24.000");
    if !folder.is_dir() || !rcdb_dir.join("DataValidationConfig.config").is_file() || !options.is_some_and(|o| o.is_file()) {
        eprintln!("skipping: captures, CPS install or option catalogue not present");
        return None;
    }
    Some(Workspace::open(&folder, &RcdbRoot::new(rcdb_dir), None).unwrap())
}

fn has_rules() -> bool {
    let present = LinkRules::default_path("V1.5.24.000").is_some_and(|p| p.is_file());
    if !present {
        eprintln!("skipping link rule checks: run tools/export-rcdb-links.ps1");
    }
    present
}

/// The detail field `key` of the first record of table `id`.
fn detail(ws: &Workspace, id: &str, row: usize, key: &str) -> hyt_core::tables::DetailField {
    let record = table(ws, id).unwrap().rows[row].record.clone();
    record_detail(ws, &record).into_iter().find(|d| d.column.key == key).unwrap_or_else(|| panic!("{id}: no field {key}"))
}

fn options_of(cell: &hyt_core::tables::Cell) -> Vec<String> {
    match &cell.editor {
        Some(Editor::Choice { options }) => options.clone(),
        other => panic!("not a choice: {other:?}"),
    }
}

#[test]
fn references_come_from_cps_config() {
    let Some(ws) = open() else { return };
    assert_eq!(ws.refs.source, Source::CpsConfig);
    let mut fields: Vec<String> = ws.refs.fields().map(|(f, b, k)| format!("{f}/{b} {k}")).collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            "3/1 Emergency System",
            "3/1 Encrypt Key",
            "3/1 Location Info Revert Channel",
            "3/1 Phone System",
            "3/1 RRS Channel",
            "3/1 RX Group List",
            "3/1 Scan List Roam List",
            "3/1 TX Contacts ID",
            "85/620 Priority Channel 1",
            "85/620 Priority Channel 2",
            "9/4 Emergency System",
            "9/4 Scan List",
            "95/636 Group Call ID",
            "95/636 Revert Channel",
            "96/637 Revert Channel",
        ]
    );
}

#[test]
fn digital_channel_references_and_special_values() {
    let Some(mut ws) = open() else { return };
    // Emergency System -> digital emergency systems (1-based, 0 = None).
    let e = detail(&ws, "digital", 0, "Emergency System");
    assert_eq!(e.cell.text, "DigitalSys 1");
    assert_eq!(options_of(&e.cell), ["None", "DigitalSys 1"]);
    // Phone System -> phone systems (none on this radio).
    let p = detail(&ws, "digital", 0, "Phone System");
    assert_eq!((p.cell.text.as_str(), options_of(&p.cell)), ("None", vec!["None".to_string()]));
    // RRS / location revert channel -> digital channels, 0 = Selected, 65535 = None.
    let rrs = detail(&ws, "digital", 0, "RRS Channel");
    assert_eq!(rrs.cell.text, "None");
    assert_eq!(options_of(&rrs.cell), ["Selected", "None", "HYT Test 1", "HYT Test 2"]);
    let at = rrs.cell.field.clone().unwrap();
    ws.set_value(&at, "HYT Test 2").unwrap();
    assert_eq!(detail(&ws, "digital", 0, "RRS Channel").cell.text, "HYT Test 2");
    assert_eq!(hyt_core::workspace::value_at(ws.view(), &at).and_then(|v| v.as_i64()), Some(2));
    ws.set_value(&at, "Selected").unwrap();
    assert_eq!(hyt_core::workspace::value_at(ws.view(), &at).and_then(|v| v.as_i64()), Some(0));
    let c = change_list(&ws);
    assert_eq!((c[0].old.as_str(), c[0].new.as_str()), ("None", "Selected"));
    ws.discard();
    let l = detail(&ws, "digital", 1, "Location Info Revert Channel");
    assert_eq!(l.cell.text, "None");
}

#[test]
fn emergency_and_scan_references() {
    let Some(ws) = open() else { return };
    // Digital emergency system: revert channel (0 = Selected) and group call ID (a contact).
    let t = ws.view().iter().find(|f| f.name == "QEU_C_DEmergency").unwrap();
    let at = |key: &str| FieldRef { file: t.id, block: t.blocks[0].id, record: 0, member: None, key: key.into() };
    assert_eq!(ws.reference_names(&at("Revert Channel")).unwrap()[0], (0, "Selected".to_string()));
    let group = ws.reference_names(&at("Group Call ID")).unwrap();
    assert_eq!(hyt_core::refs::label(&group, 1).as_deref(), Some("Call 1"));
    // Analog emergency revert channel names analog channels.
    let a = ws.view().iter().find(|f| f.name == "QEU_C_AEmergency").unwrap();
    let revert = FieldRef { file: a.id, block: a.blocks[0].id, record: 0, member: None, key: "Revert Channel".into() };
    let names: Vec<String> = ws.reference_names(&revert).unwrap().into_iter().map(|(_, n)| n).collect();
    assert_eq!(names, ["Selected", "446.13125", "446.18125"]);

    // Scan priority channels: members of the same scan list, 0 = None, 65535 = Selected; only the
    // special values can be chosen (the companion type field is not understood yet).
    let scan = table(&ws, "list:QEU_C_ScanConfig").unwrap();
    let i = scan.columns.iter().position(|c| c.key == "Priority Channel 1").unwrap();
    let cell = &scan.rows[0].cells[i];
    assert_eq!(cell.text, "None");
    assert_eq!(options_of(cell), ["None", "Selected"]);
    let names = ws.reference_names(cell.field.as_ref().unwrap()).unwrap();
    assert_eq!(names, [(0, "None".to_string()), (65535, "Selected".to_string()), (1, "Selected (member 1)".to_string())]);
    let mut ws = ws;
    let err = ws.set_value(cell.field.as_ref().unwrap(), "Selected (member 1)").unwrap_err().to_string();
    assert!(err.contains("not supported yet"), "{err}");
    ws.set_value(cell.field.as_ref().unwrap(), "Selected").unwrap();
    assert_eq!(table(&ws, "list:QEU_C_ScanConfig").unwrap().rows[0].cells[i].text, "Selected");
}

#[test]
fn rx_only_greys_out_tx_settings_like_cps() {
    let Some(mut ws) = open() else { return };
    if !has_rules() {
        return;
    }
    assert!(ws.rules_note.is_none(), "{:?}", ws.rules_note);
    // CPS rule 459/461: a digital channel's TX settings are greyed out while RX Only is on.
    let power = detail(&ws, "digital", 0, "Power Level");
    assert!(power.cell.disabled.is_none() && power.cell.editable);
    let rx_only = detail(&ws, "digital", 0, "RX Only").cell.field.unwrap();
    ws.set_value(&rx_only, "On").unwrap();
    let power = detail(&ws, "digital", 0, "Power Level");
    assert!(!power.cell.editable);
    assert_eq!(power.cell.disabled.as_deref(), Some("CPS greys this out while RX Only is On"));
    let err = ws.set_value(power.cell.field.as_ref().unwrap(), "Low").unwrap_err().to_string();
    assert!(err.contains("cannot be changed: CPS greys this out while RX Only is On"), "{err}");
    // The other channel is not affected.
    assert!(detail(&ws, "digital", 1, "Power Level").cell.disabled.is_none());
    // Also in the table.
    let t = table(&ws, "digital").unwrap();
    let i = t.columns.iter().position(|c| c.key == "Power Level").unwrap();
    assert!(t.rows[0].cells[i].disabled.is_some() && t.rows[1].cells[i].disabled.is_none());
    ws.set_value(&rx_only, "Off").unwrap();
    assert!(detail(&ws, "digital", 0, "Power Level").cell.disabled.is_none());
    assert!(!ws.has_changes());
}

#[test]
fn quick_gps_forces_revert_channels_to_none() {
    let Some(mut ws) = open() else { return };
    if !has_rules() {
        return;
    }
    // CPS rule 290: Quick GPS on -> RRS Channel and Location Info Revert Channel set to 65535
    // (None) and greyed out. Start from an RRS channel set, so the forced value shows.
    // CPS hides Quick GPS itself on this radio (delete_flag 1, tests/visibility.rs); this test is
    // about the link rule engine, so the visibility classification is switched off here.
    ws.hidden.clear();
    let rrs = detail(&ws, "digital", 0, "RRS Channel").cell.field.unwrap();
    ws.set_value(&rrs, "HYT Test 1").unwrap();
    let quick = detail(&ws, "digital", 0, "Quick GPS").cell.field.unwrap();
    ws.set_value(&quick, "On").unwrap();
    let cell = detail(&ws, "digital", 0, "RRS Channel").cell;
    assert_eq!(cell.text, "None");
    assert_eq!(cell.disabled.as_deref(), Some("CPS greys this out while Quick GPS is On"));
    // One pending change remains: Quick GPS (RRS Channel is back at its original None).
    let changes = change_list(&ws);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].setting, "Quick GPS");
}

#[test]
fn tx_tone_value_follows_tone_type() {
    let Some(mut ws) = open() else { return };
    if !has_rules() {
        return;
    }
    // CPS rules 460 (RX Only off: TX tone value greyed) and 122 (TX tone type set and RX Only off:
    // enabled): the more specific rule wins.
    let tx_type = detail(&ws, "analog", 0, "TX CTCorCDC Type");
    let value = detail(&ws, "analog", 0, "TX CTCorCDC Value");
    assert_ne!(tx_type.cell.text, "None", "this radio's analog channel 1 has a TX tone");
    assert!(value.cell.disabled.is_none());
    ws.set_value(tx_type.cell.field.as_ref().unwrap(), "None").unwrap();
    let value = detail(&ws, "analog", 0, "TX CTCorCDC Value");
    assert_eq!(value.cell.disabled.as_deref(), Some("CPS greys this out while RX Only is Off"));
}

#[test]
fn without_link_rules_nothing_is_disabled() {
    let Some(ws) = open() else { return };
    let missing = std::env::temp_dir().join(format!("hyt-no-rules-{}", std::process::id()));
    std::fs::create_dir_all(&missing).unwrap();
    let options = missing.join("options.json");
    std::fs::copy(Options::default_path("V1.5.24.000").unwrap(), &options).unwrap();
    let rcdb = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let plain = Workspace::open(&ws.folder, &RcdbRoot::new(rcdb), Some(&options)).unwrap();
    assert!(plain.rules.is_empty() && plain.rules_note.is_some());
    let analog = table(&plain, "analog").unwrap();
    assert!(analog.rows.iter().all(|r| r.cells.iter().all(|c| c.disabled.is_none())));
    std::fs::remove_dir_all(&missing).unwrap();
}

/// Open question 5e: with an empty key list (242), a channel's Encrypt Key can only be None.
#[test]
fn encrypt_key_names_only_existing_keys() {
    let Some(ws) = open() else { return };
    // Shown by name, and only None can be chosen (the GUI does not offer it at all on this radio:
    // it needs the Basic Encryption licence, see tests/visibility.rs).
    let field = detail(&ws, "digital", 0, "Encrypt Key");
    assert_eq!(field.cell.text, "None");
    assert_eq!(options_of(&field.cell), ["None"], "the key list is empty");
    // Edits that do not go through the GUI (hyt apply) cannot point at a key that does not exist,
    // but may still switch a channel's encryption on.
    let saved = hyt_core::load_read(&ws.folder).unwrap();
    let with = |encrypt: i64, key: i64| {
        let mut edits = ws.edit_form().to_vec();
        let dch = edits.iter_mut().find(|f| f.name == "QEU_C_DChannel").unwrap();
        let fields = &mut dch.blocks[0].records[0].fields.as_mut().unwrap().0;
        fields.insert("Encrypt".into(), serde_json::json!(encrypt));
        fields.insert("Encrypt Key".into(), serde_json::json!(key));
        hyt_core::edit::apply_edits(&saved, &ws.layout, ws.options.as_ref(), ws.links(), edits)
    };
    let err = with(1, 3).unwrap_err();
    assert!(format!("{err:#}").contains("key 3 does not exist (the radio's key list is empty)"), "{err:#}");
    let (_, changes) = with(1, 0).unwrap();
    assert!(changes.iter().any(|c| c.key == "Encrypt"));
}
