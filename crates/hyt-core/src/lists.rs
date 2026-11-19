//! List editors: the radio's lists other than channels, zones and contacts (RX group, scan and
//! roam lists, one-touch calls, preset channels, quick messages, phone, HDC1200 and 2-Tone lists),
//! as GUI tables whose values are edited through [`Workspace::set_value`] like every other table.
//!
//! Nothing is added or removed here: CPS has not been recorded adding or removing these records or
//! their members, so record positions never change and members are shown read-only, by name.
//! Files the model profile never writes (e.g. the E2EE key list) are left out. Field and file
//! names are those of RCDB V1.5.24.000; columns follow the order of CPS's screens.

use hytera_codeplug::decode::DecodedFile;
use serde::Serialize;
use serde_json::Value;

use crate::tables::{Cell, Column, Editor, RecordRef, Row, Table, cell, channel_alias, column, display, file};
use crate::workspace::{FieldRef, Workspace};

/// How the members of a two-level list are named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Members {
    /// `Channel List` + `Channel Type` (0 digital, 1 analog); 65535 = the selected channel.
    Channels,
    /// `Channel List` of digital channels; 65535 = the selected channel.
    DigitalChannels,
    /// `Call ID` of a contact (inferred: named after the contact with that call ID).
    Contacts,
}

