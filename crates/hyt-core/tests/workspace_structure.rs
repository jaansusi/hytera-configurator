//! GUI milestone 2 in the workspace: adding and removing channels, zone member actions, and
//! references shown by name. The add and delete operations are checked against CPS's own
//! recordings (captures/add-channel-*, captures/delete-channel-*). Skipped when the private
//! captures, the CPS install or the option catalogue are missing.

use std::path::{Path, PathBuf};

use hyt_core::tables::{Editor, change_list, channel_options, record_detail, table};
use hyt_core::workspace::{ChannelKind, FieldRef, Workspace};
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn open(folder: &str) -> Option<Workspace> {
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let folder = root().join(folder);
    if !dir.is_dir() || !folder.join("manifest.json").is_file() || !Options::default_path("V1.5.24.000")?.is_file() {
        eprintln!("skipping: captures, CPS RCDB data or option catalogue not present");
        return None;
    }
    Some(Workspace::open(&folder, &RcdbRoot::new(dir), None).unwrap())
}

/// The radio's configuration on 2026-11-19 before the GUI rename (D: HYT Test 1, APTLK_CH2;
/// A: 446.13125, 446.18125; Zone 1 = D1, A1, A2; scan list 1's designated channel is D1).
const CURRENT: &str = "captures/remove-hyt-test-4_2026-11-19/after";

fn cps_file(folder: &str, name: &str) -> Vec<u8> {
    std::fs::read(root().join(folder).join(name)).unwrap()
}

fn column(t: &hyt_core::tables::Table, key: &str) -> Vec<String> {
    let i = t.columns.iter().position(|c| c.key == key).unwrap();
    t.rows.iter().map(|r| r.cells[i].text.clone()).collect()
}

fn alias_ref(ws: &Workspace, kind: &str, row: usize) -> FieldRef {
    let t = table(ws, kind).unwrap();
    let i = t.columns.iter().position(|c| c.key == "Channel Alias").unwrap();
    t.rows[row].cells[i].field.clone().unwrap()
}

fn members(ws: &Workspace) -> Vec<String> {
    table(ws, "zones").unwrap().rows[0].members.as_ref().unwrap().iter().map(|m| format!("{} {}", m.kind, m.alias)).collect()
}

#[test]
fn adding_a_channel_and_zone_member_reproduces_cps() {
    let Some(mut ws) = open("captures/add-channel-read_2026-11-18_files") else { return };
    let position = ws.add_channel(ChannelKind::Digital).unwrap();
    assert_eq!(position, 2);
    ws.add_zone_member(0, ChannelKind::Digital, 3).unwrap();
    let cps = "captures/add-channel-write_2026-11-18_files";
    assert!(ws.files()[&3] == cps_file(cps, "003_QEU_C_DChannel.bin"), "digital channel file differs from CPS's");
    assert!(ws.files()[&6] == cps_file(cps, "006_QEU_C_Zone.bin"), "zone file differs from CPS's");
    let t = table(&ws, "digital").unwrap();
    assert!(t.rows[2].added && !t.rows[1].added);
    assert_eq!(column(&t, "Channel Alias")[2], "CH D1", "new channels start from CPS's template");
    assert!(members(&ws).last().unwrap().ends_with("CH D1"));
}

#[test]
fn removing_a_channel_reproduces_cps() {
    let Some(mut ws) = open("captures/delete-channel-read_2026-11-18_files") else { return };
    assert_eq!(column(&table(&ws, "digital").unwrap(), "Channel Alias")[2], "CH D1");
    ws.remove_channel(ChannelKind::Digital, 2).unwrap();
    let cps = "captures/delete-channel-write_2026-11-18_files";
    assert!(ws.files()[&3] == cps_file(cps, "003_QEU_C_DChannel.bin"), "digital channel file differs from CPS's");
    assert!(ws.files()[&6] == cps_file(cps, "006_QEU_C_Zone.bin"), "zone file differs from CPS's");
    // The zone lists the remaining channels by name (the old channel 4 is now 3).
    assert_eq!(members(&ws).last().unwrap(), "Digital HYT Test 4");
    let changes = change_list(&ws);
    assert!(changes.iter().any(|c| c.setting == "Removed" && c.place.contains("\"CH D1\"")), "{changes:?}");
}

#[test]
fn added_then_removed_channels_renumber_like_a_single_edit() {
    let Some(mut a) = open(CURRENT) else { return };
    let Some(mut b) = open(CURRENT) else { return };
    // A: add two channels, name them, put both in Zone 1, then remove the first one again.
    a.add_channel(ChannelKind::Digital).unwrap();
    a.add_channel(ChannelKind::Digital).unwrap();
    a.set_value(&alias_ref(&a, "digital", 2), "Temp").unwrap();
    a.set_value(&alias_ref(&a, "digital", 3), "Keep").unwrap();
    a.add_zone_member(0, ChannelKind::Digital, 3).unwrap();
    a.add_zone_member(0, ChannelKind::Digital, 4).unwrap();
    assert_eq!(members(&a)[3..], ["Digital Temp", "Digital Keep"]);
    a.remove_channel(ChannelKind::Digital, 2).unwrap();
    assert_eq!(column(&table(&a, "digital").unwrap(), "Channel Alias"), ["HYT Test 1", "APTLK_CH2", "Keep"]);
    assert_eq!(members(&a), ["Digital HYT Test 1", "Analog 446.13125", "Analog 446.18125", "Digital Keep"]);
    // B: the same result in one go.
    b.add_channel(ChannelKind::Digital).unwrap();
    b.set_value(&alias_ref(&b, "digital", 2), "Keep").unwrap();
    b.add_zone_member(0, ChannelKind::Digital, 3).unwrap();
    assert_eq!(a.files(), b.files());
    // Removing the last added channel leaves nothing to write.
    a.remove_channel(ChannelKind::Digital, 2).unwrap();
    assert!(!a.has_changes(), "{:?}", a.changes());
}

