//! Decode whole files into named fields, and apply decoded values back onto the original bytes.

use serde::{Deserialize, Serialize};

use crate::layout::{FieldDef, Layout};
use crate::structure::{self, Block, empty_sublist_like, parse_sublist, rebuild_sublist};
use crate::value::{self, Value};
use crate::{Error, Result};

/// Field values of one record (or one member of a two-level record), in layout order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fields(pub serde_json::Map<String, serde_json::Value>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Position of this record in the file it was decoded from (0-based). In an edit, it says which
    /// original record an entry is; entries without it are new records. Records left out of an
    /// edit are removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    /// Fields of a normal record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Fields>,
    /// Members of a two-level record (e.g. the channels of a zone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<Fields>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedBlock {
    pub id: u16,
    pub short_name: String,
    pub two_dim: bool,
    pub capacity: u16,
    pub records: Vec<Record>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodedFile {
    pub id: u16,
    pub name: String,
    pub blocks: Vec<DecodedBlock>,
}

/// Map keys: field names, made unique within a record by appending ` #2`, ` #3`, ….
pub fn keys(fields: &[FieldDef]) -> Vec<String> {
    let mut seen = std::collections::HashMap::<&str, usize>::new();
    fields
        .iter()
        .map(|f| {
            let n = seen.entry(f.name.as_str()).or_insert(0);
            *n += 1;
            if *n == 1 { f.name.clone() } else { format!("{} #{n}", f.name) }
        })
        .collect()
}

fn read_fields(bytes: &[u8], defs: &[FieldDef]) -> Result<Fields> {
    let mut map = serde_json::Map::new();
    for (key, f) in keys(defs).into_iter().zip(defs) {
        let v = value::read(bytes, f)?;
        map.insert(key, serde_json::to_value(v).expect("values serialise"));
    }
    Ok(Fields(map))
}

fn write_fields(bytes: &mut [u8], defs: &[FieldDef], fields: &Fields) -> Result<()> {
    for (key, f) in keys(defs).into_iter().zip(defs) {
        let Some(json) = fields.0.get(&key) else { continue };
        let v: Value = serde_json::from_value(json.clone())
            .map_err(|e| Error::Invalid(format!("field {key}: {e}")))?;
        value::write(bytes, f, &v)?;
    }
    Ok(())
}

/// Decode `file` (as read from the radio) with `layout`.
pub fn decode(file: &[u8], name: &str, layout: &Layout) -> Result<DecodedFile> {
    let header = crate::container::FileHeader::parse(file)
        .ok_or_else(|| Error::Invalid(format!("{name}: shorter than a file header")))?;
    let id = header.file_id();
    let s = structure::parse(file).map_err(|e| Error::Invalid(format!("{name}: {e}")))?;
    let mut blocks = Vec::new();
    for b in &s.blocks {
        let defs = layout.fields(id, b.id);
        let mut records = Vec::new();
        for (index, range) in b.records.iter().enumerate() {
            let bytes = &file[range.clone()];
            records.push(if b.two_dim {
                let sub = parse_sublist(bytes).map_err(|e| Error::Invalid(format!("{name} block {}: {e}", b.id)))?;
                let members = sub.members.iter().map(|m| read_fields(&bytes[m.clone()], defs)).collect::<Result<_>>()?;
                Record { index: Some(index), fields: None, members: Some(members) }
            } else {
                Record { index: Some(index), fields: Some(read_fields(bytes, defs)?), members: None }
            });
        }
        blocks.push(DecodedBlock {
            id: b.id,
            short_name: b.short_name.clone(),
            two_dim: b.two_dim,
            capacity: b.capacity,
            records,
        });
    }
    Ok(DecodedFile { id, name: name.to_string(), blocks })
}

/// Write the field values of `decoded` into a copy of `original`. Bytes not covered by any field
/// are kept. Records may be removed (left out; later ones shift down) or appended (no `index`;
/// they start from the RCDB defaults, or for two-level blocks from an empty member list shaped like
/// the block's first record), and members may be added or removed. References in other files are
/// not updated here. See [`apply_with`].
pub fn apply(original: &[u8], decoded: &DecodedFile, layout: &Layout) -> Result<Vec<u8>> {
    apply_with(original, decoded, layout, &|b: &Block, defs: &[FieldDef]| default_new_record(original, b, defs))
}

