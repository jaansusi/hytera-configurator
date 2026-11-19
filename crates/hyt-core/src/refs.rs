//! Fields that refer to other records, shown and chosen by the referenced record's name.
//!
//! Observed 2026-11-20: which fields are references, what they point to, how the value encodes the
//! record (1-based position, a list position, a call ID) and the special values (`0 = None`,
//! `0 = Selected` channel, `65535 = None`, …) come from CPS's own configuration
//! ([`hytera_codeplug::validation`]: `DataValidationConfig.config`, `ComplexRuleConfig.Config`),
//! loaded at run time. When those files are missing, a small built-in table of the references
//! hyt knew before is used (inferred 2026-11-19).
//!
//! One reference is not in CPS's configuration and stays built in (inferred): a digital channel's
//! `Scan List Roam List` points into the scan lists or the roam lists depending on its
//! `Use Scan or Roam List` (0 None, 1 scan, 2 roam), 1-based, 0 = None.

use hytera_codeplug::decode::{DecodedFile, keys};
use hytera_codeplug::layout::{FieldType, Layout};
use hytera_codeplug::rcdb::TableLink;
use hytera_codeplug::validation::{Locate, ParamReference, Show, ValidationConfig};
use serde_json::{Map, Value};

use crate::workspace::FieldRef;

/// Member `Channel List` value meaning "the selected channel" (observed in scan/roam members).
const SELECTED_MEMBER: i64 = 65535;

const CONTACTS_FILE: &str = "QEU_C_DContact";

/// Why a scan list member cannot be chosen as priority channel (shown when someone tries).
const NOTE_PRIORITY: &str = "choosing a scan list member as priority channel is not supported yet: CPS also \
                             stores the channel's type in the companion field (PriCH ChType), and CPS has not \
                             been recorded doing it";

/// What a reference field points to.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    /// Contacts, by call ID; `group_only` offers only group-call contacts.
    Contact { group_only: bool },
    /// Records of `file`/`block`: the value `start` is the first record. Named by field `name`;
    /// only records whose field `filter.0` holds one of `filter.1` are offered.
    Rows { file: u16, block: u16, name: String, start: i64, filter: Option<(String, Vec<i64>)> },
    /// Member lists of the two-level `file`, named by field `names.1` of the companion file `names.0`.
    Lists { file: u16, names: Option<(u16, String)>, start: i64 },
    /// Members of the record's own member list (same position in the two-level `file`), named
    /// `<channel> (member <n>)`.
    Members { file: u16, start: i64, channel: MemberChannel },
    /// Scan or roam list, chosen by the record's `Use Scan or Roam List`.
    ScanOrRoam { scan: Box<Target>, roam: Box<Target> },
}

/// How a member names its channel (a virtual parameter of `ComplexRuleConfig.Config`).
#[derive(Debug, Clone, PartialEq)]
struct MemberChannel {
    /// Member field with the channel number (1-based; 65535 = the selected channel).
    index: String,
    /// Member field with the channel type, when there is one.
    kind: Option<String>,
    /// (type value, channel file, alias field) of each related list.
    lists: Vec<(Option<i64>, u16, String)>,
}

#[derive(Debug, Clone, PartialEq)]
struct Spec {
    file: u16,
    block: u16,
    key: String,
    target: Target,
    /// Special values and their names.
    special: Vec<(i64, String)>,
    /// Why records (other than the special values) are shown but cannot be chosen, if so.
    no_pick: Option<&'static str>,
}

/// Where the reference definitions came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// CPS's `DataValidationConfig.config` (+ `ComplexRuleConfig.Config`).
    CpsConfig,
    /// hyt's built-in table (CPS's configuration was not found).
    BuiltIn,
}

/// The reference fields of a configuration.
#[derive(Debug, Clone)]
pub struct References {
    specs: Vec<Spec>,
    pub source: Source,
}

/// English text for CPS's UI keys of special values (from CPS's `en-US.xaml`; `CurrentChanel` is
/// "Selected Channel" there and "Selected" in hyt, as for scan/roam members).
fn special_name(key: &str) -> String {
    match key {
        "CurrentChanel" => "Selected".into(),
        "DefaultOption" => "Default".into(),
        other => other.into(),
    }
}

