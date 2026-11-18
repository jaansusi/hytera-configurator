//! Human-readable views of decoded settings, labelled with CPS's option names when available.

use std::fmt::Write as _;

use hytera_codeplug::decode::{DecodedFile, Record, keys};
use hytera_codeplug::layout::{FieldDef, Layout};
use hytera_codeplug::options::Options;
use serde_json::{Map, Value};

use crate::ReadManifest;

/// Field names whose values are never shown (the radio's programming passwords).
const SECRET_FIELDS: &[&str] = &["Write Password", "Read Possword", "Read Password"];

/// Whether field `key` holds a programming password, which is never shown or edited.
pub fn is_secret(key: &str) -> bool {
    SECRET_FIELDS.contains(&key)
}

/// Turns raw field values into the text CPS would show.
pub struct Labeler<'a> {
    pub layout: &'a Layout,
    pub options: Option<&'a Options>,
}

impl Labeler<'_> {
    fn def(&self, file: u16, block: u16, key: &str) -> Option<&FieldDef> {
        let defs = self.layout.fields(file, block);
        keys(defs).iter().position(|k| k == key).map(|i| &defs[i])
    }

    /// Display text for one field value.
    pub fn text(&self, file: u16, block: u16, key: &str, value: &Value) -> String {
        if is_secret(key) {
            return "(hidden)".into();
        }
        match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => {
                let label = n.as_i64().zip(self.def(file, block, key)).and_then(|(raw, def)| {
                    self.options.and_then(|o| o.display(def.id, raw))
                });
                label.unwrap_or_else(|| n.to_string())
            }
            other => other.to_string(),
        }
    }
}

fn first_block(decoded: &[DecodedFile], file: u16) -> Option<(u16, &[Record])> {
    let f = decoded.iter().find(|f| f.id == file)?;
    let b = f.blocks.first()?;
    Some((b.id, b.records.as_slice()))
}

fn block_record(decoded: &[DecodedFile], file: u16, block: u16) -> Option<&Map<String, Value>> {
    let f = decoded.iter().find(|f| f.id == file)?;
    let r = f.blocks.iter().find(|b| b.id == block)?.records.first()?;
    r.fields.as_ref().map(|f| &f.0)
}

/// Overview of the main settings (BP515 / RCDB V1.5.24.000 file and field names).
pub fn summary(manifest: &ReadManifest, decoded: &[DecodedFile], labels: &Labeler) -> String {
    let get = |file: u16, block: u16, fields: &Map<String, Value>, key: &str| {
        fields.get(key).map_or("?".into(), |v| labels.text(file, block, key, v))
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        "Radio: {} serial {} firmware {} RCDB {}",
        manifest.model_name.as_deref().unwrap_or("?"),
        manifest.serial_number.as_deref().unwrap_or("?"),
        manifest.firmware_version.as_deref().unwrap_or("?"),
        manifest.rcdb_version
    );
    let alias = block_record(decoded, 18, 612).map_or("?".into(), |f| get(18, 612, f, "Radio Alias"));
    let (date, band) = block_record(decoded, 18, 902)
        .map_or(("?".into(), "?".into()), |f| (get(18, 902, f, "Programming Time"), get(18, 902, f, "Frequency Range")));
    let _ = writeln!(s, "Alias: {alias}   Last programmed: {date}   Band: {band} MHz");
    if labels.options.is_none() {
        let _ = writeln!(s, "(No option catalogue loaded: values are raw. Run tools/export-rcdb-options.ps1.)");
    }

    if let Some((block, records)) = first_block(decoded, 3) {
        let _ = writeln!(s, "\nDigital channels:");
        for r in records {
            let Some(f) = &r.fields else { continue };
            let g = |k: &str| get(3, block, &f.0, k);
            let _ = writeln!(
                s,
                "  {:>3}  {:<16} RX {}  TX {}  colour code {}  {}  power {}  TX call {} ({})",
                r.index.unwrap_or(0) + 1,
                g("Channel Alias"),
                g("RX Frequency"),
                g("TX Frequency"),
                g("Color Code"),
                g("Slot Operation"),
                g("Power Level"),
                g("TX Contacts ID"),
                g("Call Type")
            );
        }
    }
    if let Some((block, records)) = first_block(decoded, 9) {
        let _ = writeln!(s, "\nAnalog channels:");
        for r in records {
            let Some(f) = &r.fields else { continue };
            let g = |k: &str| get(9, block, &f.0, k);
            let _ = writeln!(
                s,
                "  {:>3}  {:<16} RX {}  TX {}  spacing {} kHz  power {}  RX tone {} {}  TX tone {} {}",
                r.index.unwrap_or(0) + 1,
                g("Channel Alias"),
                g("RX Frequency"),
                g("TX Frequency"),
                g("Channel Spacing"),
                g("Power Level"),
                g("RX CTCorCDC Type"),
                g("RX CTCorCDC Value"),
                g("TX CTCorCDC Type"),
                g("TX CTCorCDC Value")
            );
        }
    }
    if let (Some((zblock, zones)), Some((ablock, aliases))) = (first_block(decoded, 6), first_block(decoded, 7)) {
        let _ = writeln!(s, "\nZones:");
        for (zone, alias) in zones.iter().zip(aliases) {
            let name = alias.fields.as_ref().map_or("?".into(), |f| get(7, ablock, &f.0, "Zone Alias"));
            let members: Vec<String> = zone
                .members
                .iter()
                .flatten()
                .map(|m| format!("{} {}", get(6, zblock, &m.0, "Channel Type"), get(6, zblock, &m.0, "Channel List")))
                .collect();
            let _ = writeln!(s, "  {:>3}  {:<16} {}", zone.index.unwrap_or(0) + 1, name, members.join(", "));
        }
    }
    if let Some((block, records)) = first_block(decoded, 8) {
        let _ = writeln!(s, "\nContacts:");
        for r in records {
            let Some(f) = &r.fields else { continue };
            let g = |k: &str| get(8, block, &f.0, k);
            let _ = writeln!(s, "  {:>3}  {:<16} ID {}  {}", r.index.unwrap_or(0) + 1, g("Call Alias"), g("Call ID"), g("Call Type"));
        }
    }
    s
}

/// Every field of every file, one line each, labelled.
pub fn full_listing(decoded: &[DecodedFile], labels: &Labeler) -> String {
    let mut s = String::new();
    for f in decoded {
        let _ = writeln!(s, "\n=== file {} {}", f.id, f.name);
        for b in &f.blocks {
            let _ = writeln!(s, "  -- block {} {:?} ({} of {} records)", b.id, b.short_name, b.records.len(), b.capacity);
            for r in &b.records {
                let lists: Vec<(String, &Map<String, Value>)> = match (&r.fields, &r.members) {
                    (Some(fields), _) => vec![(format!("[{}]", r.index.unwrap_or(0) + 1), &fields.0)],
                    (None, Some(members)) => members
                        .iter()
                        .enumerate()
                        .map(|(m, fields)| (format!("[{}.{}]", r.index.unwrap_or(0) + 1, m + 1), &fields.0))
                        .collect(),
                    (None, None) => Vec::new(),
                };
                for (prefix, fields) in lists {
                    for (key, value) in fields {
                        let shown = labels.text(f.id, b.id, key, value);
                        let raw = value.to_string();
                        let raw_note = if value.is_number() && shown != raw { format!("  (raw {raw})") } else { String::new() };
                        let _ = writeln!(s, "    {prefix} {key} = {shown}{raw_note}");
                    }
                }
            }
        }
    }
    s
}
