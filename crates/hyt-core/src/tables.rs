//! Tables of the main settings (digital and analog channels, zones, contacts), every field of one
//! record, and the pending changes, labelled with CPS's option text, for the GUI. Built from the
//! workspace's view (the configuration as it will be written).
//!
//! Programming passwords never appear. Field and file names are those of RCDB V1.5.24.000.

use hytera_codeplug::decode::{DecodedFile, keys};
use hytera_codeplug::layout::FieldDef;
use hytera_codeplug::options::Options;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::edit::{ADDED, REMOVED, is_text};
use crate::view::is_secret;
use crate::workspace::{ChannelKind, FieldRef, Workspace, fields_at, is_editable, value_at};

/// Choice lists longer than this are edited as text (with the allowed values as a hint).
const MAX_CHOICES: usize = 512;

/// How a column's (or cell's) values are edited.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Editor {
    /// Free text up to `max_len` characters.
    Text { max_len: usize },
    /// One of CPS's named choices, or of the referenced records' names.
    Choice { options: Vec<String> },
    /// A number in the column's unit; `hint` describes the allowed values.
    Number { hint: String },
    ReadOnly,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    /// Field key, or a name for computed columns.
    pub key: String,
    pub title: String,
    /// Unit of the values (shown in the header, not in the cells).
    pub unit: String,
    pub editor: Editor,
}

#[derive(Debug, Clone, Serialize)]
pub struct Cell {
    pub text: String,
    /// Differs from the opened folder (always set for added records).
    pub changed: bool,
    pub editable: bool,
    /// The field behind the cell (absent for computed cells).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<FieldRef>,
    /// Editor for this cell when it differs from the column's (references whose choices depend on
    /// the record).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Editor>,
}

/// One record: where it is, for the detail view.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct RecordRef {
    pub file: u16,
    pub block: u16,
    pub record: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// 1-based number as CPS shows it.
    pub no: usize,
    pub record: RecordRef,
    /// Added since the folder was opened.
    pub added: bool,
    pub cells: Vec<Cell>,
    /// For zones: the zone's members.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<ZoneMember>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ZoneMember {
    pub no: usize,
    /// "Digital" or "Analog" (CPS's text).
    pub kind: String,
    /// 1-based channel number in its list.
    pub channel: i64,
    /// The channel's alias, as CPS lists zone members.
    pub alias: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Table {
    pub id: String,
    pub title: String,
    /// Maximum number of records the radio holds.
    pub capacity: usize,
    /// Records can be added and removed in the app.
    pub resizable: bool,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
}

/// One field in the detail view of a record.
#[derive(Debug, Clone, Serialize)]
pub struct DetailField {
    pub column: Column,
    pub cell: Cell,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeView {
    /// Where, e.g. `Digital channel 1 "HYT Test 1"`.
    pub place: String,
    pub setting: String,
    pub old: String,
    pub new: String,
    /// The changed field, so the change can be reverted (absent for added/removed records).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<FieldRef>,
}

/// A channel that can be put into a zone.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelOption {
    pub kind: ChannelKind,
    /// 1-based number in its list.
    pub number: usize,
    pub label: String,
}

struct TableSpec {
    id: &'static str,
    title: &'static str,
    file: &'static str,
    /// (field key, column title)
    columns: &'static [(&'static str, &'static str)],
    resizable: bool,
}

const DIGITAL: TableSpec = TableSpec {
    id: "digital",
    title: "Digital channels",
    file: "QEU_C_DChannel",
    columns: &[
        ("Channel Alias", "Alias"),
        ("RX Frequency", "RX frequency"),
        ("TX Frequency", "TX frequency"),
        ("Color Code", "Colour code"),
        ("Slot Operation", "Slot"),
        ("Power Level", "Power"),
        ("TX Contacts ID", "TX contact"),
        ("RX Group List", "RX group list"),
        ("TX Admit", "TX admit"),
        ("TX Timeout Time", "TX timeout"),
        ("RX Only", "RX only"),
    ],
    resizable: true,
};

const ANALOG: TableSpec = TableSpec {
    id: "analog",
    title: "Analog channels",
    file: "QEU_C_AChannel",
    columns: &[
        ("Channel Alias", "Alias"),
        ("RX Frequency", "RX frequency"),
        ("TX Frequency", "TX frequency"),
        ("Channel Spacing", "Spacing"),
        ("Power Level", "Power"),
        ("RX CTCorCDC Type", "RX tone type"),
        ("RX CTCorCDC Value", "RX tone"),
        ("TX CTCorCDC Type", "TX tone type"),
        ("TX CTCorCDC Value", "TX tone"),
        ("RX Squelch Mode", "RX squelch"),
        ("Scan List", "Scan list"),
        ("TX Admit", "TX admit"),
        ("TX Time Out Time", "TX timeout"),
        ("RX Only", "RX only"),
    ],
    resizable: true,
};