struct ListSpec {
    /// The file holding the list's records (aliases/settings).
    file: &'static str,
    title: &'static str,
    /// One record, e.g. "Scan list".
    item: &'static str,
    /// (field key, column title), first the record's name where it has one.
    columns: &'static [(&'static str, &'static str)],
    /// The partner file holding each record's members (same position, `TableLink.xml`).
    members: Option<(&'static str, Members)>,
}

/// The lists, in display order (CPS's screens: Conventional › Contact, Scan, Roam, Basic Setting,
/// Digital Setting, Phone, Analog Service).
const LISTS: &[ListSpec] = &[
    ListSpec {
        file: "QEU_C_RxGrpAlias",
        title: "RX group lists",
        item: "RX group list",
        columns: &[("Rx Group List Alias", "Alias")],
        members: Some(("QEU_C_RxGrpList", Members::Contacts)),
    },
    ListSpec {
        file: "QEU_C_ScanConfig",
        title: "Scan lists",
        item: "Scan list",
        columns: &[
            ("Scan List Alias", "Alias"),
            ("Scan Type", "Scan type"),
            ("Scan TX Mode", "TX mode"),
            ("CTCorCDC Scan Mode", "CTCSS/CDCSS scan"),
            ("Designated Channel", "Designated channel"),
            ("DesignatedCH ChType", "Designated type"),
            ("Priority Channel 1", "Priority 1"),
            ("Priority Channel 2", "Priority 2"),
            ("Hang Time", "Hang time"),
        ],
        members: Some(("QEU_C_ScanList", Members::Channels)),
    },
    ListSpec {
        file: "QEU_C_RoamConfig",
        title: "Roam lists",
        item: "Roam list",
        columns: &[
            ("Roam List Alias", "Alias"),
            ("RSSI Threshold", "RSSI threshold"),
            ("Active Site Roam", "Active site roam"),
            ("Return To Selected CH", "Return to selected"),
            ("Roam Interval Time", "Interval"),
        ],
        members: Some(("QEU_C_RoamList", Members::DigitalChannels)),
    },
    ListSpec {
        file: "QEU_C_OneTouchCall",
        title: "One-touch calls",
        item: "One-touch call",
        columns: &[
            ("Call Mode", "Call mode"),
            ("Contact Type", "Contact type"),
            ("Contact ID", "Contact ID"),
            ("Call Message Type", "Message type"),
        ],
        members: None,
    },
    ListSpec {
        file: "QEU_C_PresetChannel",
        title: "Preset channels",
        item: "Preset channel",
        columns: &[
            ("Zone Index", "Zone index"),
            ("Channel Index", "Channel index"),
            ("Current Zone Type", "Zone type"),
            ("Channel Type", "Channel type"),
        ],
        members: None,
    },
    ListSpec {
        file: "QEU_C_Quick_Message",
        title: "Quick messages",
        item: "Quick message",
        columns: &[("Text Message Content", "Message")],
        members: None,
    },
    ListSpec {
        file: "QEU_C_PhoneList",
        title: "Phone contacts",
        item: "Phone contact",
        columns: &[("Phone Call Alias", "Alias"), ("Phone Number", "Number")],
        members: None,
    },
    ListSpec {
        file: "QEU_C_PhoneSystem",
        title: "Phone systems",
        item: "Phone system",
        columns: &[("Phone System Alias", "Alias"), ("Digit Duration", "Digit duration"), ("Hold Time", "Hold time")],
        members: None,
    },
    ListSpec {
        file: "QEU_C_HDC1200_System",
        title: "HDC1200 systems",
        item: "HDC1200 system",
        columns: &[
            ("System Alias", "Alias"),
            ("PrimaryID", "Primary ID"),
            ("Group ID", "Group ID"),
            ("PTT ID Type", "PTT ID type"),
            ("Pretime", "Pretime"),
        ],
        members: None,
    },
    ListSpec {
        file: "QEU_C_HDC1200_Contact",
        title: "HDC1200 contacts",
        item: "HDC1200 contact",
        columns: &[("Call Alias", "Alias"), ("HDC System", "System"), ("Call Type", "Call type"), ("Call ID", "Call ID")],
        members: None,
    },
    ListSpec {
        file: "QEU_C_2Tone_System",
        title: "2-Tone systems",
        item: "2-Tone system",
        columns: &[
            ("System Alias", "Alias"),
            ("Pretime", "Pretime"),
            ("A Tone", "A tone"),
            ("B Tone", "B tone"),
            ("C Tone", "C tone"),
            ("D Tone", "D tone"),
        ],
        members: None,
    },
    ListSpec {
        file: "QEU_C_2Tone_Contact",
        title: "2-Tone contacts",
        item: "2-Tone contact",
        columns: &[
            ("Call Alias", "Alias"),
            ("Encode Format", "Encode format"),
            ("TwoTone System", "System"),
            ("First Tone Frequency", "First tone"),
            ("Second Tone Frequency", "Second tone"),
        ],
        members: None,
    },
    ListSpec {
        file: "QEU_C_2Tone_Personality",
        title: "2-Tone personalities",
        item: "2-Tone personality",
        columns: &[
            ("PersonalityList", "Personality"),
            ("Signaling Type 1", "Signaling 1"),
            ("Call List 1", "Call list 1"),
            ("Signaling Type 2", "Signaling 2"),
            ("Call List 2", "Call list 2"),
        ],
        members: None,
    },
];

/// Prefix of list table ids: `list:<file name>`.
pub const ID_PREFIX: &str = "list:";

/// `Channel List` value meaning "the selected channel" (scan and roam list members).
const SELECTED: i64 = 65535;

const NOTE_EMPTY: &str = "No records. Adding records needs a recording of CPS doing it first.";
const NOTE_FIXED: &str = "Values can be edited; adding or removing records needs a recording of CPS doing it first.";
const NOTE_MEMBERS: &str = "Select a list to see its members. Values can be edited; adding or removing lists or \
                            changing members needs a recording of CPS doing it first.";

/// One member of a two-level list, read-only.
#[derive(Debug, Clone, Serialize)]
pub struct ListMember {
    pub no: usize,
    /// The referenced record's name (channel alias, contact alias) or `Selected`.
    pub name: String,
    /// What the member holds, e.g. `Digital 1` or `Call ID 10`.
    pub refers_to: String,
}

/// A list in the navigation.
#[derive(Debug, Clone, Serialize)]
pub struct ListInfo {
    pub id: String,
    pub title: String,
    pub rows: usize,
    pub capacity: usize,
}

fn spec(id: &str) -> Option<&'static ListSpec> {
    let name = id.strip_prefix(ID_PREFIX)?;
    LISTS.iter().find(|s| s.file == name)
}

/// Whether the profile writes `file` (lists in files it never writes are left out).
fn writable(ws: &Workspace, file: &DecodedFile) -> bool {
    !crate::settings::profile(ws).write_excluded_files.contains(&file.id)
}

