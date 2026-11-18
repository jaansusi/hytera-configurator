//! Fields that refer to other records, shown by the referenced record's name (open question 5b).
//!
//! Inferred 2026-11-19, not yet confirmed on CPS's screens: list references are 1-based positions in
//! the named list with 0 = None (their RCDB ranges are 0..capacity, e.g. RX group list 0..32 with 32
//! RX group lists); a digital channel's `Scan List Roam List` points into the scan lists or the roam
//! lists depending on its `Use Scan or Roam List` (0 None, 1 scan, 2 roam). `TX Contacts ID` holds a
//! call ID (from CPS's database, see docs/file-format.md §2a), so it names the contact with that ID.

use hytera_codeplug::decode::DecodedFile;
use serde_json::{Map, Value};

/// A field that refers to another list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Contacts, by call ID.
    Contact,
    /// 1-based position in `file` (name field `alias`), 0 = None.
    List { file: &'static str, alias: &'static str },
    /// Scan or roam list, chosen by the record's `Use Scan or Roam List`.
    ScanOrRoam,
}

const SCAN_LISTS: Target = Target::List { file: "QEU_C_ScanConfig", alias: "Scan List Alias" };
const ROAM_LISTS: Target = Target::List { file: "QEU_C_RoamConfig", alias: "Roam List Alias" };

/// (file, field, target) for RCDB V1.5.24.000.
const REFS: &[(&str, &str, Target)] = &[
    ("QEU_C_DChannel", "TX Contacts ID", Target::Contact),
    ("QEU_C_DChannel", "RX Group List", Target::List { file: "QEU_C_RxGrpAlias", alias: "Rx Group List Alias" }),
    ("QEU_C_DChannel", "Scan List Roam List", Target::ScanOrRoam),
    ("QEU_C_AChannel", "Scan List", SCAN_LISTS),
    ("QEU_C_AChannel", "Emergency System", Target::List { file: "QEU_C_AEmergency", alias: "Emergency System Alias" }),
];

fn records<'a>(files: &'a [DecodedFile], name: &str) -> impl Iterator<Item = &'a Map<String, Value>> {
    files
        .iter()
        .filter(move |f| f.name == name)
        .flat_map(|f| f.blocks.first())
        .flat_map(|b| &b.records)
        .filter_map(|r| r.fields.as_ref().map(|f| &f.0))
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

/// Whether `file`/`key` refers to another list.
pub fn is_reference(file: &str, key: &str) -> bool {
    REFS.iter().any(|(f, k, _)| *f == file && *k == key)
}

/// The values field `key` of a record of `file` (with fields `record`) may take, with the names to
/// show, from the configuration `files`. `None` when the field is not a reference.
pub fn choices(files: &[DecodedFile], file: &str, key: &str, record: &Map<String, Value>) -> Option<Vec<(i64, String)>> {
    let (_, _, target) = REFS.iter().find(|(f, k, _)| *f == file && *k == key)?;
    let target = match target {
        Target::ScanOrRoam => match record.get("Use Scan or Roam List").and_then(Value::as_i64) {
            Some(1) => SCAN_LISTS,
            Some(2) => ROAM_LISTS,
            _ => return Some(vec![(0, "None".into())]),
        },
        t => *t,
    };
    Some(unique(match target {
        Target::Contact => records(files, "QEU_C_DContact")
            .filter_map(|c| Some((c.get("Call ID")?.as_i64()?, c.get("Call Alias")?.as_str()?.to_string())))
            .collect(),
        Target::List { file, alias } => std::iter::once((0, "None".to_string()))
            .chain(records(files, file).enumerate().map(|(i, r)| {
                let name = r.get(alias).and_then(Value::as_str).unwrap_or("?").to_string();
                (i as i64 + 1, name)
            }))
            .collect(),
        Target::ScanOrRoam => unreachable!("resolved above"),
    }))
}

/// Name for `raw` among `choices`, or `None` if it matches none (then the number is shown).
pub fn label(choices: &[(i64, String)], raw: i64) -> Option<String> {
    choices.iter().find(|(r, _)| *r == raw).map(|(_, l)| l.clone())
}

/// Raw value for a name shown by [`choices`].
pub fn parse(choices: &[(i64, String)], text: &str) -> Option<i64> {
    let text = text.trim();
    choices.iter().find(|(_, l)| l == text).map(|(r, _)| *r)
}
