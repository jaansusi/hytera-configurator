//! Editing settings in place: labelled JSON, applying edits with validation, and field-level diffs.
//!
//! Only values change; adding or removing records/members is not supported yet (the structure of
//! an edited file must match the original).

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use hytera_codeplug::container::prepare_for_write;
use hytera_codeplug::decode::{DecodedFile, Fields, Record, apply_with, decode, default_new_record, keys};
use hytera_codeplug::rcdb::TableLink;
use hytera_codeplug::structure::{self, Block};
use hytera_codeplug::layout::{FieldDef, FieldType, Layout};
use hytera_codeplug::options::Options;
use serde_json::{Map, Value};

use crate::SavedRead;

/// One changed field.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub file_id: u16,
    pub file_name: String,
    pub block_id: u16,
    /// `[record]` or `[record.member]`, 1-based.
    pub position: String,
    pub key: String,
    pub old: Value,
    pub new: Value,
}

/// Whether the field holds text (edited as text, never labelled).
pub fn is_text(def: &FieldDef) -> bool {
    matches!(def.kind, FieldType::UnicodeString | FieldType::AsciiString)
}

/// Definition of field `key` (a [`keys`] entry) of `file`/`block`.
pub fn def_for<'a>(layout: &'a Layout, file: u16, block: u16, key: &str) -> Option<&'a FieldDef> {
    let defs = layout.fields(file, block);
    keys(defs).iter().position(|k| k == key).map(|i| &defs[i])
}

/// Call `f(block_id, position, fields)` for every field map of `file`.
fn visit_mut(
    file: &mut DecodedFile,
    mut f: impl FnMut(u16, String, &mut Map<String, Value>) -> Result<()>,
) -> Result<()> {
    for b in &mut file.blocks {
        for (pos, r) in b.records.iter_mut().enumerate() {
            // Position in this file's list (1-based), so new records are named sensibly too.
            let n = pos + 1;
            if let Some(fields) = &mut r.fields {
                f(b.id, format!("[{n}]"), &mut fields.0)?;
            }
            if let Some(members) = &mut r.members {
                for (m, fields) in members.iter_mut().enumerate() {
                    f(b.id, format!("[{n}.{}]", m + 1), &mut fields.0)?;
                }
            }
        }
    }
    Ok(())
}

/// Replace raw numbers by CPS's display text wherever that text converts back to the same raw
/// value (so labelled JSON round-trips exactly).
pub fn labelize(files: &mut [DecodedFile], layout: &Layout, options: &Options) {
    for file in files {
        let id = file.id;
        let _ = visit_mut(file, |block, _, map| {
            let defs = layout.fields(id, block);
            for (key, def) in keys(defs).iter().zip(defs) {
                if is_text(def) {
                    continue;
                }
                if let Some(Value::Number(n)) = map.get(key)
                    && let Some(label) = n.as_i64().and_then(|raw| options.label_for_edit(def.id, raw))
                {
                    map.insert(key.clone(), Value::String(label));
                }
            }
            Ok(())
        });
    }
}

/// Turn display text in numeric fields back into raw numbers.
pub fn delabelize(file: &mut DecodedFile, layout: &Layout, options: Option<&Options>) -> Result<()> {
    let (id, name) = (file.id, file.name.clone());
    visit_mut(file, |block, position, map| {
        for (key, value) in map.iter_mut() {
            let Value::String(text) = value else { continue };
            let def = def_for(layout, id, block, key)
                .ok_or_else(|| anyhow!("file {id} ({name}) block {block} {position}: unknown field {key:?}"))?;
            if is_text(def) {
                continue;
            }
            let raw = options.and_then(|o| o.parse_display(def.id, text)).ok_or_else(|| {
                let allowed = options.map_or("(no option catalogue loaded; use a raw number)".into(), |o| o.describe_allowed(def.id));
                anyhow!("file {id} ({name}) {position} {key}: {text:?} is not a valid value; allowed: {allowed}")
            })?;
            *value = Value::from(raw);
        }
        Ok(())
    })
}