const CONTACTS: TableSpec = TableSpec {
    id: "contacts",
    title: "Contacts",
    file: "QEU_C_DContact",
    columns: &[("Call Alias", "Alias"), ("Call ID", "Call ID"), ("Call Type", "Call type")],
    resizable: false,
};

const ZONE_ALIASES: TableSpec = TableSpec {
    id: "zones",
    title: "Zones",
    file: "QEU_C_ZoneAlias",
    columns: &[("Zone Alias", "Alias")],
    resizable: false,
};

const ZONE_MEMBERS_FILE: &str = "QEU_C_Zone";

/// Ids of the tables [`table`] builds, in display order.
pub const TABLE_IDS: [&str; 4] = ["digital", "analog", "zones", "contacts"];

fn file<'a>(files: &'a [DecodedFile], name: &str) -> Option<&'a DecodedFile> {
    files.iter().find(|f| f.name == name)
}

/// Text CPS would show for `value` of field `def`, without the unit when `strip_unit`.
fn display(options: Option<&Options>, def: Option<&FieldDef>, key: &str, value: &Value, strip_unit: bool) -> String {
    if is_secret(key) {
        return "(hidden)".into();
    }
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            let shown = n.as_i64().zip(def).and_then(|(raw, def)| options?.display(def.id, raw).map(|t| (t, def)));
            match shown {
                Some((text, def)) if strip_unit => {
                    let unit = options.and_then(|o| o.param(def.id)).map_or("", |p| p.unit.as_str());
                    if unit.is_empty() { text } else { text.strip_suffix(&format!(" {unit}")).map_or(text.clone(), str::to_string) }
                }
                Some((text, _)) => text,
                None => n.to_string(),
            }
        }
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

fn unit(options: Option<&Options>, def: &FieldDef) -> String {
    options.and_then(|o| o.param(def.id)).map_or(String::new(), |p| p.unit.clone())
}

/// The editor for field `def` (ignoring the value; see [`is_editable`] for the per-cell check).
fn editor(options: Option<&Options>, def: &FieldDef, key: &str) -> Editor {
    if is_secret(key) || key.starts_with("Reserved") {
        return Editor::ReadOnly;
    }
    if is_text(def) {
        return Editor::Text { max_len: def.length };
    }
    let Some(o) = options else { return Editor::Number { hint: "raw number (no option catalogue loaded)".into() } };
    match o.param(def.id) {
        Some(p) if !p.choices.is_empty() && p.scales.is_empty() => {
            let mut choices: Vec<(i64, String)> = p
                .choices
                .iter()
                .filter_map(|c| o.label_for_edit(def.id, c.raw).map(|l| (c.raw, l)))
                .collect();
            choices.sort();
            choices.dedup_by(|a, b| a.1 == b.1);
            if choices.len() <= MAX_CHOICES {
                Editor::Choice { options: choices.into_iter().map(|(_, l)| l).collect() }
            } else {
                Editor::Number { hint: o.describe_allowed(def.id) }
            }
        }
        Some(p) if !p.scales.is_empty() => Editor::Number { hint: o.describe_allowed(def.id) },
        _ => Editor::Number { hint: "raw number".into() },
    }
}

fn column(ws: &Workspace, file: u16, block: u16, key: &str, title: &str) -> Column {
    let options = ws.options.as_ref();
    match crate::edit::def_for(&ws.layout, file, block, key) {
        Some(def) => Column { key: key.into(), title: title.into(), unit: unit(options, def), editor: editor(options, def, key) },
        None => Column { key: key.into(), title: title.into(), unit: String::new(), editor: Editor::ReadOnly },
    }
}

fn cell(ws: &Workspace, at: FieldRef, strip_unit: bool) -> Cell {
    let def = ws.def(&at);
    let Some(value) = value_at(ws.view(), &at) else {
        return Cell { text: String::new(), changed: false, editable: false, field: None, editor: None };
    };
    let refs = ws.reference_choices(&at);
    let text = match (&refs, value.as_i64()) {
        (Some(choices), Some(raw)) => crate::refs::label(choices, raw).unwrap_or_else(|| raw.to_string()),
        _ => display(ws.options.as_ref(), def, &at.key, value, strip_unit),
    };
    let changed = ws.original_value(&at) != Some(value);
    let editable = def.is_some_and(|d| is_editable(d, &at.key, value));
    let editor = refs.map(|choices| Editor::Choice { options: choices.into_iter().map(|(_, l)| l).collect() });
    Cell { text, changed, editable, field: Some(at), editor }
}

