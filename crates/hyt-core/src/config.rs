//! Configurations as data, made into any radio's files.
//!
//! A configuration is the value of every setting (decoded with the layout of the radio it came
//! from; see [`crate::ConfigurationFile`]). To program a radio, its own files (as just read) are
//! rebuilt from that data with [`overlay`], using the radio's own layout. So a configuration is not
//! tied to one firmware: settings are matched by CPS's parameter id, which stays the same across
//! RCDB versions (observed for V1.5.08.000 and V1.5.24.000; see docs/file-format.md).
//!
//! Every byte of every record belongs to a field (decode round-trip tests), so for a radio with
//! the configuration's own layout the result holds exactly the configuration's values. Bytes
//! outside records (file header, block headers, unused member slots of two-level lists) are the
//! radio's own.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use hytera_codeplug::decode::{DecodedBlock, DecodedFile, Fields, Record, apply_with, decode, default_new_record, keys};
use hytera_codeplug::layout::{FieldDef, Layout};
use hytera_codeplug::structure::Block;
use hytera_codeplug::value::{self, Value};
use serde::Serialize;

use crate::SavedRead;

/// What could not be carried over exactly. Empty when the radio has the configuration's layout.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct OverlayReport {
    /// RCDB layout of the configuration and of the radio.
    pub from_layout: String,
    pub to_layout: String,
    /// Files of the configuration this radio does not have (left out).
    pub files_left_out: Vec<String>,
    /// Files of this radio the configuration does not have (the radio's own contents kept).
    pub files_kept: Vec<String>,
    /// Settings this radio's firmware does not have (left out), as `file: names`.
    pub settings_left_out: Vec<String>,
    /// Settings only this radio's firmware has (its own value, or the default for new records, kept).
    pub settings_kept: Vec<String>,
    /// Settings whose value does not fit this radio's field (its own value kept).
    pub not_converted: Vec<String>,
}

impl OverlayReport {
    /// Whether the configuration was made for another layout.
    pub fn converted(&self) -> bool {
        self.from_layout != self.to_layout
    }

    pub fn is_exact(&self) -> bool {
        self.files_left_out.is_empty()
            && self.files_kept.is_empty()
            && self.settings_left_out.is_empty()
            && self.settings_kept.is_empty()
            && self.not_converted.is_empty()
    }

    /// One line per item, for showing to the user.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.converted() {
            out.push(format!(
                "converted from RCDB {} (the configuration) to RCDB {} (this radio's firmware)",
                self.from_layout, self.to_layout
            ));
        }
        let mut list = |title: &str, items: &[String]| {
            if !items.is_empty() {
                out.push(format!("{title}: {}", items.join("; ")));
            }
        };
        list("not on this radio, left out", &self.files_left_out);
        list("not in the configuration, this radio's own kept", &self.files_kept);
        list("settings this radio's firmware does not have, left out", &self.settings_left_out);
        list("settings only this radio's firmware has, its own kept", &self.settings_kept);
        list("values that do not fit this radio, its own kept", &self.not_converted);
        out
    }
}

/// The radio's files rebuilt from a configuration, and what could not be carried over.
pub struct Overlay {
    pub files: BTreeMap<u16, Vec<u8>>,
    pub report: OverlayReport,
}

