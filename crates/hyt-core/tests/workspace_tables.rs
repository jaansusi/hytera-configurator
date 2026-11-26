//! The GUI's model: tables, record details, edits through `Workspace` (validated like `hyt apply`),
//! the change list, save-as, and the guarded write flow's refusals. Uses the radio's configuration
//! after 2026-11-19 (captures/remove-hyt-test-4_2026-11-19/after). Skipped when the private
//! captures, the CPS install or the option catalogue are missing.

use std::path::{Path, PathBuf};

use hyt_core::edit::apply_edits;
use hyt_core::tables::{Editor, RecordRef, TABLE_IDS, change_list, record_detail, table};
use hyt_core::workspace::{FieldRef, Workspace};
use hyt_core::write_flow::{WriteJob, WriteSettings, write_config};
use hyt_core::{decode_read, load_config_default};
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_proto::Session;
use hytera_proto::transport::ReplayTransport;
use serde_json::json;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rcdb() -> Option<RcdbRoot> {
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    dir.is_dir().then(|| RcdbRoot::new(dir))
}

fn open() -> Option<Workspace> {
    let folder = root().join("captures/remove-hyt-test-4_2026-11-19/after");
    let options = Options::default_path("V1.5.24.000")?;
    let rcdb = rcdb()?;
    if !folder.is_dir() || !options.is_file() {
        eprintln!("skipping: captures, CPS RCDB data or option catalogue not present");
        return None;
    }
    Some(Workspace::open(&folder, &rcdb, None).unwrap())
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hyt-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn texts(t: &hyt_core::tables::Table, column: &str) -> Vec<String> {
    let i = t.columns.iter().position(|c| c.key == column).unwrap();
    t.rows.iter().map(|r| r.cells[i].text.clone()).collect()
}

fn field(t: &hyt_core::tables::Table, row: usize, column: &str) -> FieldRef {
    let i = t.columns.iter().position(|c| c.key == column).unwrap();
    t.rows[row].cells[i].field.clone().unwrap()
}

#[test]
fn tables_show_the_configuration_like_cps() {
    let Some(ws) = open() else { return };
    assert!(ws.options.is_some());

    let digital = table(&ws, "digital").unwrap();
    assert_eq!(digital.capacity, 256);
    assert_eq!(texts(&digital, "Channel Alias"), ["HYT Test 1", "APTLK_CH2"]);
    assert_eq!(texts(&digital, "RX Frequency"), ["444.725", "444.725"]);
    assert_eq!(texts(&digital, "Color Code"), ["9", "14"]);
    assert_eq!(texts(&digital, "Slot Operation"), ["Slot 1", "Slot 2"]);
    assert_eq!(texts(&digital, "Power Level"), ["High", "High"]);
    let rx = digital.columns.iter().find(|c| c.key == "RX Frequency").unwrap();
    assert_eq!(rx.unit, "MHz");
    assert!(matches!(rx.editor, Editor::Number { .. }));
    let power = digital.columns.iter().find(|c| c.key == "Power Level").unwrap();
    assert!(matches!(&power.editor, Editor::Choice { options } if options.iter().any(|o| o == "High")));
    let alias = digital.columns.iter().find(|c| c.key == "Channel Alias").unwrap();
    assert_eq!(alias.editor, Editor::Text { max_len: 16 });

    let analog = table(&ws, "analog").unwrap();
    assert_eq!(texts(&analog, "Channel Alias"), ["446.13125", "446.18125"]);
    assert_eq!(texts(&analog, "Channel Spacing"), ["12.5", "12.5"]);
    assert_eq!(texts(&analog, "RX CTCorCDC Value"), ["107.2", "107.2"]);

    let zones = table(&ws, "zones").unwrap();
    assert_eq!(texts(&zones, "Zone Alias"), ["Zone 1"]);
    assert_eq!(texts(&zones, "members"), ["HYT Test 1, 446.13125, 446.18125"]);
    let members = zones.rows[0].members.as_ref().unwrap();
    assert_eq!(members.iter().map(|m| m.kind.as_str()).collect::<Vec<_>>(), ["Digital", "Analog", "Analog"]);

    let contacts = table(&ws, "contacts").unwrap();
    assert_eq!(texts(&contacts, "Call Alias"), ["Call 1"]);
    assert_eq!(texts(&contacts, "Call ID"), ["1"]);
    assert_eq!(texts(&contacts, "Call Type"), ["Group Call"]);

    for id in TABLE_IDS {
        assert!(table(&ws, id).unwrap().rows.iter().flat_map(|r| &r.cells).all(|c| !c.changed));
    }
}

#[test]
fn passwords_are_never_shown_or_editable() {
    let Some(mut ws) = open() else { return };
    let secrets = ["Write Password", "Read Possword", "Read Password"];
    // Every record of every file: no password field in any detail view.
    let records: Vec<RecordRef> = ws
        .view()
        .iter()
        .flat_map(|f| f.blocks.iter().flat_map(move |b| (0..b.records.len()).map(move |record| RecordRef { file: f.id, block: b.id, record })))
        .collect();
    let mut seen_password_block = false;
    for r in &records {
        let detail = record_detail(&ws, r);
        assert!(detail.iter().all(|d| !secrets.contains(&d.column.key.as_str())), "password shown in {r:?}");
        seen_password_block |= detail.iter().any(|d| d.column.key == "CPS Write Lock");
    }
    assert!(seen_password_block, "the password block's other fields are still listed");
    let comm_basic = ws.view().iter().find(|f| f.name == "Comm_Basic").unwrap();
    let at = FieldRef { file: comm_basic.id, block: 896, record: 0, member: None, key: "Write Password".into() };
    assert!(ws.set_value(&at, "1234").is_err());
    assert!(!ws.has_changes());
}

#[test]
fn edits_are_validated_like_hyt_apply() {
    let Some(mut ws) = open() else { return };
    let digital = table(&ws, "digital").unwrap();

    // Rename and retune channel 2.
    ws.set_value(&field(&digital, 1, "Channel Alias"), "GUI Test 2").unwrap();
    ws.set_value(&field(&digital, 1, "RX Frequency"), "430.0125").unwrap();
    ws.set_value(&field(&digital, 1, "Power Level"), "Low").unwrap();
    assert_eq!(ws.changes().len(), 3);

    // Invalid values are refused and change nothing.
    let before = ws.changes().to_vec();
    for (column, text) in [
        ("Channel Alias", "a name that is far too long"),
        ("RX Frequency", "600"),
        ("RX Frequency", "430.000012"), // off CPS's 5 Hz step grid
        ("Color Code", "16"),
        ("Power Level", "Very high"),
    ] {
        let err = ws.set_value(&field(&digital, 1, column), text);
        assert!(err.is_err(), "{column} = {text:?} was accepted");
    }
    assert_eq!(ws.changes(), before.as_slice());

    // The tables show the new values as changed.
    let digital = table(&ws, "digital").unwrap();
    assert_eq!(texts(&digital, "Channel Alias"), ["HYT Test 1", "GUI Test 2"]);
    assert_eq!(texts(&digital, "RX Frequency"), ["444.725", "430.0125"]);
    let changed: Vec<bool> = digital.rows[1].cells.iter().map(|c| c.changed).collect();
    assert_eq!(changed.iter().filter(|c| **c).count(), 3);
    // Zones list members by their new names.
    assert_eq!(texts(&table(&ws, "zones").unwrap(), "members"), ["HYT Test 1, 446.13125, 446.18125"]);

    // The change list is labelled.
    let list = change_list(&ws);
    let rx = list.iter().find(|c| c.setting == "RX Frequency").unwrap();
    assert_eq!(rx.place, "Digital channel 2 \"GUI Test 2\"");
    assert_eq!((rx.old.as_str(), rx.new.as_str()), ("444.725 MHz", "430.0125 MHz"));

    // Same files as `hyt apply` with the equivalent JSON edit.
    let saved = load_config_default(&ws.folder).unwrap();
    let mut edit = decode_read(&saved, &ws.layout).unwrap().into_iter().find(|f| f.name == "QEU_C_DChannel").unwrap();
    let fields = &mut edit.blocks[0].records[1].fields.as_mut().unwrap().0;
    fields.insert("Channel Alias".into(), json!("GUI Test 2"));
    fields.insert("RX Frequency".into(), json!("430.0125 MHz"));
    fields.insert("Power Level".into(), json!("Low"));
    let links = rcdb().unwrap().table_links().unwrap();
    let (files, _) = apply_edits(&saved, &ws.layout, ws.options.as_ref(), &links, vec![edit]).unwrap();
    assert_eq!(&files, ws.files());

    // Reverting one change, then discarding the rest.
    ws.revert(&rx.field.clone().unwrap()).unwrap();
    assert_eq!(ws.changes().len(), 2);
    ws.discard();
    assert!(!ws.has_changes());
    assert_eq!(ws.files(), &saved.files);
}

#[test]
fn zone_and_contact_edits_and_save_as() {
    let Some(mut ws) = open() else { return };
    ws.set_value(&field(&table(&ws, "zones").unwrap(), 0, "Zone Alias"), "Home").unwrap();
    ws.set_value(&field(&table(&ws, "contacts").unwrap(), 0, "Call ID"), "91").unwrap();
    ws.set_value(&field(&table(&ws, "analog").unwrap(), 0, "TX CTCorCDC Value"), "88.5").unwrap();
    assert_eq!(ws.changes().len(), 3);

    let out = temp_dir("save-as");
    let original = ws.folder.clone();
    ws.save_as(&out).unwrap();
    assert!(!ws.has_changes());
    assert_eq!(ws.folder, out);
    assert_eq!(texts(&table(&ws, "zones").unwrap(), "Zone Alias"), ["Home"]);
    assert_eq!(texts(&table(&ws, "analog").unwrap(), "TX CTCorCDC Value"), ["88.5", "107.2"]);

    // The saved folder keeps the radio's identity (so `hyt write` accepts it) and differs from
    // the original in exactly the edited files.
    let a = load_config_default(&original).unwrap();
    let b = load_config_default(&out).unwrap();
    assert_eq!(a.manifest.serial_number, b.manifest.serial_number);
    let written = |f: &[u8]| hytera_codeplug::container::prepare_for_write(f).unwrap();
    let differing: Vec<String> = a
        .manifest
        .files
        .iter()
        .filter(|f| written(&a.files[&f.id]) != written(&b.files[&f.id]))
        .map(|f| f.name.clone())
        .collect();
    assert_eq!(differing, ["QEU_C_ZoneAlias", "QEU_C_DContact", "QEU_C_AChannel"]);
    // Saving into a non-empty folder is refused.
    assert!(ws.save_as(&out).is_err());
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn write_flow_backs_up_then_refuses_another_model() {
    let Some(rcdb) = rcdb() else { return };
    let capture = root().join("captures/read_2026-11-18.jsonl");
    let folder = root().join("captures/hyt-read_2026-11-18");
    if !capture.is_file() || !folder.is_dir() {
        return;
    }
    let mut saved = load_config_default(&folder).unwrap();
    // Another radio of the same model is fine (it keeps its identity); another model is not.
    saved.manifest.model_name = Some("BP999".into());
    let backup = temp_dir("write-refused");
    let mut session = Session::new(ReplayTransport::from_jsonl_file(&capture).unwrap());
    let job = WriteJob { saved: &saved, source: "test folder", backup: &backup, port: "replay" };
    let err = write_config(&mut session, &rcdb, &job, &WriteSettings::default(), |_| {}).unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("only go onto the same model") && text.contains("nothing was written"), "{text}");
    // The fresh backup was taken first and matches the radio.
    let taken = load_config_default(&backup).unwrap();
    assert_eq!(taken.manifest.serial_number.as_deref(), Some("R23N141382"));
    assert_eq!(taken.files.len(), 58);
    let log = std::fs::read_to_string(backup.join(hyt_core::write_flow::WRITE_LOG)).unwrap();
    assert!(log.contains("FAILED") && log.contains("only go onto the same model"), "{log}");
    std::fs::remove_dir_all(&backup).unwrap();

    // An existing backup folder is refused before talking to the radio.
    std::fs::create_dir_all(&backup).unwrap();
    let mut session = Session::new(ReplayTransport::from_jsonl_file(&capture).unwrap());
    assert!(write_config(&mut session, &rcdb, &job, &WriteSettings::default(), |_| {}).is_err());
    assert_eq!(session.into_transport().writes_matched(), 0);
    std::fs::remove_dir_all(&backup).unwrap();
}