/// Raw value for a filter key (observed: bit fields shown as Off/Close vs On/OPEN; call type
/// group = 1). `None` for keys hyt does not know (then the filter is not applied).
fn filter_value(key: &str) -> Option<i64> {
    match key {
        "Off" | "Close" => Some(0),
        "On" | "OPEN" | "Open" => Some(1),
        "GroupCall" => Some(1),
        _ => None,
    }
}

fn key_of(layout: &Layout, file: u16, block: u16, param: i64) -> Option<String> {
    let defs = layout.fields(file, block);
    keys(defs).into_iter().zip(defs).find(|(_, d)| i64::from(d.id) == param).map(|(k, _)| k)
}

fn file_id(files: &[DecodedFile], name: &str) -> Option<u16> {
    files.iter().find(|f| f.name == name).map(|f| f.id)
}

/// The first text field of the first block of `file` (a list's alias).
fn alias_key(layout: &Layout, file: u16) -> Option<String> {
    let block = *layout.block_ids(file).first()?;
    let defs = layout.fields(file, block);
    keys(defs)
        .into_iter()
        .zip(defs)
        .find(|(_, d)| matches!(d.kind, FieldType::UnicodeString | FieldType::AsciiString))
        .map(|(k, _)| k)
}

/// Member lists of two-level `file` named by its companion one-level file (`TableLink.xml`).
fn lists_target(layout: &Layout, links: &[TableLink], file: u16, start: i64) -> Target {
    let names = links.iter().find(|l| l.two_dim == file).and_then(|l| Some((l.one_dim, alias_key(layout, l.one_dim)?)));
    Target::Lists { file, names, start }
}

impl References {
    /// The references of `files` (decoded with `layout`) from CPS's `config`, or hyt's built-in
    /// table when `config` is `None`. `links` pairs two-level files with their name files.
    pub fn new(config: Option<&ValidationConfig>, layout: &Layout, links: &[TableLink], files: &[DecodedFile]) -> Self {
        let mut specs = match config {
            Some(c) => from_config(c, layout, links, files),
            None => built_in(layout, links, files),
        };
        // Not in CPS's configuration (inferred): the digital channel's scan/roam list.
        if let (Some(dch), Some(scan), Some(roam)) =
            (file_id(files, "QEU_C_DChannel"), file_id(files, "QEU_C_ScanList"), file_id(files, "QEU_C_RoamList"))
            && let Some(block) = layout.block_ids(dch).first().copied()
            && !specs.iter().any(|s| s.file == dch && s.key == "Scan List Roam List")
            && key_of(layout, dch, block, 19).as_deref() == Some("Scan List Roam List")
        {
            specs.push(Spec {
                file: dch,
                block,
                key: "Scan List Roam List".into(),
                target: Target::ScanOrRoam {
                    scan: Box::new(lists_target(layout, links, scan, 1)),
                    roam: Box::new(lists_target(layout, links, roam, 1)),
                },
                special: vec![(0, "None".into())],
                no_pick: None,
            });
        }
        let source = if config.is_some() { Source::CpsConfig } else { Source::BuiltIn };
        References { specs, source }
    }

    fn spec(&self, file: u16, block: u16, key: &str) -> Option<&Spec> {
        self.specs.iter().find(|s| s.file == file && s.block == block && s.key == key)
    }

    /// Whether field `key` of `file`/`block` refers to other records.
    pub fn is_reference(&self, file: u16, block: u16, key: &str) -> bool {
        self.spec(file, block, key).is_some()
    }

    /// Every reference field as (file, block, key).
    pub fn fields(&self) -> impl Iterator<Item = (u16, u16, &str)> {
        self.specs.iter().map(|s| (s.file, s.block, s.key.as_str()))
    }

