//! Field layouts from `RcdbJsons\<version>.txt` (shipped with CPS). See `docs/file-format.md` §2.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::rcdb::RcdbRoot;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Byte,
    /// One-byte number (used for levels, timeouts, flags).
    Char,
    Short,
    Ushort,
    Int,
    Uint,
    Float,
    /// `length` bits at `bit_start` inside a `bit_container`-byte little-endian word.
    Bit,
    AsciiString,
    UnicodeString,
}

impl FieldType {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "Byte" => FieldType::Byte,
            "Char" => FieldType::Char,
            "Short" => FieldType::Short,
            "Ushort" => FieldType::Ushort,
            "Int" => FieldType::Int,
            "Uint" => FieldType::Uint,
            "Float" => FieldType::Float,
            "Bit" => FieldType::Bit,
            "AsciiString" => FieldType::AsciiString,
            "UnicodeString" => FieldType::UnicodeString,
            _ => return None,
        })
    }

    /// Bytes per element (not meaningful for `Bit`).
    pub fn element_size(self) -> usize {
        match self {
            FieldType::Byte | FieldType::Char | FieldType::AsciiString | FieldType::Bit => 1,
            FieldType::Short | FieldType::Ushort | FieldType::UnicodeString => 2,
            FieldType::Int | FieldType::Uint | FieldType::Float => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub id: u32,
    pub name: String,
    pub file_id: u16,
    pub block_id: u16,
    /// 0 = single record, 1 = list, 2 = list whose records are member lists.
    pub block_type: u8,
    /// Byte offset inside the record (or member, for block type 2).
    pub offset: usize,
    pub kind: FieldType,
    /// Element count; characters for strings; bit width for `Bit`.
    pub length: usize,
    pub bit_start: Option<u32>,
    /// Size in bytes of the word holding a `Bit` field.
    pub bit_container: usize,
    pub default_en: String,
}

impl FieldDef {
    /// Bytes the field occupies, starting at `offset`.
    pub fn byte_len(&self) -> usize {
        match self.kind {
            FieldType::Bit => self.bit_container,
            k => k.element_size() * self.length,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawField {
    name: String,
    id: u32,
    length: i64,
    #[serde(rename = "Type")]
    kind: String,
    #[serde(default)]
    default_value_en: String,
    #[serde(rename = "File_Id")]
    file_id: u16,
    #[serde(rename = "Block_Id")]
    block_id: u16,
    #[serde(rename = "Block_Type")]
    block_type: u8,
    offset: i64,
    #[serde(rename = "Bit_Field_Len")]
    bit_field_len: i64,
    #[serde(rename = "Bit_Start_Pos")]
    bit_start_pos: i64,
}

#[derive(Deserialize)]
struct RawLayout {
    detail: Vec<RawField>,
}

/// All field definitions of one RCDB version, grouped by (file, block) and sorted by offset.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub version: String,
    blocks: BTreeMap<(u16, u16), Vec<FieldDef>>,
}

impl Layout {
    pub fn parse(version: &str, json: &str) -> Result<Self> {
        let raw: RawLayout = serde_json::from_str(json.trim_start_matches('\u{FEFF}'))
            .map_err(|e| Error::Invalid(format!("layout JSON: {e}")))?;
        let mut blocks: BTreeMap<(u16, u16), Vec<FieldDef>> = BTreeMap::new();
        for f in raw.detail {
            let kind = FieldType::parse(&f.kind)
                .ok_or_else(|| Error::Invalid(format!("field {} ({}): unknown type {}", f.id, f.name, f.kind)))?;
            let (Ok(offset), Ok(length)) = (usize::try_from(f.offset), usize::try_from(f.length)) else {
                return Err(Error::Invalid(format!("field {} ({}): negative offset or length", f.id, f.name)));
            };
            let bit_start = (kind == FieldType::Bit).then(|| f.bit_start_pos.max(0) as u32);
            let bit_container = if kind == FieldType::Bit { f.bit_field_len.clamp(1, 4) as usize } else { 0 };
            if let Some(start) = bit_start
                && start as usize + length > bit_container * 8
            {
                return Err(Error::Invalid(format!("field {} ({}): bits do not fit their word", f.id, f.name)));
            }
            blocks.entry((f.file_id, f.block_id)).or_default().push(FieldDef {
                id: f.id,
                name: f.name,
                file_id: f.file_id,
                block_id: f.block_id,
                block_type: f.block_type,
                offset,
                kind,
                length,
                bit_start,
                bit_container,
                default_en: f.default_value_en,
            });
        }
        for fields in blocks.values_mut() {
            fields.sort_by_key(|f| (f.offset, f.bit_start.unwrap_or(0), f.id));
        }
        Ok(Layout { version: version.to_string(), blocks })
    }

    /// Fields of one block, sorted by offset (empty if the block is unknown).
    pub fn fields(&self, file_id: u16, block_id: u16) -> &[FieldDef] {
        self.blocks.get(&(file_id, block_id)).map_or(&[], Vec::as_slice)
    }

    /// Every definition with layout id `id` (normally one).
    pub fn find(&self, id: u32) -> Vec<&FieldDef> {
        self.blocks.values().flatten().filter(|f| f.id == id).collect()
    }

    pub fn block_ids(&self, file_id: u16) -> Vec<u16> {
        self.blocks.keys().filter(|(f, _)| *f == file_id).map(|&(_, b)| b).collect()
    }
}

impl RcdbRoot {
    /// Field layout for `version` from `RcdbJsons\<version>.txt`.
    pub fn layout(&self, version: &str) -> Result<Layout> {
        let path: PathBuf = self.path().join("RcdbJsons").join(format!("{version}.txt"));
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io { path: path.clone(), source })?;
        Layout::parse(version, &text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
    }
}