/// Key used for a whole record or member that was added or removed.
pub const ADDED: &str = "(added)";
pub const REMOVED: &str = "(removed)";

fn record_value(r: &Record) -> Value {
    match (&r.fields, &r.members) {
        (Some(f), _) => Value::Object(f.0.clone()),
        (None, Some(m)) => Value::Array(m.iter().map(|f| Value::Object(f.0.clone())).collect()),
        (None, None) => Value::Null,
    }
}

/// Field-level differences between two decodings of the same file. Records of `new` are matched
/// to `old` by their `index` (entries without one are [`ADDED`]; old records not referenced are
/// [`REMOVED`]); members are compared by position. Fields missing from `new` count as unchanged.
pub fn changes(old: &DecodedFile, new: &DecodedFile) -> Result<Vec<Change>> {
    let mut out = Vec::new();
    if old.blocks.len() != new.blocks.len() {
        bail!("file {} ({}): different number of blocks", old.id, old.name);
    }
    let mut push = |block: u16, position: String, key: &str, old_v: Value, new_v: Value| {
        out.push(Change {
            file_id: old.id,
            file_name: old.name.clone(),
            block_id: block,
            position,
            key: key.to_string(),
            old: old_v,
            new: new_v,
        });
    };
    for (ob, nb) in old.blocks.iter().zip(&new.blocks) {
        if ob.id != nb.id {
            bail!("file {} ({}): block {} vs {}", old.id, old.name, ob.id, nb.id);
        }
        let mut seen = vec![false; ob.records.len()];
        let mut pairs: Vec<(String, Option<&Record>, Option<&Record>)> = Vec::new();
        for (p, nr) in nb.records.iter().enumerate() {
            match nr.index.filter(|&i| i < ob.records.len() && !seen[i]) {
                Some(i) => {
                    seen[i] = true;
                    pairs.push((format!("[{}]", i + 1), Some(&ob.records[i]), Some(nr)));
                }
                None => pairs.push((format!("[{}]", p + 1), None, Some(nr))),
            }
        }
        for (i, or) in ob.records.iter().enumerate() {
            if !seen[i] {
                pairs.push((format!("[{}]", i + 1), Some(or), None));
            }
        }
        for (pos, o, n) in pairs {
            let i = pos.trim_matches(['[', ']']).parse::<usize>().unwrap_or(1) - 1;
            match (o, n) {
                (Some(or), Some(nr)) => match ((&or.fields, &or.members), (&nr.fields, &nr.members)) {
                    ((Some(a), _), (Some(b), _)) => {
                        for (key, nv) in &b.0 {
                            if a.0.get(key) != Some(nv) {
                                push(ob.id, pos.clone(), key, a.0.get(key).cloned().unwrap_or(Value::Null), nv.clone());
                            }
                        }
                    }
                    ((_, Some(am)), (_, Some(bm))) => {
                        for m in 0..am.len().max(bm.len()) {
                            let mpos = format!("[{}.{}]", i + 1, m + 1);
                            match (am.get(m), bm.get(m)) {
                                (Some(a), Some(b)) => {
                                    for (key, nv) in &b.0 {
                                        if a.0.get(key) != Some(nv) {
                                            push(ob.id, mpos.clone(), key, a.0.get(key).cloned().unwrap_or(Value::Null), nv.clone());
                                        }
                                    }
                                }
                                (None, Some(b)) => push(ob.id, mpos, ADDED, Value::Null, Value::Object(b.0.clone())),
                                (Some(a), None) => push(ob.id, mpos, REMOVED, Value::Object(a.0.clone()), Value::Null),
                                (None, None) => {}
                            }
                        }
                    }
                    _ => {}
                },
                (None, Some(nr)) => push(ob.id, pos, ADDED, Value::Null, record_value(nr)),
                (Some(or), None) => push(ob.id, pos, REMOVED, record_value(or), Value::Null),
                (None, None) => {}
            }
        }
    }
    Ok(out)
}