    /// Every value the field at `at` (in a record with fields `record`) can name, with the names
    /// to show (special values first), from the configuration `files`. `None` when the field is
    /// not a reference.
    pub fn names(&self, files: &[DecodedFile], at: &FieldRef, record: &Map<String, Value>) -> Option<Vec<(i64, String)>> {
        let spec = self.spec(at.file, at.block, &at.key)?;
        Some(unique(spec.special.iter().cloned().chain(entries(files, &spec.target, at, record, false)).collect()))
    }

    /// The values that can be chosen for the field at `at` (as [`References::names`], without
    /// records CPS filters out or hyt cannot set yet).
    pub fn choices(&self, files: &[DecodedFile], at: &FieldRef, record: &Map<String, Value>) -> Option<Vec<(i64, String)>> {
        let spec = self.spec(at.file, at.block, &at.key)?;
        let names = self.names(files, at, record)?;
        if spec.no_pick.is_some() {
            return Some(names.into_iter().filter(|(raw, _)| spec.special.iter().any(|(s, _)| s == raw)).collect());
        }
        let offered: Vec<i64> = entries(files, &spec.target, at, record, true).into_iter().map(|(raw, _)| raw).collect();
        Some(names.into_iter().filter(|(raw, _)| spec.special.iter().any(|(s, _)| s == raw) || offered.contains(raw)).collect())
    }

    /// Why `raw` (a name [`References::names`] lists) cannot be chosen for the field at `at`.
    pub fn why_not(&self, at: &FieldRef) -> String {
        match self.spec(at.file, at.block, &at.key).and_then(|s| s.no_pick) {
            Some(note) => note.to_string(),
            None => "CPS does not offer it for this field".into(),
        }
    }
}

/// References from CPS's configuration that this configuration's files have.
fn from_config(config: &ValidationConfig, layout: &Layout, links: &[TableLink], files: &[DecodedFile]) -> Vec<Spec> {
    let present = |file: u16| files.iter().any(|f| f.id == file);
    let mut specs: Vec<Spec> = Vec::new();
    for r in &config.references {
        let defs = layout.find(r.param);
        let defs: Vec<_> = defs.into_iter().filter(|d| present(d.file_id) && d.block_type != 2).collect();
        let Some(def) = defs.iter().find(|d| Some(d.block_id) == r.block).or(defs.first()) else { continue };
        let Some(key) = key_of(layout, def.file_id, def.block_id, i64::from(def.id)) else { continue };
        if specs.iter().any(|s| s.file == def.file_id && s.block == def.block_id && s.key == key) {
            continue;
        }
        let Some(target) = target_of(r, def.file_id, config, layout, links) else { continue };
        let no_pick = matches!(target, Target::Members { .. }).then_some(NOTE_PRIORITY);
        specs.push(Spec {
            file: def.file_id,
            block: def.block_id,
            key,
            target,
            special: r.special.iter().map(|(v, k)| (*v, special_name(k))).collect(),
            no_pick,
        });
    }
    // Call IDs (dial rules) of conventional records outside the contact list and member lists.
    let contacts = file_id(files, CONTACTS_FILE);
    for d in config.dial_params.iter().filter(|d| d.product == "Conventional") {
        let Some(block) = d.block else { continue };
        let Some((file, defs)) = layout.block(block) else { continue };
        if !present(file) || Some(file) == contacts || contacts.is_none() || defs.first().is_some_and(|f| f.block_type == 2) {
            continue;
        }
        let Some(key) = key_of(layout, file, block, i64::from(d.param)) else { continue };
        if specs.iter().any(|s| s.file == file && s.block == block && s.key == key) {
            continue;
        }
        let group_only = d.call_type.as_deref() == Some("Group");
        specs.push(Spec { file, block, key, target: Target::Contact { group_only }, special: Vec::new(), no_pick: None });
    }
    specs
}

