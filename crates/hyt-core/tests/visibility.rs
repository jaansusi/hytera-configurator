//! Fields CPS does not show (licence-limited or internal), checked against CPS's screens for the
//! BP515 on 2026-11-19 (captures/cps-encryption_2026-11-19/). Uses the radio's configuration after
//! 2026-11-19 (captures/remove-hyt-test-4_2026-11-19/after). Skipped when the private captures, the
//! CPS install or the exported screen data are missing.

use std::path::{Path, PathBuf};

use hyt_core::tables::{TABLE_IDS, record_detail, table};
use hyt_core::visibility::HiddenKind;
use hyt_core::workspace::Workspace;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};

fn open() -> Option<Workspace> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../captures/remove-hyt-test-4_2026-11-19/after");
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let ui = hytera_codeplug::ui::UiTree::default_path("V1.5.24.000");
    if !folder.is_dir() || !dir.join("CPSParams.xlsx").is_file() || !ui.is_some_and(|p| p.is_file()) {
        eprintln!("skipping: captures, CPS install or exported screen data not present");
        return None;
    }
    Some(Workspace::open(&folder, &RcdbRoot::new(dir), None).unwrap())
}

/// The fields shown by default (neither licence-limited nor internal).
fn shown(ws: &Workspace, id: &str) -> Vec<String> {
    let t = table(ws, id).unwrap();
    record_detail(ws, &t.rows[0].record).into_iter().filter(|f| f.column.hidden.is_none()).map(|f| f.column.key).collect()
}

#[test]
fn digital_channel_shows_what_cps_shows() {
    let Some(ws) = open() else { return };
    let mut visible = shown(&ws, "digital");
    visible.sort();
    // CPS's dialog for digital channel 1 (digital-channel-1.png), by layout name.
    let mut expected = vec![
        "Channel Alias", "Slot Operation", "RX Only", "Auto Start Scan Roam", "TalkAround", "Color Code",
        "Pseudo Trunk Designated TX", "Use Scan or Roam List", "Scan List Roam List", "RX Frequency", "TX Frequency",
        "Power Level", "TX Timeout Time", "TOT Rekey Time", "Private Call Confirmed", "TX Contacts ID", "TX Admit",
        "TOT Prealert Time", "TOT Reset Time", "Call Type", "RRS Channel", "RX Group List",
    ];
    expected.sort_unstable();
    assert_eq!(visible, expected);

    let t = table(&ws, "digital").unwrap();
    let detail = record_detail(&ws, &t.rows[0].record);
    let kind = |key: &str| detail.iter().find(|f| f.column.key == key).and_then(|f| f.column.hidden.as_ref()).map(|h| h.kind);
    assert_eq!(kind("Encrypt"), Some(HiddenKind::Licence));
    assert_eq!(kind("Encrypt Key"), Some(HiddenKind::Licence));
    assert_eq!(kind("IP Multi Site Connect"), Some(HiddenKind::Licence));
    assert_eq!(kind("Encrypt Type"), Some(HiddenKind::Internal));
    assert_eq!(kind("Channel Type"), Some(HiddenKind::Internal));
    // Hidden by CPS's parameter database, not by a licence (docs/research-cps-visibility.md).
    assert_eq!(kind("Priority Interrupt Encode"), Some(HiddenKind::Internal));
    assert_eq!(kind("Special Frame"), Some(HiddenKind::Internal));
    assert_eq!(kind("Emergency System"), Some(HiddenKind::Internal));
    assert!(detail.iter().filter(|f| f.column.hidden.is_some()).all(|f| !f.cell.editable), "hidden fields are read-only");
}

#[test]
fn nothing_about_encryption_is_shown_like_cps_search() {
    let Some(mut ws) = open() else { return };
    // CPS's search for "encrypt" finds nothing on this radio (search-encrypt.png).
    let found = hyt_core::settings::search(&ws, "encrypt");
    let visible: Vec<&str> = found
        .iter()
        .flat_map(|t| &t.sections)
        .flat_map(|s| &s.groups)
        .flat_map(|g| &g.settings)
        .filter(|s| s.column.hidden.is_none() && s.label.to_lowercase().contains("encrypt"))
        .map(|s| s.label.as_str())
        .collect();
    assert!(visible.is_empty(), "{visible:?}");
    // And such fields cannot be changed.
    let t = table(&ws, "digital").unwrap();
    let encrypt = record_detail(&ws, &t.rows[0].record).into_iter().find(|f| f.column.key == "Encrypt").unwrap();
    let err = ws.set_value(&encrypt.cell.field.unwrap(), "On").unwrap_err();
    assert!(format!("{err:#}").contains("Needs the Basic Encryption licence"), "{err:#}");
}

/// Feature Control shows exactly Two Tone, Pure Analog, Pure Digital and Roam (feature-control.png):
/// the model-number rules make those four visible.
#[test]
fn feature_control_shows_the_four_flags_cps_shows() {
    let Some(ws) = open() else { return };
    let topic = hyt_core::settings::topic(&ws, "about-this-radio").unwrap();
    let section = topic.sections.iter().find(|s| s.title == "Feature Control").expect("Feature Control section");
    for g in &section.groups {
        let mut shown: Vec<&str> = g.settings.iter().filter(|s| s.column.hidden.is_none()).map(|s| s.label.as_str()).collect();
        shown.sort_unstable();
        assert_eq!(shown, ["Pure Analog", "Pure Digital", "Roam", "Two Tone"], "{:?}", g.title);
    }
}

/// Lists CPS hides as a whole while their licence is Off: roam lists (Roam), 2-Tone and HDC1200
/// (Two Tone, category Analog Service).
#[test]
fn lists_of_unlicensed_features_are_marked() {
    let Some(ws) = open() else { return };
    let hidden: Vec<String> = hyt_core::lists::index(&ws).into_iter().filter(|l| l.hidden.is_some()).map(|l| l.id).collect();
    assert_eq!(
        hidden,
        [
            "list:QEU_C_RoamConfig",
            "list:QEU_C_HDC1200_System",
            "list:QEU_C_HDC1200_Contact",
            "list:QEU_C_2Tone_System",
            "list:QEU_C_2Tone_Contact",
            "list:QEU_C_2Tone_Personality"
        ]
    );
}

#[test]
fn main_table_columns_are_all_shown() {
    let Some(ws) = open() else { return };
    for id in TABLE_IDS {
        let t = table(&ws, id).unwrap();
        let hidden: Vec<&str> = t.columns.iter().filter(|c| c.hidden.is_some()).map(|c| c.key.as_str()).collect();
        assert!(hidden.is_empty(), "{id}: {hidden:?}");
    }
    // The list editors show raw values as they are (CPS shows some of them in another form),
    // except lists CPS hides as a whole for a missing licence.
    for l in hyt_core::lists::index(&ws).into_iter().filter(|l| l.hidden.is_none()) {
        let t = table(&ws, &l.id).unwrap();
        let hidden: Vec<&str> = t.columns.iter().filter(|c| c.hidden.is_some()).map(|c| c.key.as_str()).collect();
        assert!(hidden.is_empty(), "{}: {hidden:?}", l.id);
    }
}