/// All files of a configuration (edited ones re-encoded) plus the list of changes.
pub type Edited = (BTreeMap<u16, Vec<u8>>, Vec<Change>);

fn file_name(saved: &SavedRead, id: u16) -> Option<&str> {
    saved.manifest.files.iter().find(|f| f.id == id).map(|f| f.name.as_str())
}

fn file_id(saved: &SavedRead, name: &str) -> Option<u16> {
    saved.manifest.files.iter().find(|f| f.name == name).map(|f| f.id)
}

/// CPS creates a new record in `QEU_C_X` by copying the record of `QEU_C_Template_X` when that
/// file exists (observed for digital channels 2026-11-18); otherwise the RCDB defaults are used.
fn template_record(saved: &SavedRead, name: &str, b: &Block) -> Option<Vec<u8>> {
    let template_name = name.replacen("QEU_C_", "QEU_C_Template_", 1);
    if template_name == name {
        return None;
    }
    let bytes = saved.files.get(&file_id(saved, &template_name)?)?;
    let s = structure::parse(bytes).ok()?;
    let range = s.blocks.first()?.records.first()?.clone();
    (range.len() == b.record_size).then(|| bytes[range].to_vec())
}

/// Checks values written into one file against the option catalogue.
struct ValueCheck<'a> {
    layout: &'a Layout,
    options: Option<&'a Options>,
    file: u16,
    name: &'a str,
}

impl ValueCheck<'_> {
    fn check(&self, block: u16, position: &str, key: &str, v: &Value) -> Result<()> {
        let (file, name) = (self.file, self.name);
        let def = def_for(self.layout, file, block, key)
            .ok_or_else(|| anyhow!("file {file} ({name}) {position}: unknown field {key:?}"))?;
        if let (Some(o), Some(raw)) = (self.options, v.as_i64())
            && o.allows(def.id, raw) == Some(false)
        {
            bail!("file {file} ({name}) {position} {key}: {raw} is not allowed; allowed: {}", o.describe_allowed(def.id));
        }
        Ok(())
    }
}

/// Channel lists whose records may be removed, with the zone-member `Channel Type` that refers to them.
const CHANNEL_LISTS: [(&str, i64); 2] = [("QEU_C_DChannel", 0), ("QEU_C_AChannel", 1)];

/// The current version of a file: the edited one if present, otherwise the saved one decoded.
fn current(saved: &SavedRead, layout: &Layout, edits: &[DecodedFile], name: &str) -> Result<Option<DecodedFile>> {
    let Some(id) = file_id(saved, name) else { return Ok(None) };
    if let Some(e) = edits.iter().find(|e| e.id == id) {
        return Ok(Some(e.clone()));
    }
    Ok(Some(decode(&saved.files[&id], name, layout)?))
}

/// For a channel list changed by the edit: new 1-based number of each old channel number (index =
/// old number − 1; `None` = removed), counting channels appended by the edit as old numbers after
/// the original ones. `None` when numbering is unchanged.
fn channel_renumbering(saved: &SavedRead, edits: &[DecodedFile], name: &str) -> Result<Option<Vec<Option<i64>>>> {
    let Some(id) = file_id(saved, name) else { return Ok(None) };
    let Some(edit) = edits.iter().find(|e| e.id == id) else { return Ok(None) };
    let old_count = record_count(&saved.files[&id]).unwrap_or(0);
    let Some(block) = edit.blocks.first() else { return Ok(None) };
    let mut map = vec![None; old_count];
    for (pos, r) in block.records.iter().enumerate() {
        match r.index.filter(|&i| i < old_count) {
            Some(i) => map[i] = Some(pos as i64 + 1),
            None => map.push(Some(pos as i64 + 1)),
        }
    }
    let identity = map.iter().enumerate().all(|(i, n)| *n == Some(i as i64 + 1));
    Ok((!identity).then_some(map))
}