/// The starting bytes of a record appended to block `b`: an empty member list for two-level
/// blocks (shaped like the first existing record), otherwise the RCDB defaults.
pub fn default_new_record(original: &[u8], b: &Block, defs: &[FieldDef]) -> Result<Vec<u8>> {
    if b.two_dim {
        let first = b.records.first().ok_or_else(|| {
            Error::Invalid(format!("block {} has no record to copy the member-list layout from", b.id))
        })?;
        empty_sublist_like(&original[first.clone()])
    } else {
        value::default_record(defs, b.record_size)
    }
}

/// Creates the starting bytes of a record appended to a block (given the block and its fields).
pub type NewRecordFn<'a> = dyn Fn(&Block, &[FieldDef]) -> Result<Vec<u8>> + 'a;

/// [`apply`] with a caller-chosen way to create appended records (e.g. from CPS's channel
/// template). Files whose record counts change are rebuilt ([`structure::rebuild`], which drops
/// any stale tail); all others are patched in place.
pub fn apply_with(original: &[u8], decoded: &DecodedFile, layout: &Layout, new_record: &NewRecordFn) -> Result<Vec<u8>> {
    let s = structure::parse(original)?;
    if s.blocks.len() != decoded.blocks.len() {
        return Err(Error::Invalid(format!("{}: expected {} blocks, got {}", decoded.name, s.blocks.len(), decoded.blocks.len())));
    }
    let mut all_records: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut counts_changed = false;
    for (b, db) in s.blocks.iter().zip(&decoded.blocks) {
        if b.id != db.id {
            return Err(Error::Invalid(format!("{}: expected block {}, got {}", decoded.name, b.id, db.id)));
        }
        // Which original record each entry comes from: existing records keep their order, new
        // ones (no index) go after them; original records not listed are removed.
        let mut sources = Vec::new();
        for rec in &db.records {
            let source = rec.index.filter(|&i| i < b.records.len());
            match (sources.last().copied().flatten(), source) {
                (Some(prev), Some(i)) if i <= prev => {
                    return Err(Error::Invalid(format!(
                        "{}: block {}: records must keep their order (record {} after {}); moving records is not supported",
                        decoded.name,
                        b.id,
                        i + 1,
                        prev + 1
                    )));
                }
                _ if source.is_some() && sources.iter().any(Option::is_none) => {
                    return Err(Error::Invalid(format!("{}: block {}: new records must come after existing ones", decoded.name, b.id)));
                }
                _ => {}
            }
            sources.push(source);
        }
        counts_changed |= sources.len() != b.records.len() || sources.iter().enumerate().any(|(i, s)| *s != Some(i));
        let defs = layout.fields(decoded.id, b.id);
        let mut records = Vec::new();
        for (i, rec) in db.records.iter().enumerate() {
            let mut bytes = match sources[i] {
                Some(src) => original[b.records[src].clone()].to_vec(),
                None => new_record(b, defs)?,
            };
            match (&rec.fields, &rec.members) {
                (Some(fields), _) => write_fields(&mut bytes, defs, fields)?,
                (None, Some(members)) => {
                    let sub = parse_sublist(&bytes)
                        .map_err(|e| Error::Invalid(format!("{}: block {} record {}: {e}", decoded.name, b.id, i + 1)))?;
                    let mut slots = Vec::new();
                    for (j, fields) in members.iter().enumerate() {
                        let mut slot = match sub.members.get(j) {
                            Some(range) => bytes[range.clone()].to_vec(),
                            None => value::default_record(defs, sub.member_size)?,
                        };
                        write_fields(&mut slot, defs, fields)?;
                        slots.push(slot);
                    }
                    bytes = rebuild_sublist(&bytes, &slots)
                        .map_err(|e| Error::Invalid(format!("{}: block {} record {}: {e}", decoded.name, b.id, i + 1)))?;
                }
                (None, None) => {}
            }
            records.push(bytes);
        }
        all_records.push(records);
    }
    if counts_changed {
        return structure::rebuild(original, &s, &all_records);
    }
    let mut out = original.to_vec();
    for (b, records) in s.blocks.iter().zip(&all_records) {
        for (range, bytes) in b.records.iter().zip(records) {
            out[range.clone()].copy_from_slice(bytes);
        }
    }
    Ok(out)
}