#[test]
fn removing_existing_channels_updates_zones_or_is_refused() {
    let Some(mut ws) = open(CURRENT) else { return };
    // Digital 1 is scan list 1's designated channel: CPS's handling is unknown, so refused.
    let err = ws.remove_channel(ChannelKind::Digital, 0).unwrap_err();
    assert!(format!("{err:#}").contains("designated channel"), "{err:#}");
    assert!(!ws.has_changes());
    // Analog 1 is the radio's current channel (QEU_C_Config): refused too.
    let err = ws.remove_channel(ChannelKind::Analog, 0).unwrap_err();
    assert!(format!("{err:#}").contains("current channel"), "{err:#}");
    // Analog 2 is in Zone 1: the member goes with it.
    ws.remove_channel(ChannelKind::Analog, 1).unwrap();
    assert_eq!(members(&ws), ["Digital HYT Test 1", "Analog 446.13125"]);
    assert_eq!(ws.channel_count(ChannelKind::Analog), 1);
    let changes = change_list(&ws);
    assert!(changes.iter().any(|c| c.place == "Analog channel 2 \"446.18125\"" && c.setting == "Removed" && c.old == "\"446.18125\""), "{changes:#?}");
    assert!(changes.iter().any(|c| c.setting == "Channel removed" && c.old == "Analog 2 \"446.18125\""), "{changes:#?}");
    assert!(ws.revert(&alias_ref(&ws, "analog", 0)).is_ok(), "reverting an unchanged field is a no-op");
    ws.discard();
    assert!(!ws.has_changes());
}

#[test]
fn zone_members_can_be_added_moved_and_removed() {
    let Some(mut ws) = open(CURRENT) else { return };
    let options = channel_options(&ws);
    assert_eq!(options.len(), 4);
    assert_eq!(options[1].label, "Digital 2 \"APTLK_CH2\"");
    ws.add_zone_member(0, ChannelKind::Digital, 2).unwrap();
    assert!(ws.add_zone_member(0, ChannelKind::Digital, 2).is_err(), "no duplicates");
    assert!(ws.add_zone_member(0, ChannelKind::Analog, 3).is_err(), "no such channel");
    assert_eq!(members(&ws).last().unwrap(), "Digital APTLK_CH2");
    ws.move_zone_member(0, 3, true).unwrap();
    assert_eq!(members(&ws), ["Digital HYT Test 1", "Analog 446.13125", "Digital APTLK_CH2", "Analog 446.18125"]);
    assert!(ws.move_zone_member(0, 0, true).is_err());
    let changes = change_list(&ws);
    assert!(changes.iter().any(|c| c.setting == "Channel" && c.old == "Analog 2 \"446.18125\"" && c.new == "Digital 2 \"APTLK_CH2\""), "{changes:#?}");
    let settings: Vec<&str> = changes.iter().map(|c| c.setting.as_str()).collect();
    assert_eq!(settings, ["Channel", "Channel added"], "one line per member position");
    assert!(table(&ws, "zones").unwrap().rows[0].cells.last().unwrap().changed);
    // Moving back and removing the added member restores the folder exactly.
    ws.move_zone_member(0, 2, false).unwrap();
    ws.remove_zone_member(0, 3).unwrap();
    assert!(!ws.has_changes(), "{:?}", ws.changes());
    // Removing an original member.
    ws.remove_zone_member(0, 0).unwrap();
    assert_eq!(members(&ws), ["Analog 446.13125", "Analog 446.18125"]);
}

#[test]
fn references_are_shown_and_edited_by_name() {
    let Some(mut ws) = open(CURRENT) else { return };
    let digital = table(&ws, "digital").unwrap();
    assert_eq!(column(&digital, "RX Group List"), ["Rx Group List 1", "Rx Group List 1"]);
    // TX contact 10 is a call ID no contact has: shown as the number.
    assert_eq!(column(&digital, "TX Contacts ID"), ["10", "10"]);
    let i = digital.columns.iter().position(|c| c.key == "TX Contacts ID").unwrap();
    let cell = &digital.rows[0].cells[i];
    assert_eq!(cell.editor, Some(Editor::Choice { options: vec!["Call 1".into()] }));
    ws.set_value(cell.field.as_ref().unwrap(), "Call 1").unwrap();
    let digital = table(&ws, "digital").unwrap();
    assert_eq!(column(&digital, "TX Contacts ID"), ["Call 1", "10"]);
    let c = change_list(&ws);
    assert_eq!((c[0].old.as_str(), c[0].new.as_str()), ("10", "Call 1"));

    assert_eq!(column(&table(&ws, "analog").unwrap(), "Scan List"), ["None", "None"]);
    let a = &table(&ws, "analog").unwrap().rows[0].record;
    let detail = record_detail(&ws, a);
    let emergency = detail.iter().find(|d| d.column.key == "Emergency System").unwrap();
    assert_eq!(emergency.cell.text, "AnalogSys 1");
    // Scan/roam list follows "Use Scan or Roam List" (None here).
    let d = record_detail(&ws, &table(&ws, "digital").unwrap().rows[0].record);
    assert_eq!(d.iter().find(|d| d.column.key == "Scan List Roam List").unwrap().cell.text, "None");
}