fn member_values(f: &Fields, keys: &[&str]) -> Vec<Option<i64>> {
    keys.iter().map(|k| f.0.get(*k).and_then(Value::as_i64)).collect()
}

/// A field that may point to a conventional channel, other than zone members (which are updated).
struct ChannelRef {
    file: &'static str,
    field: &'static str,
    /// Field holding the channel type (0 digital, 1 analog) next to it, if any.
    kind_field: Option<&'static str>,
    /// Channel type when the field always refers to one list.
    kind: Option<i64>,
    /// What to call it in messages.
    what: &'static str,
}

/// Every channel reference in RCDB V1.5.24.000 (except zones). CPS's handling of these when a
/// channel is deleted has not been observed, so an edit that would affect one is refused.
const CHANNEL_REFS: &[ChannelRef] = &[
    ChannelRef { file: "QEU_C_ScanList", field: "Channel List", kind_field: Some("Channel Type"), kind: None, what: "a scan list" },
    ChannelRef { file: "QEU_C_RoamList", field: "Channel List", kind_field: None, kind: Some(0), what: "a roam list" },
    ChannelRef { file: "QEU_C_ScanConfig", field: "Designated Channel", kind_field: Some("DesignatedCH ChType"), kind: None, what: "a scan list's designated channel" },
    ChannelRef { file: "QEU_C_ScanConfig", field: "Priority Channel 1", kind_field: Some("PriCH1 ChType"), kind: None, what: "a scan list's priority channel 1" },
    ChannelRef { file: "QEU_C_ScanConfig", field: "Priority Channel 2", kind_field: Some("PriCH2 ChType"), kind: None, what: "a scan list's priority channel 2" },
    ChannelRef { file: "QEU_C_DEmergency", field: "Revert Channel", kind_field: None, kind: None, what: "a digital emergency revert channel" },
    ChannelRef { file: "QEU_C_AEmergency", field: "Revert Channel", kind_field: None, kind: None, what: "an analog emergency revert channel" },
    ChannelRef { file: "QEU_C_Config", field: "Power On Channel", kind_field: None, kind: None, what: "the power-on channel" },
    ChannelRef { file: "QEU_C_Config", field: "Current CH Address", kind_field: None, kind: None, what: "the current channel" },
    ChannelRef { file: "QEU_DeveloperConfig", field: "Stun Channel List", kind_field: None, kind: None, what: "the stun channel" },
    ChannelRef { file: "QEU_C_DChannel", field: "RRS Channel", kind_field: None, kind: Some(0), what: "a digital channel's RRS channel" },
    ChannelRef { file: "QEU_C_DChannel", field: "Location Info Revert Channel", kind_field: None, kind: Some(0), what: "a digital channel's location revert channel" },
];

/// 0 and 0xFFFF mean "none" or "selected channel", not a channel number.
fn is_channel_number(n: i64) -> bool {
    n != 0 && n != 0xFFFF
}