/// Rebuild `radio`'s files (decoded with `radio_layout`) from the configuration `config` (decoded
/// with `config_layout`). Files in `own` stay the radio's as they are (e.g. files CPS never writes).
///
/// Per file and block: the configuration's records in order (more or fewer than the radio has;
/// new records start from the radio's channel template or the RCDB defaults, like CPS), each field
/// set from the configuration's field with the same parameter id. Settings the radio has and the
/// configuration does not keep the radio's value. Fails when the radio cannot hold the
/// configuration's records.
pub fn overlay(
    config: &[DecodedFile],
    config_layout: &Layout,
    radio: &SavedRead,
    radio_layout: &Layout,
    own: &[u16],
) -> Result<Overlay> {
    let mut report = OverlayReport {
        from_layout: config_layout.version.clone(),
        to_layout: radio_layout.version.clone(),
        ..Default::default()
    };
    let mut files = radio.files.clone();
    for entry in &radio.manifest.files {
        if own.contains(&entry.id) {
            continue;
        }
        let Some(source) = config.iter().find(|f| f.id == entry.id) else {
            report.files_kept.push(format!("{} {}", entry.id, entry.name));
            continue;
        };
        if source.name != entry.name {
            bail!("file {} is {} in the configuration but {} on this radio", entry.id, source.name, entry.name);
        }
        let bytes = radio.files.get(&entry.id).with_context(|| format!("file {} ({}) missing", entry.id, entry.name))?;
        let current = decode(bytes, &entry.name, radio_layout).with_context(|| format!("decoding {}", entry.name))?;
        let mapped = map_file(source, config_layout, &current, radio_layout, &mut report)?;
        let new_record = |b: &Block, defs: &[FieldDef]| match crate::edit::template_record(radio, &entry.name, b) {
            Some(t) => Ok(t),
            None => default_new_record(bytes, b, defs),
        };
        let rebuilt = apply_with(bytes, &mapped, radio_layout, &new_record)
            .with_context(|| format!("rebuilding file {} ({})", entry.id, entry.name))?;
        files.insert(entry.id, rebuilt);
    }
    for f in config {
        if !radio.manifest.files.iter().any(|e| e.id == f.id) {
            report.files_left_out.push(format!("{} {}", f.id, f.name));
        }
    }
    Ok(Overlay { files, report })
}

/// A field of the radio's layout, and the configuration's field (key and definition) it takes its
/// value from, when the configuration's layout has one.
struct Target<'a> {
    key: String,
    from: Option<(String, &'a FieldDef)>,
    to: &'a FieldDef,
}

/// Where each field of the radio's layout takes its value from in the configuration's layout:
/// the configuration's field with the same parameter id (the n-th of that id for the n-th).
struct FieldMap<'a> {
    targets: Vec<Target<'a>>,
}

impl<'a> FieldMap<'a> {
    fn new(from: &'a [FieldDef], to: &'a [FieldDef]) -> (Self, Vec<&'a str>, Vec<&'a str>) {
        let numbered = |defs: &'a [FieldDef]| -> Vec<((u32, usize), String, &'a FieldDef)> {
            let mut seen = HashMap::<u32, usize>::new();
            keys(defs)
                .into_iter()
                .zip(defs)
                .map(|(k, d)| {
                    let n = seen.entry(d.id).or_insert(0);
                    *n += 1;
                    ((d.id, *n), k, d)
                })
                .collect()
        };
        let (src, dst) = (numbered(from), numbered(to));
        let by_id: HashMap<(u32, usize), (String, &FieldDef)> = src.iter().map(|(n, k, d)| (*n, (k.clone(), *d))).collect();
        let left_out = src.iter().filter(|(n, _, _)| !dst.iter().any(|(m, _, _)| m == n)).map(|(_, _, d)| d.name.as_str()).collect();
        let kept = dst.iter().filter(|(n, _, _)| !by_id.contains_key(n)).map(|(_, _, d)| d.name.as_str()).collect();
        let targets = dst.into_iter().map(|(n, key, to)| Target { key, from: by_id.get(&n).cloned(), to }).collect();
        (FieldMap { targets }, left_out, kept)
    }

    /// The radio's fields for a record: the configuration's values where they fit, else `base`'s.
    fn fields(&self, source: &Fields, base: Option<&Fields>, not_converted: &mut Vec<String>) -> Fields {
        let mut out = serde_json::Map::new();
        for Target { key, from, to } in &self.targets {
            let taken = from.as_ref().and_then(|(k, d)| source.0.get(k).map(|v| (v, *d)));
            match taken {
                Some((v, d)) if fits(d, to, v) => {
                    out.insert(key.clone(), v.clone());
                }
                other => {
                    if other.is_some() {
                        not_converted.push(to.name.clone());
                    }
                    if let Some(v) = base.and_then(|b| b.0.get(key)) {
                        out.insert(key.clone(), v.clone());
                    }
                }
            }
        }
        Fields(out)
    }
}