fn record_table(ws: &Workspace, spec: &TableSpec) -> Option<Table> {
    let f = file(ws.view(), spec.file)?;
    let block = f.blocks.first()?;
    let columns: Vec<Column> = spec.columns.iter().map(|(k, t)| column(ws, f.id, block.id, k, t)).collect();
    let rows = (0..block.records.len())
        .map(|record| Row {
            no: record + 1,
            record: RecordRef { file: f.id, block: block.id, record },
            added: ws.original_index(f.id, block.id, record).is_none(),
            cells: spec
                .columns
                .iter()
                .map(|(key, _)| {
                    cell(ws, FieldRef { file: f.id, block: block.id, record, member: None, key: (*key).into() }, true)
                })
                .collect(),
            members: None,
        })
        .collect();
    Some(Table {
        id: spec.id.into(),
        title: spec.title.into(),
        capacity: block.capacity.into(),
        resizable: spec.resizable,
        columns,
        rows,
    })
}

/// Alias of channel `number` (1-based) of member type `kind` (0 digital, 1 analog) in `files`.
fn channel_alias(files: &[DecodedFile], kind: i64, number: i64) -> Option<String> {
    let list = ChannelKind::from_member_type(kind)?.file();
    let index = usize::try_from(number.checked_sub(1)?).ok()?;
    let r = file(files, list)?.blocks.first()?.records.get(index)?;
    r.fields.as_ref()?.0.get("Channel Alias")?.as_str().map(str::to_string)
}

/// `Digital 3 "name"` for a zone member's channel in `files`.
fn channel_label(ws: &Workspace, files: &[DecodedFile], kind: i64, number: i64) -> String {
    let kind_text = display(ws.options.as_ref(), member_type_def(ws), "Channel Type", &Value::from(kind), false);
    match channel_alias(files, kind, number) {
        Some(alias) => format!("{kind_text} {number} \"{alias}\""),
        None => format!("{kind_text} {number}"),
    }
}

fn member_type_def(ws: &Workspace) -> Option<&FieldDef> {
    let zones = file(ws.base(), ZONE_MEMBERS_FILE)?;
    crate::edit::def_for(&ws.layout, zones.id, zones.blocks.first()?.id, "Channel Type")
}

/// Members of every zone (by zone position), with channel aliases.
fn zone_members(ws: &Workspace) -> Vec<Vec<ZoneMember>> {
    let Some(zones) = file(ws.view(), ZONE_MEMBERS_FILE) else { return Vec::new() };
    let Some(block) = zones.blocks.first() else { return Vec::new() };
    let type_def = member_type_def(ws);
    block
        .records
        .iter()
        .map(|r| {
            r.members
                .iter()
                .flatten()
                .enumerate()
                .map(|(i, m)| {
                    let kind_raw = m.0.get("Channel Type").cloned().unwrap_or(Value::Null);
                    let channel = m.0.get("Channel List").and_then(Value::as_i64).unwrap_or(0);
                    let kind_num = kind_raw.as_i64().unwrap_or(-1);
                    ZoneMember {
                        no: i + 1,
                        kind: display(ws.options.as_ref(), type_def, "Channel Type", &kind_raw, false),
                        channel,
                        alias: channel_alias(ws.view(), kind_num, channel).unwrap_or_else(|| format!("(channel {channel})")),
                    }
                })
                .collect()
        })
        .collect()
}

fn zones_table(ws: &Workspace) -> Option<Table> {
    let mut table = record_table(ws, &ZONE_ALIASES)?;
    let members = zone_members(ws);
    table.columns.push(Column { key: "members".into(), title: "Channels".into(), unit: String::new(), editor: Editor::ReadOnly });
    let base_members = |zone: usize| -> Option<Vec<(i64, i64)>> {
        let b = file(ws.base(), ZONE_MEMBERS_FILE)?.blocks.first()?;
        let ms = b.records.get(zone)?.members.as_ref()?;
        Some(ms.iter().filter_map(|m| Some((m.0.get("Channel Type")?.as_i64()?, m.0.get("Channel List")?.as_i64()?))).collect())
    };
    for row in &mut table.rows {
        let list = members.get(row.no - 1).cloned().unwrap_or_default();
        let text = list.iter().map(|m| m.alias.clone()).collect::<Vec<_>>().join(", ");
        // Changed when the zone's channels differ by identity, not just by renumbering.
        let changed = base_members(row.no - 1).is_none_or(|b| {
            let names = |files: &[DecodedFile], ms: &[(i64, i64)]| -> Vec<Option<String>> {
                ms.iter().map(|(t, n)| channel_alias(files, *t, *n)).collect()
            };
            names(ws.base(), &b) != names(ws.view(), &ws.zone_members(row.no - 1))
        });
        row.cells.push(Cell { text, changed, editable: false, field: None, editor: None });
        row.members = Some(list);
    }
    Some(table)
}