/// When channels are removed, update references like CPS (observed 2026-11-18): zone members
/// pointing to a removed channel are dropped, later ones renumbered. Other references
/// ([`CHANNEL_REFS`], preset channels) make the edit fail if they point to an affected channel.
fn renumber_channel_references(saved: &SavedRead, layout: &Layout, edits: &mut Vec<DecodedFile>) -> Result<()> {
    for (list, kind) in CHANNEL_LISTS {
        let Some(map) = channel_renumbering(saved, edits, list)? else { continue };
        let first_affected = map.iter().enumerate().find(|(i, n)| **n != Some(*i as i64 + 1)).map_or(1, |(i, _)| i as i64 + 1);

        for r in CHANNEL_REFS {
            let Some(file) = current(saved, layout, edits, r.file)? else { continue };
            let maps = file.blocks.iter().flat_map(|b| &b.records).flat_map(|rec| {
                rec.fields.iter().chain(rec.members.iter().flatten()).collect::<Vec<_>>()
            });
            for f in maps {
                let [Some(n)] = member_values(f, &[r.field])[..] else { continue };
                let k = r.kind.or_else(|| r.kind_field.and_then(|kf| member_values(f, &[kf])[0]));
                if is_channel_number(n) && n >= first_affected && k.is_none_or(|k| k == kind) {
                    bail!(
                        "{list}: channel {n} is used as {}; removing channels that change its number is not supported yet",
                        r.what
                    );
                }
            }
        }
        if let Some(preset) = current(saved, layout, edits, "QEU_C_PresetChannel")? {
            let used = preset.blocks.iter().flat_map(|b| &b.records).filter_map(|r| r.fields.as_ref()).any(|f| {
                member_values(f, &["Zone Index", "Channel Index"]).iter().any(|v| v.is_some_and(|z| z != 0xFFFF))
            });
            if used {
                bail!("{list}: preset channels are set; removing channels while presets are used is not supported yet");
            }
        }

        // Zones: drop members of removed channels, renumber the rest (members refer to the
        // numbering of the read; channels appended in this edit continue it).
        let Some(mut zones) = current(saved, layout, edits, "QEU_C_Zone")? else { continue };
        for r in zones.blocks.iter_mut().flat_map(|b| b.records.iter_mut()) {
            if let Some(members) = &mut r.members {
                let mut kept = Vec::new();
                for mut m in members.drain(..) {
                    match member_values(&m, &["Channel List", "Channel Type"])[..] {
                        [Some(n), Some(k)] if k == kind && n >= 1 && (n as usize) <= map.len() => {
                            if let Some(new) = map[n as usize - 1] {
                                m.0.insert("Channel List".into(), Value::from(new));
                                kept.push(m);
                            }
                        }
                        _ => kept.push(m),
                    }
                }
                *members = kept;
            }
        }
        edits.retain(|e| e.id != zones.id);
        edits.push(zones);
    }
    Ok(())
}

/// Whether `actual` contains everything `expected` asks for: same record and member counts, and
/// every field given in `expected` has that value in `actual`.
fn realises(expected: &DecodedFile, actual: &DecodedFile) -> bool {
    let has = |e: &Fields, a: &Fields| e.0.iter().all(|(k, v)| a.0.get(k) == Some(v));
    expected.blocks.len() == actual.blocks.len()
        && expected.blocks.iter().zip(&actual.blocks).all(|(eb, ab)| {
            eb.records.len() == ab.records.len()
                && eb.records.iter().zip(&ab.records).all(|(er, ar)| match ((&er.fields, &er.members), (&ar.fields, &ar.members)) {
                    ((Some(e), _), (Some(a), _)) => has(e, a),
                    ((_, Some(em)), (_, Some(am))) => em.len() == am.len() && em.iter().zip(am).all(|(e, a)| has(e, a)),
                    _ => true,
                })
        })
}