/// Whether value `v` of field `from` can be stored in field `to` unchanged.
fn fits(from: &FieldDef, to: &FieldDef, v: &serde_json::Value) -> bool {
    if from.kind == to.kind && from.length == to.length && from.bit_container == to.bit_container {
        return true;
    }
    let Ok(parsed) = serde_json::from_value::<Value>(v.clone()) else { return false };
    let mut scratch = vec![0u8; to.offset + to.byte_len()];
    value::write(&mut scratch, to, &parsed).is_ok()
}

fn map_file(
    source: &DecodedFile,
    from: &Layout,
    current: &DecodedFile,
    to: &Layout,
    report: &mut OverlayReport,
) -> Result<DecodedFile> {
    let mut blocks = Vec::new();
    for tb in &current.blocks {
        let Some(sb) = source.blocks.iter().find(|b| b.id == tb.id) else {
            if !tb.records.is_empty() {
                report.settings_kept.push(format!("{}: block {} ({})", current.name, tb.id, tb.short_name.trim()));
            }
            blocks.push(tb.clone());
            continue;
        };
        if sb.records.len() > usize::from(tb.capacity) {
            bail!(
                "{}: this radio holds at most {} records in block {} ({}), the configuration has {}",
                current.name,
                tb.capacity,
                tb.id,
                tb.short_name.trim(),
                sb.records.len()
            );
        }
        let (map, left_out, kept) = FieldMap::new(from.fields(source.id, sb.id), to.fields(current.id, tb.id));
        let place = format!("{} ({})", current.name, tb.short_name.trim());
        let used = !sb.records.is_empty() || !tb.records.is_empty();
        if used && !left_out.is_empty() {
            report.settings_left_out.push(format!("{place}: {}", summary(&left_out)));
        }
        if used && !kept.is_empty() {
            report.settings_kept.push(format!("{place}: {}", summary(&kept)));
        }
        let mut not_converted = Vec::new();
        let mut records = Vec::new();
        for (i, sr) in sb.records.iter().enumerate() {
            let base = tb.records.get(i);
            let index = base.map(|_| i);
            let record = match (&sr.fields, &sr.members) {
                (Some(f), _) => Record {
                    index,
                    fields: Some(map.fields(f, base.and_then(|b| b.fields.as_ref()), &mut not_converted)),
                    members: None,
                },
                (None, Some(members)) => {
                    let base_members = base.and_then(|b| b.members.as_ref());
                    let members = members
                        .iter()
                        .enumerate()
                        .map(|(j, m)| map.fields(m, base_members.and_then(|bm| bm.get(j)), &mut not_converted))
                        .collect();
                    Record { index, fields: None, members: Some(members) }
                }
                (None, None) => Record { index, fields: None, members: None },
            };
            records.push(record);
        }
        not_converted.sort();
        not_converted.dedup();
        if !not_converted.is_empty() {
            let names: Vec<&str> = not_converted.iter().map(String::as_str).collect();
            report.not_converted.push(format!("{place}: {}", summary(&names)));
        }
        blocks.push(DecodedBlock { records, ..tb.clone() });
    }
    Ok(DecodedFile { id: current.id, name: current.name.clone(), blocks })
}

/// `a, b, c (+N more)`, without reserved fields' names.
fn summary(names: &[&str]) -> String {
    let mut shown: Vec<&str> = names.iter().copied().filter(|n| !n.to_ascii_lowercase().starts_with("reserve")).collect();
    shown.dedup();
    let hidden = names.len() - shown.len();
    const MAX: usize = 6;
    let more = shown.len().saturating_sub(MAX) + hidden;
    shown.truncate(MAX);
    let mut s = shown.join(", ");
    if more > 0 {
        if !s.is_empty() {
            s.push_str(", ");
        }
        s.push_str(&format!("+{more} more"));
    }
    s
}
