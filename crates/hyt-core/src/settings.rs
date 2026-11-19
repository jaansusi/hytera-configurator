//! The settings view: every single-record setting CPS shows for the radio (radio alias, tones,
//! keys, menu, power, emergency, …), grouped into a short list of topics with plain names, using
//! CPS's screen structure ([`hytera_codeplug::ui::UiTree`]) only as a source of grouping and order.
//!
//! Lists (channels, zones, contacts, scan/roam/RX group lists, …) have their own views and are left
//! out. Fields CPS never shows are left out. Passwords are never included. Some topics and pages
//! are read-only: what the radio reports about itself, developer settings, password/lock settings,
//! and files that CPS never writes.

use std::collections::HashSet;

use hytera_codeplug::decode::keys;
use hytera_codeplug::layout::FieldDef;
use hytera_models::{GENERIC_G2, ModelProfile, PROFILES};
use serde::Serialize;

use crate::tables::{Cell, Column, Editor, cell, column};
use crate::view::is_secret;
use crate::workspace::{FieldRef, Workspace, value_at};

/// One setting as shown.
#[derive(Debug, Clone, Serialize)]
pub struct Setting {
    /// CPS's name for it.
    pub label: String,
    pub column: Column,
    pub cell: Cell,
    /// Why it cannot be changed here, when it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    /// Shown only when a section has several groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub settings: Vec<Setting>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Section {
    pub title: String,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Topic {
    pub id: String,
    pub title: String,
    /// Shown above the topic (e.g. why it is read-only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub sections: Vec<Section>,
}

/// A topic in the list, with counts.
#[derive(Debug, Clone, Serialize)]
pub struct TopicInfo {
    pub id: String,
    pub title: String,
    pub settings: usize,
    pub changed: usize,
    pub read_only: bool,
}

struct PlanSetting {
    at: FieldRef,
    label: String,
    read_only: Option<&'static str>,
}

struct PlanGroup {
    title: String,
    settings: Vec<PlanSetting>,
}

struct PlanSection {
    title: String,
    groups: Vec<PlanGroup>,
}

struct PlanTopic {
    title: &'static str,
    note: Option<&'static str>,
    sections: Vec<PlanSection>,
}

const ABOUT: &str = "About this radio";
const DEVELOPER: &str = "Developer";
const READ_ONLY_ABOUT: &str = "Reported by the radio; not editable.";
const READ_ONLY_DEVELOPER: &str = "Developer settings, shown for reference only.";
const READ_ONLY_PASSWORD: &str = "Passwords and programming locks are not changed by this app.";
const READ_ONLY_DEVICE: &str = "Set by the radio.";
const READ_ONLY_FILE: &str = "CPS never writes this file.";

/// Our topic for a CPS category path (root first).
fn topic_for(path: &[&str]) -> Option<&'static str> {
    let second = path.get(1).copied().unwrap_or("");
    Some(match (path.first().copied()?, second) {
        ("Radio Information" | "Feature Control", _) => ABOUT,
        ("Developer", _) | ("Common", "Developer") => DEVELOPER,
        ("Narrowband Setting", _) => "Narrowband",
        ("Common", "Basic Setting") => "General",
        ("Common", "Audio Setting") => "Audio",
        ("Common", "Key") => "Keys",
        ("Common", "Power and Performance") => "Power",
        ("Common", "UI Indication") => "Display and tones",
        ("Common", "Menu") => "Menu",
        ("Common", "Service Setting") => "Services",
        ("Common", "Locaion Service" | "Location Service") => "Location",
        ("Common", "Security") => "Safety",
        ("Conventional", "Basic Setting") => "Conventional",
        ("Conventional", "Digital Setting") => "Digital",
        ("Conventional", "Analog Setting") => "Analog",
        ("Conventional", "Emergency") => "Emergency",
        ("Conventional", "Scan" | "Roam") => "Scan and roam",
        ("Common", _) => "Other",
        ("Conventional", _) => "Conventional",
        _ => "Other",
    })
}

/// Display order of topics.
const ORDER: &[&str] = &[
    "General",
    "Display and tones",
    "Audio",
    "Keys",
    "Menu",
    "Power",
    "Services",
    "Location",
    "Safety",
    "Conventional",
    "Digital",
    "Analog",
    "Scan and roam",
    "Emergency",
    "Narrowband",
    "Other",
    ABOUT,
    DEVELOPER,
];

fn slug(title: &str) -> String {
    title.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

pub(crate) fn profile(ws: &Workspace) -> &'static ModelProfile {
    PROFILES.iter().copied().find(|p| p.key == ws.manifest().profile).unwrap_or(&GENERIC_G2)
}