/// Apply edited (possibly labelled) decoded files to a saved read. Returns all files (edited ones
/// re-encoded and prepared for writing) and the list of changes.
///
/// Records may be appended (channels start from CPS's template, other records from the RCDB
/// defaults); channels (digital or analog) may be removed, with zone members updated like CPS;
/// zone members may be added or removed; other record removals are refused. Changed numeric
/// values, and values given for new records, must be allowed by the option catalogue when it knows
/// the parameter. Linked lists (`TableLink.xml`) must keep equal lengths, and zone members must
/// point to existing channels.
pub fn apply_edits(
    saved: &SavedRead,
    layout: &Layout,
    options: Option<&Options>,
    links: &[TableLink],
    mut edits: Vec<DecodedFile>,
) -> Result<Edited> {
    for e in &mut edits {
        delabelize(e, layout, options)?;
    }
    renumber_channel_references(saved, layout, &mut edits)?;
    let mut files = saved.files.clone();
    let mut all_changes = Vec::new();
    for edited in edits {
        let id = edited.id;
        let original = saved.files.get(&id).with_context(|| format!("file {id} is not in the saved read"))?;
        let name = file_name(saved, id).unwrap_or(&edited.name).to_string();
        let before = decode(original, &name, layout)?;
        let file_changes = changes(&before, &edited)?;
        if file_changes.is_empty() {
            continue;
        }
        let checker = ValueCheck { layout, options, file: id, name: &name };
        let removable = CHANNEL_LISTS.iter().any(|(list, _)| *list == name);
        if ENCRYPT_KEY_FILES.contains(&name.as_str()) {
            check_encrypt_keys(saved, id, &name, &file_changes)?;
        }
        for c in &file_changes {
            match (c.key.as_str(), &c.new) {
                (REMOVED, _) if !c.position.contains('.') && !removable => {
                    bail!("file {id} ({name}) {}: removing these records is not supported yet (only channels)", c.position)
                }
                (REMOVED, _) => {}
                (ADDED, Value::Object(fields)) => {
                    for (key, v) in fields {
                        checker.check(c.block_id, &c.position, key, v)?;
                    }
                }
                (ADDED, Value::Array(members)) => {
                    for (key, v) in members.iter().filter_map(Value::as_object).flatten() {
                        checker.check(c.block_id, &c.position, key, v)?;
                    }
                }
                (ADDED, _) => {}
                (key, v) => checker.check(c.block_id, &c.position, key, v)?,
            }
        }
        let new_record = |b: &Block, defs: &[FieldDef]| -> hytera_codeplug::Result<Vec<u8>> {
            match template_record(saved, &name, b) {
                Some(t) => Ok(t),
                None => default_new_record(original, b, defs),
            }
        };
        let rebuilt =
            apply_with(original, &edited, layout, &new_record).with_context(|| format!("encoding file {id} ({name})"))?;
        let prepared = prepare_for_write(&rebuilt).with_context(|| format!("preparing file {id} ({name})"))?;
        // Sanity check: decoding the result shows what the edit asked for.
        let after = decode(&prepared, &name, layout)?;
        if !realises(&edited, &after) {
            bail!("file {id} ({name}): the re-encoded file does not match the edit (internal error)");
        }
        files.insert(id, prepared);
        all_changes.extend(file_changes);
    }
    if !all_changes.is_empty() {
        check_consistency(saved, &files, layout, links)?;
    }
    Ok((files, all_changes))
}

/// Files whose records have a digital channel's `Encrypt Key` (the channel list and CPS's
/// channel template, which new channels copy).
const ENCRYPT_KEY_FILES: [&str; 2] = ["QEU_C_DChannel", "QEU_C_Template_DChannel"];
/// The encryption key list (`QEU_C_E2EEKEYLIST`, 0 to 30 keys).
pub(crate) const KEY_LIST_FILE: &str = "QEU_C_E2EEKEYLIST";

/// A changed or added `Encrypt Key` must be 0 (None) or the number of a key in the key list
/// (inferred: 1-based record of [`KEY_LIST_FILE`]; open question 5e). The catalogue allows 1..30
/// whatever the list holds, so without this a channel could point at a key that does not exist.
/// Values the radio already had are not checked, so an unchanged configuration can be written back.
fn check_encrypt_keys(saved: &SavedRead, id: u16, name: &str, file_changes: &[Change]) -> Result<()> {
    let keys = file_id(saved, KEY_LIST_FILE).and_then(|f| saved.files.get(&f)).and_then(|b| record_count(b)).unwrap_or(0);
    for c in file_changes {
        let value = match (c.key.as_str(), &c.new) {
            ("Encrypt Key", v) => v.as_i64(),
            (ADDED, Value::Object(fields)) => fields.get("Encrypt Key").and_then(Value::as_i64),
            _ => None,
        };
        let Some(key) = value.filter(|k| *k != 0) else { continue };
        if key < 0 || key as usize > keys {
            let have = match keys {
                0 => "the radio's key list is empty".to_string(),
                1 => "the radio's key list has 1 key".to_string(),
                n => format!("the radio's key list has {n} keys"),
            };
            bail!("file {id} ({name}) {} Encrypt Key: key {key} does not exist ({have}); choose None or an existing key", c.position);
        }
    }
    Ok(())
}

