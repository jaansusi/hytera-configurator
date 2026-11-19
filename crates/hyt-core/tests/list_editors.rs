//! List editors (`hyt_core::lists`): RX group, scan and roam lists and the other one-level lists
//! as tables, members read-only by name, value edits through `Workspace::set_value`, and the
//! change list. Uses the radio's configuration of 2026-11-19 (captures/unattended_2026-11-19/start).
//! Skipped when the private captures, the CPS install or the option catalogue are missing.

use std::path::{Path, PathBuf};

use hyt_core::lists;
use hyt_core::tables::{Table, change_list, record_detail, table};
use hyt_core::workspace::{FieldRef, Workspace};
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};

fn open() -> Option<Workspace> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../captures/unattended_2026-11-19/start");
    let rcdb_dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let options = Options::default_path("V1.5.24.000");
    if !folder.is_dir() || !rcdb_dir.is_dir() || !options.is_some_and(|o| o.is_file()) {
        eprintln!("skipping: captures, CPS RCDB data or option catalogue not present");
        return None;
    }
    Some(Workspace::open(&folder, &RcdbRoot::new(rcdb_dir), None).unwrap())
}

fn texts(t: &Table, column: &str) -> Vec<String> {
    let i = t.columns.iter().position(|c| c.key == column).unwrap_or_else(|| panic!("{}: no column {column}", t.id));
    t.rows.iter().map(|r| r.cells[i].text.clone()).collect()
}

fn field(t: &Table, row: usize, column: &str) -> FieldRef {
    let i = t.columns.iter().position(|c| c.key == column).unwrap();
    t.rows[row].cells[i].field.clone().unwrap()
}

#[test]
fn lists_show_the_radios_records() {
    let Some(ws) = open() else { return };
    let index = lists::index(&ws);
    let ids: Vec<&str> = index.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "list:QEU_C_RxGrpAlias",
            "list:QEU_C_ScanConfig",
            "list:QEU_C_RoamConfig",
            "list:QEU_C_OneTouchCall",
            "list:QEU_C_PresetChannel",
            "list:QEU_C_Quick_Message",
            "list:QEU_C_PhoneList",
            "list:QEU_C_PhoneSystem",
            "list:QEU_C_HDC1200_System",
            "list:QEU_C_HDC1200_Contact",
            "list:QEU_C_2Tone_System",
            "list:QEU_C_2Tone_Contact",
            "list:QEU_C_2Tone_Personality",
        ]
    );
    // Files the BP515 profile never writes are not offered.
    assert!(table(&ws, "list:QEU_C_E2EEKEYLIST").is_none());

    let rx = table(&ws, "list:QEU_C_RxGrpAlias").unwrap();
    assert_eq!(texts(&rx, "Rx Group List Alias"), ["Rx Group List 1"]);
    assert_eq!(rx.capacity, 16, "RX group list members file holds 16 lists (aliases: 32)");
    assert_eq!(rx.rows[0].list_members.as_ref().unwrap().len(), 0);
    assert_eq!(texts(&rx, "members"), [""]);

    let scan = table(&ws, "list:QEU_C_ScanConfig").unwrap();
    assert_eq!(texts(&scan, "Scan List Alias"), ["Scan List 1"]);
    assert_eq!(texts(&scan, "Scan Type"), ["Normal"]);
    assert_eq!(texts(&scan, "Priority Channel 1"), ["None"]);
    assert_eq!(scan.capacity, 8);
    let members = scan.rows[0].list_members.as_ref().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!((members[0].name.as_str(), members[0].refers_to.as_str()), ("Selected", "The selected channel"));
    assert_eq!(texts(&scan, "members"), ["Selected"]);

    let roam = table(&ws, "list:QEU_C_RoamConfig").unwrap();
    assert_eq!(texts(&roam, "Roam List Alias"), ["Roam List 1"]);
    assert_eq!(texts(&roam, "RSSI Threshold"), ["-108"]);
    assert_eq!(roam.rows[0].list_members.as_ref().unwrap()[0].name, "Selected");

    assert_eq!(texts(&table(&ws, "list:QEU_C_Quick_Message").unwrap(), "Text Message Content"), ["None"]);
    assert_eq!(texts(&table(&ws, "list:QEU_C_HDC1200_Contact").unwrap(), "Call Alias"), ["HDC1200 Call 1"]);
    assert_eq!(texts(&table(&ws, "list:QEU_C_2Tone_System").unwrap(), "System Alias"), ["System 1"]);
    assert_eq!(table(&ws, "list:QEU_C_OneTouchCall").unwrap().rows.len(), 5);
    assert_eq!(table(&ws, "list:QEU_C_PresetChannel").unwrap().rows.len(), 4);
    let phones = table(&ws, "list:QEU_C_PhoneList").unwrap();
    assert!(phones.rows.is_empty());
    assert!(phones.note.as_deref().unwrap().contains("recording of CPS"));

    for info in &index {
        let t = table(&ws, &info.id).unwrap();
        // Nothing structural is offered, every column is a real field, nothing is changed yet.
        assert!(!t.resizable, "{} offers adding/removing", t.id);
        assert!(t.item.is_some());
        for row in &t.rows {
            assert!(!row.added);
            for (col, cell) in t.columns.iter().zip(&row.cells) {
                assert!(col.key == "members" || cell.field.is_some(), "{}: column {} has no field", t.id, col.key);
                assert!(!cell.changed);
            }
            assert!(!record_detail(&ws, &row.record).is_empty());
        }
    }
}

