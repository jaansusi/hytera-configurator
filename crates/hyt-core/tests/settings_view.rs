//! The settings view on the radio's configuration (captures/unattended_2026-11-19/start). Skipped
//! when the private captures, the CPS install or the exported option catalogue / ui.json are
//! missing.

use std::path::{Path, PathBuf};

use hyt_core::settings::{index, search, topic};
use hyt_core::tables::change_list;
use hyt_core::workspace::Workspace;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_codeplug::ui::UiTree;

fn open() -> Option<Workspace> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../captures/unattended_2026-11-19/start");
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !dir.is_dir() || !folder.join("manifest.json").is_file() || !UiTree::default_path("V1.5.24.000")?.is_file() {
        eprintln!("skipping: captures, CPS RCDB data or ui.json not present");
        return None;
    }
    let ws = Workspace::open(&folder, &RcdbRoot::new(dir), None).unwrap();
    ws.ui.is_some().then_some(ws)
}

fn find<'a>(t: &'a hyt_core::settings::Topic, label: &str) -> &'a hyt_core::settings::Setting {
    t.sections.iter().flat_map(|s| &s.groups).flat_map(|g| &g.settings).find(|s| s.label == label).unwrap_or_else(|| panic!("no {label}"))
}

#[test]
fn topics_cover_the_radio_settings() {
    let Some(ws) = open() else { return };
    let idx = index(&ws);
    for t in &idx {
        eprintln!("{:<20} {:>4} settings{}", t.title, t.settings, if t.read_only { " (read-only)" } else { "" });
    }
    let titles: Vec<&str> = idx.iter().map(|t| t.title.as_str()).collect();
    for expected in ["General", "Display and tones", "Menu", "Emergency", "About this radio"] {
        assert!(titles.contains(&expected), "{titles:?}");
    }
    assert!(idx.iter().map(|t| t.settings).sum::<usize>() > 400);
    let general = topic(&ws, "general").unwrap();
    let alias = find(&general, "Radio Alias");
    assert_eq!(alias.cell.text, "Radio 1");
    assert!(alias.cell.editable && alias.read_only.is_none());
    // Passwords never appear; the lock flags are read-only.
    let all = serde_json::to_string(&idx.iter().map(|t| topic(&ws, &t.id).unwrap()).collect::<Vec<_>>()).unwrap();
    assert!(!all.contains("Write Password") && !all.contains("Read Possword"));
    let lock = find(&general, "CPS Write Lock");
    assert!(!lock.cell.editable && lock.read_only.is_some());
    let about = topic(&ws, "about-this-radio").unwrap();
    assert!(about.note.is_some());
    assert!(about.sections.iter().flat_map(|s| &s.groups).flat_map(|g| &g.settings).all(|s| !s.cell.editable));
}

#[test]
fn settings_are_edited_and_listed_by_name() {
    let Some(mut ws) = open() else { return };
    let alias = find(&topic(&ws, "general").unwrap(), "Radio Alias").cell.field.clone().unwrap();
    ws.set_value(&alias, "Night Test").unwrap();
    assert!(ws.set_value(&alias, "This alias is far too long for the radio").is_err());
    let general = topic(&ws, "general").unwrap();
    assert!(find(&general, "Radio Alias").cell.changed);
    assert_eq!(index(&ws).iter().find(|t| t.id == "general").unwrap().changed, 1);
    let changes = change_list(&ws);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].place, "Settings › General › Radio Alias");
    assert_eq!((changes[0].setting.as_str(), changes[0].old.as_str(), changes[0].new.as_str()), ("Radio Alias", "Radio 1", "Night Test"));
    // The name match comes first; settings whose CPS description mentions it follow, marked.
    let hits = search(&ws, "radio alias");
    assert_eq!(hits[0].sections[0].groups[0].settings[0].label, "Radio Alias");
    assert_eq!(hits[0].note, None);
    assert!(hits.len() > 1 && hits[1..].iter().all(|t| t.note.as_deref().is_some_and(|n| n.starts_with("Found in CPS's description"))));
    let labels: Vec<&str> =
        hits[1..].iter().flat_map(|t| &t.sections).flat_map(|s| &s.groups).flat_map(|g| &g.settings).map(|s| s.label.as_str()).collect();
    assert!(labels.contains(&"Send Talker Alias"), "{labels:?}");
    assert!(search(&ws, "no such setting anywhere").is_empty());
}