fn record_count(bytes: &[u8]) -> Option<usize> {
    Some(structure::parse(bytes).ok()?.blocks.first()?.records.len())
}

/// Checks across files after an edit: linked lists keep equal lengths, and zone members point to
/// existing channels (member type 0 = digital, 1 = analog; channel numbers are 1-based).
fn check_consistency(saved: &SavedRead, files: &BTreeMap<u16, Vec<u8>>, layout: &Layout, links: &[TableLink]) -> Result<()> {
    for l in links {
        let (Some(two), Some(one)) = (files.get(&l.two_dim), files.get(&l.one_dim)) else { continue };
        let (a, b) = (record_count(two), record_count(one));
        if a != b {
            bail!(
                "{} has {} records but its partner {} has {}; add or remove both together",
                file_name(saved, l.two_dim).unwrap_or("?"),
                a.unwrap_or(0),
                file_name(saved, l.one_dim).unwrap_or("?"),
                b.unwrap_or(0)
            );
        }
    }
    // CPS requires contacts to be unique by (Call ID, Call Type) (observed in its validation config,
    // docs/cps-features.md). Only a change can introduce a duplicate, so check whenever the file
    // differs from the read.
    if let Some(id) = file_id(saved, "QEU_C_DContact")
        && let Some(bytes) = files.get(&id)
        && saved.files.get(&id) != Some(bytes)
    {
        let contacts = decode(bytes, "QEU_C_DContact", layout)?;
        let mut seen: BTreeMap<(i64, i64), usize> = BTreeMap::new();
        for (i, r) in contacts.blocks.iter().flat_map(|b| &b.records).enumerate() {
            let Some(f) = &r.fields else { continue };
            let get = |k: &str| f.0.get(k).and_then(Value::as_i64);
            let (Some(call_id), Some(kind)) = (get("Call ID"), get("Call Type")) else { continue };
            if let Some(first) = seen.insert((call_id, kind), i + 1) {
                bail!("contacts {first} and {} have the same call ID {call_id} and call type; CPS does not allow that", i + 1);
            }
        }
    }
    let count_of = |name: &str| file_id(saved, name).and_then(|id| files.get(&id)).and_then(|b| record_count(b));
    let (Some(zone_id), Some(digital), Some(analog)) =
        (file_id(saved, "QEU_C_Zone"), count_of("QEU_C_DChannel"), count_of("QEU_C_AChannel"))
    else {
        return Ok(());
    };
    let zones = decode(&files[&zone_id], "QEU_C_Zone", layout)?;
    for r in zones.blocks.iter().flat_map(|b| &b.records) {
        for (m, member) in r.members.iter().flatten().enumerate() {
            let get = |k: &str| member.0.get(k).and_then(Value::as_i64);
            let (Some(channel), Some(kind)) = (get("Channel List"), get("Channel Type")) else { continue };
            let limit = match kind {
                0 => digital,
                1 => analog,
                _ => continue,
            };
            if channel < 1 || channel as usize > limit {
                bail!(
                    "zone {} member {}: {} channel {channel} does not exist (there are {limit})",
                    r.index.unwrap_or(0) + 1,
                    m + 1,
                    if kind == 0 { "digital" } else { "analog" }
                );
            }
        }
    }
    Ok(())
}