/// The table `id` (one of [`TABLE_IDS`]); `None` if the configuration has no such file.
pub fn table(ws: &Workspace, id: &str) -> Option<Table> {
    match id {
        "digital" => record_table(ws, &DIGITAL),
        "analog" => record_table(ws, &ANALOG),
        "contacts" => record_table(ws, &CONTACTS),
        "zones" => zones_table(ws),
        _ => None,
    }
}

/// Every channel, for choosing zone members.
pub fn channel_options(ws: &Workspace) -> Vec<ChannelOption> {
    [ChannelKind::Digital, ChannelKind::Analog]
        .into_iter()
        .flat_map(|kind| {
            (1..=ws.channel_count(kind)).map(move |number| ChannelOption {
                kind,
                number,
                label: channel_label(ws, ws.view(), kind.member_type(), number as i64),
            })
        })
        .collect()
}

/// Every field of one record except reserved bytes and passwords, labelled, in layout order.
pub fn record_detail(ws: &Workspace, r: &RecordRef) -> Vec<DetailField> {
    let defs = ws.layout.fields(r.file, r.block);
    let probe = FieldRef { file: r.file, block: r.block, record: r.record, member: None, key: String::new() };
    if fields_at(ws.view(), &probe).is_none() {
        return Vec::new();
    }
    keys(defs)
        .into_iter()
        .filter(|k| !is_secret(k) && !k.starts_with("Reserved"))
        .map(|key| {
            let at = FieldRef { key: key.clone(), ..probe.clone() };
            DetailField { column: column(ws, r.file, r.block, &key, &key), cell: cell(ws, at, true) }
        })
        .collect()
}

/// What to call a record in the change list. `files` is where its alias is looked up.
fn place(file_name: &str, block_short: &str, at: Option<&FieldRef>, position: &str, files: &[DecodedFile]) -> String {
    let (kind, alias_key) = match file_name {
        "QEU_C_DChannel" => ("Digital channel", "Channel Alias"),
        "QEU_C_AChannel" => ("Analog channel", "Channel Alias"),
        "QEU_C_ZoneAlias" => ("Zone", "Zone Alias"),
        "QEU_C_Zone" => ("Zone", ""),
        "QEU_C_DContact" => ("Contact", "Call Alias"),
        _ => return format!("{file_name} / {block_short} {position}"),
    };
    let Some(at) = at else { return format!("{kind} {}", position.trim_matches(['[', ']'])) };
    let alias = fields_at(files, &FieldRef { member: None, key: String::new(), ..at.clone() })
        .and_then(|f| f.get(alias_key))
        .and_then(Value::as_str)
        .map(|a| format!(" \"{a}\""))
        .unwrap_or_default();
    match at.member {
        Some(m) => format!("{kind} {}{alias} member {}", at.record + 1, m + 1),
        None => format!("{kind} {}{alias}", at.record + 1),
    }
}

/// Zone 1's name for zone member change places.
fn zone_alias(ws: &Workspace, zone: usize) -> Option<String> {
    let b = file(ws.view(), ZONE_ALIASES.file)?.blocks.first()?;
    b.records.get(zone)?.fields.as_ref()?.0.get("Zone Alias")?.as_str().map(str::to_string)
}

fn member_of(v: &Value) -> Option<(i64, i64)> {
    let m: &Map<String, Value> = v.as_object()?;
    Some((m.get("Channel Type")?.as_i64()?, m.get("Channel List")?.as_i64()?))
}