/// The target of CPS reference `r` for a field in `file`, if hyt supports its kind.
fn target_of(r: &ParamReference, file: u16, config: &ValidationConfig, layout: &Layout, links: &[TableLink]) -> Option<Target> {
    let (source_file, source_defs) = layout.block(r.source_block)?;
    let two_level = source_defs.first().is_some_and(|d| d.block_type == 2);
    let start = match r.locate {
        Locate::RowIndex { start } | Locate::TableIndex { start } => start,
        _ => return None,
    };
    match (&r.show, two_level) {
        (Show::TableName, true) => Some(lists_target(layout, links, source_file, start)),
        (Show::ParamValue { link_index }, false) => {
            let name = key_of(layout, source_file, r.source_block, r.source_param(*link_index)?.param)?;
            let mut filter = None;
            for (param, f) in r.filters() {
                let values: Option<Vec<i64>> = f.keys.iter().map(|k| filter_value(k)).collect();
                match (key_of(layout, source_file, r.source_block, param), values) {
                    (Some(key), Some(values)) if f.include => filter = Some((key, values)),
                    // A filter hyt cannot read: offer every record rather than guess.
                    _ => {}
                }
            }
            Some(Target::Rows { file: source_file, block: r.source_block, name, start, filter })
        }
        (Show::ParamValue { link_index }, true) => {
            // Members of the record's own list: the field must be in the list's name file.
            let virtual_param = config.virtual_param(r.source_param(*link_index)?.param)?;
            let partner = links.iter().any(|l| l.two_dim == source_file && l.one_dim == file) || source_file == file;
            if !partner || virtual_param.block != r.source_block {
                return None;
            }
            let index = key_of(layout, source_file, r.source_block, i64::from(virtual_param.index_param?))?;
            let kind = virtual_param.type_param.and_then(|p| key_of(layout, source_file, r.source_block, i64::from(p)));
            let lists = virtual_param
                .related
                .iter()
                .filter_map(|b| {
                    let (f, _) = layout.block(b.block)?;
                    Some((b.type_value, f, key_of(layout, f, b.block, i64::from(b.alias_param?))?))
                })
                .collect();
            Some(Target::Members { file: source_file, start, channel: MemberChannel { index, kind, lists } })
        }
        _ => None,
    }
}

/// hyt's table from before CPS's configuration was read (inferred 2026-11-19), by file name.
fn built_in(layout: &Layout, links: &[TableLink], files: &[DecodedFile]) -> Vec<Spec> {
    let mut specs = Vec::new();
    let mut add = |file: &str, key: &str, target: Option<Target>, special: Vec<(i64, String)>| {
        let (Some(id), Some(target)) = (file_id(files, file), target) else { return };
        let Some(block) = layout.block_ids(id).first().copied() else { return };
        specs.push(Spec { file: id, block, key: key.into(), target, special, no_pick: None });
    };
    let none = || vec![(0, "None".to_string())];
    let lists = |members: &str| file_id(files, members).map(|f| lists_target(layout, links, f, 1));
    let rows = |file: &str, name: &str| {
        let id = file_id(files, file)?;
        Some(Target::Rows { file: id, block: *layout.block_ids(id).first()?, name: name.into(), start: 1, filter: None })
    };
    add("QEU_C_DChannel", "TX Contacts ID", Some(Target::Contact { group_only: false }), Vec::new());
    add("QEU_C_DChannel", "RX Group List", lists("QEU_C_RxGrpList"), none());
    add("QEU_C_AChannel", "Scan List", lists("QEU_C_ScanList"), none());
    add("QEU_C_AChannel", "Emergency System", rows("QEU_C_AEmergency", "Emergency System Alias"), none());
    specs
}

fn block_records(files: &[DecodedFile], file: u16, block: Option<u16>) -> &[hytera_codeplug::decode::Record] {
    files
        .iter()
        .find(|f| f.id == file)
        .and_then(|f| match block {
            Some(b) => f.blocks.iter().find(|x| x.id == b),
            None => f.blocks.first(),
        })
        .map_or(&[], |b| b.records.as_slice())
}

fn text(fields: Option<&Map<String, Value>>, key: &str) -> String {
    fields.and_then(|f| f.get(key)).and_then(Value::as_str).unwrap_or("?").to_string()
}