/// Where each shown setting lives, built from CPS's screen structure and the layout.
fn plan(ws: &Workspace) -> Vec<PlanTopic> {
    let Some(ui) = &ws.ui else { return Vec::new() };
    let excluded = profile(ws).write_excluded_files;
    let mut seen: HashSet<(u16, u16, String)> = HashSet::new();
    let mut topics: Vec<PlanTopic> = Vec::new();
    for (path, page) in ui.pages() {
        let Some(title) = topic_for(&path) else { continue };
        let page_read_only = match page.name.as_str() {
            "Password" => Some(READ_ONLY_PASSWORD),
            "Device Info" => Some(READ_ONLY_DEVICE),
            _ => None,
        };
        let topic_read_only = match title {
            ABOUT => Some(READ_ONLY_ABOUT),
            DEVELOPER => Some(READ_ONLY_DEVELOPER),
            _ => None,
        };
        let mut groups = Vec::new();
        for g in &page.groups {
            let mut settings = Vec::new();
            for &id in &g.params {
                for def in ws.layout.find(id) {
                    if def.block_type != 0 || !ws.base().iter().any(|f| f.id == def.file_id) {
                        continue;
                    }
                    let Some(key) = key_of(ws, def) else { continue };
                    if is_secret(&key) || key.starts_with("Reserved") || !seen.insert((def.file_id, def.block_id, key.clone())) {
                        continue;
                    }
                    let at = FieldRef { file: def.file_id, block: def.block_id, record: 0, member: None, key: key.clone() };
                    if value_at(ws.view(), &at).is_none() {
                        continue;
                    }
                    let label = ws.options.as_ref().and_then(|o| o.param(def.id)).map_or(key.clone(), |p| p.name.clone());
                    let read_only = topic_read_only
                        .or(page_read_only)
                        .or_else(|| excluded.contains(&def.file_id).then_some(READ_ONLY_FILE));
                    settings.push(PlanSetting { at, label, read_only });
                }
            }
            if !settings.is_empty() {
                groups.push(PlanGroup { title: g.name.clone(), settings });
            }
        }
        if groups.is_empty() {
            continue;
        }
        let section = PlanSection { title: page.name.clone(), groups };
        match topics.iter_mut().find(|t| t.title == title) {
            Some(t) => t.sections.push(section),
            None => topics.push(PlanTopic { title, note: topic_read_only, sections: vec![section] }),
        }
    }
    topics.sort_by_key(|t| ORDER.iter().position(|o| *o == t.title).unwrap_or(ORDER.len()));
    topics
}

/// The decoded key of `def` (names are made unique within a block, see [`keys`]).
fn key_of(ws: &Workspace, def: &FieldDef) -> Option<String> {
    let defs = ws.layout.fields(def.file_id, def.block_id);
    keys(defs).into_iter().zip(defs).find(|(_, d)| d.id == def.id && d.offset == def.offset).map(|(k, _)| k)
}

fn render(ws: &Workspace, s: &PlanSetting) -> Setting {
    let mut column = column(ws, s.at.file, s.at.block, &s.at.key, &s.label);
    let mut cell = cell(ws, s.at.clone(), true);
    if s.read_only.is_some() {
        cell.editable = false;
        column.editor = Editor::ReadOnly;
    }
    Setting { label: s.label.clone(), column, cell, read_only: s.read_only.map(str::to_string) }
}

fn render_topic(ws: &Workspace, t: &PlanTopic, keep: &dyn Fn(&PlanTopic, &PlanSection, &PlanGroup, &PlanSetting) -> bool) -> Topic {
    let sections = t
        .sections
        .iter()
        .filter_map(|sec| {
            let several = sec.groups.len() > 1;
            let groups: Vec<Group> = sec
                .groups
                .iter()
                .filter_map(|g| {
                    let settings: Vec<Setting> = g.settings.iter().filter(|s| keep(t, sec, g, s)).map(|s| render(ws, s)).collect();
                    (!settings.is_empty()).then(|| Group {
                        title: (several && g.title != sec.title).then(|| g.title.clone()),
                        settings,
                    })
                })
                .collect();
            (!groups.is_empty()).then(|| Section { title: sec.title.clone(), groups })
        })
        .collect();
    Topic { id: slug(t.title), title: t.title.to_string(), note: t.note.map(str::to_string), sections }
}

/// The topics, with how many settings each has and how many are changed.
pub fn index(ws: &Workspace) -> Vec<TopicInfo> {
    plan(ws)
        .iter()
        .map(|t| {
            let all: Vec<&PlanSetting> = t.sections.iter().flat_map(|s| &s.groups).flat_map(|g| &g.settings).collect();
            TopicInfo {
                id: slug(t.title),
                title: t.title.to_string(),
                settings: all.len(),
                changed: all.iter().filter(|s| ws.original_value(&s.at) != value_at(ws.view(), &s.at)).count(),
                read_only: t.note.is_some(),
            }
        })
        .collect()
}

/// One topic with all its settings.
pub fn topic(ws: &Workspace, id: &str) -> Option<Topic> {
    plan(ws).iter().find(|t| slug(t.title) == id).map(|t| render_topic(ws, t, &|_, _, _, _| true))
}

/// Settings whose name (or section, group or topic) contains every word of `query`.
pub fn search(ws: &Workspace, query: &str) -> Vec<Topic> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let keep = |t: &PlanTopic, sec: &PlanSection, g: &PlanGroup, s: &PlanSetting| {
        let hay = format!("{} {} {} {} {}", t.title, sec.title, g.title, s.label, s.at.key).to_lowercase();
        words.iter().all(|w| hay.contains(w.as_str()))
    };
    plan(ws).iter().map(|t| render_topic(ws, t, &keep)).filter(|t| !t.sections.is_empty()).collect()
}

/// `Settings › Topic › Section` for a setting shown here (for the change list).
pub fn place_of(ws: &Workspace, file: u16, block: u16, key: &str) -> Option<String> {
    plan(ws).iter().find_map(|t| {
        t.sections.iter().find_map(|sec| {
            sec.groups
                .iter()
                .flat_map(|g| &g.settings)
                .any(|s| s.at.file == file && s.at.block == block && s.at.key == key)
                .then(|| format!("Settings › {} › {}", t.title, sec.title))
        })
    })
}

/// The label CPS uses for a setting shown here.
pub fn label_of(ws: &Workspace, file: u16, block: u16, key: &str) -> Option<String> {
    plan(ws).iter().flat_map(|t| &t.sections).flat_map(|s| &s.groups).flat_map(|g| &g.settings).find_map(|s| {
        (s.at.file == file && s.at.block == block && s.at.key == key).then(|| s.label.clone())
    })
}