/// The lists the configuration has, in display order, with their sizes.
pub fn index(ws: &Workspace) -> Vec<ListInfo> {
    LISTS
        .iter()
        .filter_map(|s| table(ws, &format!("{ID_PREFIX}{}", s.file)))
        .map(|t| ListInfo { id: t.id, title: t.title, rows: t.rows.len(), capacity: t.capacity })
        .collect()
}

/// Contacts' (alias, call type) by call ID.
fn contacts_with_id(files: &[DecodedFile], call_id: i64) -> Vec<(String, Option<i64>)> {
    file(files, "QEU_C_DContact")
        .and_then(|f| f.blocks.first())
        .map(|b| &b.records[..])
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r.fields.as_ref())
        .filter(|f| f.0.get("Call ID").and_then(Value::as_i64) == Some(call_id))
        .filter_map(|f| Some((f.0.get("Call Alias")?.as_str()?.to_string(), f.0.get("Call Type").and_then(Value::as_i64))))
        .collect()
}

/// Name of the contact an RX group list member's call ID refers to, when that is unambiguous:
/// the only contact with that call ID, or else the only group-call contact with it (inferred).
pub(crate) fn contact_name(files: &[DecodedFile], call_id: i64) -> Option<String> {
    let found = contacts_with_id(files, call_id);
    if let [(name, _)] = &found[..] {
        return Some(name.clone());
    }
    let groups: Vec<&String> = found.iter().filter(|(_, t)| *t == Some(1)).map(|(n, _)| n).collect();
    match groups[..] {
        [name] => Some(name.clone()),
        _ => None,
    }
}

/// The members of the record at `record` of the member file `members_file`, named.
fn members_of(ws: &Workspace, members_file: &str, kind: Members, record: usize) -> Vec<ListMember> {
    let Some(f) = file(ws.view(), members_file) else { return Vec::new() };
    let Some(block) = f.blocks.first() else { return Vec::new() };
    let type_def = crate::edit::def_for(&ws.layout, f.id, block.id, "Channel Type");
    let Some(members) = block.records.get(record).and_then(|r| r.members.as_ref()) else { return Vec::new() };
    members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let get = |k: &str| m.0.get(k).and_then(Value::as_i64);
            let (name, refers_to) = match kind {
                Members::Contacts => {
                    let id = get("Call ID").unwrap_or(0);
                    let name = contact_name(ws.view(), id).unwrap_or_else(|| format!("(call ID {id})"));
                    (name, format!("Call ID {id}"))
                }
                Members::Channels | Members::DigitalChannels => {
                    let number = get("Channel List").unwrap_or(0);
                    let t = if kind == Members::Channels { get("Channel Type").unwrap_or(-1) } else { 0 };
                    if number == SELECTED {
                        ("Selected".to_string(), "The selected channel".to_string())
                    } else {
                        let kind_text = if kind == Members::Channels {
                            let raw = m.0.get("Channel Type").cloned().unwrap_or(Value::Null);
                            display(ws.options.as_ref(), type_def, "Channel Type", &raw, false)
                        } else {
                            "Digital".to_string()
                        };
                        let alias = channel_alias(ws.view(), t, number).unwrap_or_else(|| format!("(channel {number})"));
                        (alias, format!("{kind_text} {number}"))
                    }
                }
            };
            ListMember { no: i + 1, name, refers_to }
        })
        .collect()
}