/// The records `target` can name, as (raw value, name); only the offered ones when `offered`.
fn entries(files: &[DecodedFile], target: &Target, at: &FieldRef, record: &Map<String, Value>, offered: bool) -> Vec<(i64, String)> {
    match target {
        Target::Contact { group_only } => block_records(files, file_id(files, CONTACTS_FILE).unwrap_or(u16::MAX), None)
            .iter()
            .filter_map(|r| r.fields.as_ref().map(|f| &f.0))
            .filter(|c| !(offered && *group_only) || c.get("Call Type").and_then(Value::as_i64) == Some(1))
            .filter_map(|c| Some((c.get("Call ID")?.as_i64()?, c.get("Call Alias")?.as_str()?.to_string())))
            .collect(),
        Target::Rows { file, block, name, start, filter } => block_records(files, *file, Some(*block))
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let Some((key, values)) = filter.as_ref().filter(|_| offered) else { return true };
                r.fields.as_ref().and_then(|f| f.0.get(key)).and_then(Value::as_i64).is_some_and(|v| values.contains(&v))
            })
            .map(|(i, r)| (start + i as i64, text(r.fields.as_ref().map(|f| &f.0), name)))
            .collect(),
        Target::Lists { file, names, start } => {
            let lists = block_records(files, *file, None).len();
            let named = names.as_ref().map(|(f, _)| block_records(files, *f, None));
            let count = named.map_or(lists, |n| n.len().min(lists));
            (0..count)
                .map(|i| {
                    let name = match (names, named) {
                        (Some((_, key)), Some(n)) => text(n[i].fields.as_ref().map(|f| &f.0), key),
                        _ => format!("List {}", i + 1),
                    };
                    (start + i as i64, name)
                })
                .collect()
        }
        Target::Members { file, start, channel } => {
            let Some(members) = block_records(files, *file, None).get(at.record).and_then(|r| r.members.as_ref()) else {
                return Vec::new();
            };
            members
                .iter()
                .enumerate()
                .map(|(i, m)| (start + i as i64, format!("{} (member {})", member_name(files, channel, &m.0), i + 1)))
                .collect()
        }
        Target::ScanOrRoam { scan, roam } => match record.get("Use Scan or Roam List").and_then(Value::as_i64) {
            Some(1) => entries(files, scan, at, record, offered),
            Some(2) => entries(files, roam, at, record, offered),
            _ => Vec::new(),
        },
    }
}

/// The name of the channel a list member holds: its alias, or `Selected`.
fn member_name(files: &[DecodedFile], channel: &MemberChannel, member: &Map<String, Value>) -> String {
    let number = member.get(&channel.index).and_then(Value::as_i64).unwrap_or(0);
    if number == SELECTED_MEMBER {
        return "Selected".into();
    }
    let kind = channel.kind.as_ref().and_then(|k| member.get(k)).and_then(Value::as_i64);
    let list = channel.lists.iter().find(|(t, _, _)| t.is_none() || *t == kind);
    let alias = list.and_then(|(_, file, key)| {
        let r = block_records(files, *file, None).get(usize::try_from(number - 1).ok()?)?;
        r.fields.as_ref()?.0.get(key)?.as_str().map(str::to_string)
    });
    alias.unwrap_or_else(|| format!("(channel {number})"))
}

/// Make labels unique by adding the raw value to repeated ones.
fn unique(mut choices: Vec<(i64, String)>) -> Vec<(i64, String)> {
    let labels: Vec<String> = choices.iter().map(|(_, l)| l.clone()).collect();
    for (raw, label) in &mut choices {
        if labels.iter().filter(|l| *l == label).count() > 1 {
            *label = format!("{label} ({raw})");
        }
    }
    choices
}

/// Name for `raw` among `choices`, or `None` if it matches none (then the number is shown).
pub fn label(choices: &[(i64, String)], raw: i64) -> Option<String> {
    choices.iter().find(|(r, _)| *r == raw).map(|(_, l)| l.clone())
}

/// Raw value for a name shown by [`References::choices`] or [`References::names`].
pub fn parse(choices: &[(i64, String)], text: &str) -> Option<i64> {
    let text = text.trim();
    choices.iter().find(|(_, l)| l == text).map(|(r, _)| *r)
}