/// The pending changes, labelled.
pub fn change_list(ws: &Workspace) -> Vec<ChangeView> {
    let options = ws.options.as_ref();
    ws.changes()
        .iter()
        .map(|c| {
            let at = FieldRef::of_change(c);
            let block_short = ws
                .base()
                .iter()
                .find(|f| f.id == c.file_id)
                .and_then(|f| f.blocks.iter().find(|b| b.id == c.block_id))
                .map_or(String::new(), |b| b.short_name.trim().to_string());
            let structural = c.key == ADDED || c.key == REMOVED;
            let zone_file = c.file_name == ZONE_MEMBERS_FILE;
            // Removed records are named from the opened folder, everything else from the view.
            let mut place_text = if c.key == REMOVED && at.as_ref().is_some_and(|a| a.member.is_none()) {
                let alias = c.old.get("Channel Alias").or_else(|| c.old.get("Call Alias")).and_then(Value::as_str);
                let kind = place(&c.file_name, &block_short, None, &c.position, ws.base());
                alias.map_or(kind.clone(), |a| format!("{kind} \"{a}\""))
            } else {
                place(&c.file_name, &block_short, at.as_ref(), &c.position, ws.view())
            };
            if zone_file && let Some(a) = &at && let Some(name) = zone_alias(ws, a.record) {
                place_text = match a.member {
                    Some(m) => format!("Zone {} \"{name}\" member {}", a.record + 1, m + 1),
                    None => format!("Zone {} \"{name}\"", a.record + 1),
                };
            }
            let def = (!structural).then(|| crate::edit::def_for(&ws.layout, c.file_id, c.block_id, &c.key)).flatten();
            // Old values are member positions of the opened folder, new ones of the view.
            let member_at = |files: &[DecodedFile], record: Option<usize>| -> Option<(i64, i64)> {
                let a = at.as_ref()?;
                let f = fields_at(files, &FieldRef { record: record?, ..a.clone() })?;
                Some((f.get("Channel Type")?.as_i64()?, f.get("Channel List")?.as_i64()?))
            };
            let (old, new) = match (structural, zone_file, c.key.as_str()) {
                (true, true, _) => (
                    member_of(&c.old).map_or("-".into(), |(t, n)| channel_label(ws, ws.base(), t, n)),
                    member_of(&c.new).map_or("-".into(), |(t, n)| channel_label(ws, ws.view(), t, n)),
                ),
                (true, false, _) => {
                    let name = |v: &Value, files: &[DecodedFile]| -> String {
                        if v.is_null() {
                            return "-".into();
                        }
                        let given = v.get("Channel Alias").or_else(|| v.get("Call Alias")).and_then(Value::as_str);
                        let shown = at.as_ref().and_then(|a| fields_at(files, a)).and_then(|f| {
                            f.get("Channel Alias").or_else(|| f.get("Call Alias")).and_then(Value::as_str)
                        });
                        format!("\"{}\"", given.or(shown).unwrap_or("?"))
                    };
                    let added = c.key == ADDED;
                    (if added { "-".into() } else { name(&c.old, ws.base()) }, if added { name(&c.new, ws.view()) } else { "-".into() })
                }
                (false, true, "Channel List" | "Channel Type") => {
                    // The whole member before and after, by name.
                    let a = at.as_ref();
                    let base_record = a.and_then(|a| ws.original_index(a.file, a.block, a.record));
                    let label = |files: &[DecodedFile], record: Option<usize>| {
                        member_at(files, record).map_or("-".into(), |(t, n)| channel_label(ws, files, t, n))
                    };
                    (label(ws.base(), base_record), label(ws.view(), a.map(|a| a.record)))
                }
                _ => {
                    let refs = at.as_ref().and_then(|a| ws.reference_choices(a));
                    let text = |v: &Value, refs: Option<&Vec<(i64, String)>>| match (refs, v.as_i64()) {
                        (Some(r), Some(raw)) => crate::refs::label(r, raw).unwrap_or_else(|| raw.to_string()),
                        _ => display(options, def, &c.key, v, false),
                    };
                    (text(&c.old, refs.as_ref()), text(&c.new, refs.as_ref()))
                }
            };
            let setting = match (c.key.as_str(), zone_file) {
                (ADDED, true) => "Channel added".to_string(),
                (REMOVED, true) => "Channel removed".to_string(),
                (ADDED, false) => "Added".to_string(),
                (REMOVED, false) => "Removed".to_string(),
                ("Channel List" | "Channel Type", true) => "Channel".to_string(),
                (k, _) => k.to_string(),
            };
            // Reverting is offered for value changes of records that existed, outside zone members.
            let field = at.filter(|a| !structural && a.member.is_none() && ws.original_value(a).is_some());
            ChangeView { place: place_text, setting, old, new, field }
        })
        .fold(Vec::new(), |mut out: Vec<ChangeView>, c| {
            // A member whose type and number both changed is one change.
            let duplicate = out.last().is_some_and(|l| l.setting == "Channel" && c.setting == "Channel" && l.place == c.place);
            if !duplicate {
                out.push(c);
            }
            out
        })
}