#[test]
fn list_values_are_edited_through_the_validated_path() {
    let Some(mut ws) = open() else { return };
    let scan = table(&ws, "list:QEU_C_ScanConfig").unwrap();
    let roam = table(&ws, "list:QEU_C_RoamConfig").unwrap();
    let rx = table(&ws, "list:QEU_C_RxGrpAlias").unwrap();
    let original = ws.files().clone();

    ws.set_value(&field(&scan, 0, "Scan List Alias"), "Scan Home").unwrap();
    ws.set_value(&field(&scan, 0, "Hang Time"), "4").unwrap();
    ws.set_value(&field(&roam, 0, "Roam List Alias"), "Roam Home").unwrap();
    ws.set_value(&field(&rx, 0, "Rx Group List Alias"), "Group RX").unwrap();

    // The new values are in the files to save/write, and only in those files.
    let changed: Vec<u16> = ws.files().iter().filter(|(id, b)| original.get(id) != Some(b)).map(|(id, _)| *id).collect();
    assert_eq!(changed, [39, 81, 85]);
    let scan = table(&ws, "list:QEU_C_ScanConfig").unwrap();
    assert_eq!(texts(&scan, "Scan List Alias"), ["Scan Home"]);
    assert_eq!(texts(&scan, "Hang Time"), ["4"]);
    assert!(scan.rows[0].cells[0].changed);

    let changes = change_list(&ws);
    let places: Vec<(&str, &str, &str, &str)> =
        changes.iter().map(|c| (c.place.as_str(), c.setting.as_str(), c.old.as_str(), c.new.as_str())).collect();
    assert!(places.contains(&("Scan list 1 \"Scan Home\"", "Scan List Alias", "Scan List 1", "Scan Home")), "{places:?}");
    assert!(places.contains(&("Scan list 1 \"Scan Home\"", "Hang Time", "3 s", "4 s")), "{places:?}");
    assert!(places.contains(&("Roam list 1 \"Roam Home\"", "Roam List Alias", "Roam List 1", "Roam Home")), "{places:?}");
    assert!(places.contains(&("RX group list 1 \"Group RX\"", "Rx Group List Alias", "Rx Group List 1", "Group RX")), "{places:?}");
    assert!(changes.iter().all(|c| c.field.is_some()), "every list value change can be reverted");

    // Invalid values are refused and change nothing.
    let before = ws.changes().to_vec();
    for (t, column, text) in [
        (&scan, "Scan List Alias", "a name that is far too long"),
        (&scan, "Scan Type", "Sideways"),
        (&scan, "Hang Time", "1000"),
        (&roam, "RSSI Threshold", "-20"),
    ] {
        assert!(ws.set_value(&field(t, 0, column), text).is_err(), "{column} = {text:?} was accepted");
    }
    assert_eq!(ws.changes(), &before[..]);

    // Members are read-only: no member edit goes through set_value.
    let members_file = ws.view().iter().find(|f| f.name == "QEU_C_ScanList").unwrap();
    let member = FieldRef { file: members_file.id, block: members_file.blocks[0].id, record: 0, member: Some(0), key: "Channel List".into() };
    assert!(ws.set_value(&member, "1").is_err());
    assert_eq!(ws.changes(), &before[..]);

    // Reverting restores the opened folder's bytes.
    for c in change_list(&ws) {
        ws.revert(&c.field.unwrap()).unwrap();
    }
    assert!(!ws.has_changes());
    assert_eq!(ws.files(), &original);
}