/// The list table `id` (`list:<file name>`); `None` if the configuration has no such list or the
/// profile never writes its file.
pub fn table(ws: &Workspace, id: &str) -> Option<Table> {
    let spec = spec(id)?;
    let f = file(ws.view(), spec.file)?;
    if !writable(ws, f) {
        return None;
    }
    let block = f.blocks.first()?;
    let mut capacity = usize::from(block.capacity);
    let members_file = spec.members.and_then(|(name, kind)| Some((file(ws.view(), name)?, name, kind)));
    if let Some((mf, _, _)) = members_file
        && let Some(mb) = mf.blocks.first()
    {
        // The two files can have different capacities (scan lists: 8 member lists, 64 settings
        // records); only as many lists as both hold can be complete.
        capacity = capacity.min(usize::from(mb.capacity));
    }
    let mut columns: Vec<Column> = spec.columns.iter().map(|(k, t)| column(ws, f.id, block.id, k, t)).collect();
    if members_file.is_some() {
        let title = if spec.members.is_some_and(|(_, k)| k == Members::Contacts) { "Contacts" } else { "Channels" };
        columns.push(Column { key: "members".into(), title: title.into(), unit: String::new(), editor: Editor::ReadOnly });
    }
    let rows: Vec<Row> = (0..block.records.len())
        .map(|record| {
            let mut cells: Vec<Cell> = spec
                .columns
                .iter()
                .map(|(key, _)| cell(ws, FieldRef { file: f.id, block: block.id, record, member: None, key: (*key).into() }, true))
                .collect();
            let list_members = members_file.map(|(_, name, kind)| members_of(ws, name, kind, record));
            if let Some(ms) = &list_members {
                let text = ms.iter().map(|m| m.name.clone()).collect::<Vec<_>>().join(", ");
                cells.push(Cell { text, changed: false, editable: false, field: None, editor: None });
            }
            Row {
                no: record + 1,
                record: RecordRef { file: f.id, block: block.id, record },
                added: false,
                cells,
                members: None,
                list_members,
            }
        })
        .collect();
    let note = if rows.is_empty() {
        NOTE_EMPTY
    } else if members_file.is_some() {
        NOTE_MEMBERS
    } else {
        NOTE_FIXED
    };
    Some(Table {
        id: format!("{ID_PREFIX}{}", spec.file),
        title: spec.title.into(),
        capacity,
        resizable: false,
        columns,
        rows,
        item: Some(spec.item.into()),
        note: Some(note.into()),
    })
}

/// The name field of a record of `spec`'s file, if its first column is text.
fn record_name(files: &[DecodedFile], spec: &ListSpec, record: usize) -> Option<String> {
    let f = file(files, spec.file)?;
    let r = f.blocks.first()?.records.get(record)?;
    let first = spec.columns.first()?.0;
    r.fields.as_ref()?.0.get(first)?.as_str().map(str::to_string)
}

/// What to call the record (or member) at `at` of `file_name` in the change list, e.g.
/// `Scan list 1 "Scan List 1"`; `None` for files that are not one of these lists.
pub(crate) fn place_of(ws: &Workspace, file_name: &str, at: &FieldRef) -> Option<String> {
    let spec = LISTS.iter().find(|s| s.file == file_name || s.members.is_some_and(|(m, _)| m == file_name))?;
    let name = record_name(ws.view(), spec, at.record).map(|n| format!(" \"{n}\"")).unwrap_or_default();
    Some(match at.member {
        Some(m) => format!("{} {}{name} member {}", spec.item, at.record + 1, m + 1),
        None => format!("{} {}{name}", spec.item, at.record + 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hytera_codeplug::decode::{DecodedBlock, Fields, Record};
    use serde_json::json;

    fn contacts(list: &[(&str, i64, i64)]) -> Vec<DecodedFile> {
        let records = list
            .iter()
            .enumerate()
            .map(|(i, (alias, id, call_type))| Record {
                index: Some(i),
                fields: Some(Fields(
                    json!({"Call Alias": alias, "Call ID": id, "Call Type": call_type}).as_object().unwrap().clone(),
                )),
                members: None,
            })
            .collect();
        vec![DecodedFile {
            id: 8,
            name: "QEU_C_DContact".into(),
            blocks: vec![DecodedBlock { id: 3, short_name: "Conv Con".into(), two_dim: false, capacity: 160, records }],
        }]
    }

    #[test]
    fn rx_group_members_are_named_only_when_unambiguous() {
        let files = contacts(&[("Call 1", 1, 1), ("Alice", 7, 0), ("Team 9", 9, 1), ("Bob 9", 9, 0), ("A", 5, 1), ("B", 5, 1)]);
        assert_eq!(contact_name(&files, 1).as_deref(), Some("Call 1"));
        assert_eq!(contact_name(&files, 7).as_deref(), Some("Alice"));
        // Private and group contact share the ID: the group call is the RX group member.
        assert_eq!(contact_name(&files, 9).as_deref(), Some("Team 9"));
        // Two group contacts with the same ID, or none: no name.
        assert_eq!(contact_name(&files, 5), None);
        assert_eq!(contact_name(&files, 10), None);
    }
}
