//! Reading and writing one field inside a record. Decoding is lossless: writing a decoded value
//! back leaves the record byte-identical.

use serde::{Deserialize, Serialize};

use crate::layout::{FieldDef, FieldType};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Number(i64),
    Float(f32),
    Text(String),
    List(Vec<Value>),
    /// Bytes that are not clean text (e.g. a string with data after its terminator).
    Raw { hex: String },
}

fn err(f: &FieldDef, msg: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("field {} ({}): {msg}", f.id, f.name))
}

fn slice<'a>(record: &'a [u8], f: &FieldDef) -> Result<&'a [u8]> {
    record
        .get(f.offset..f.offset + f.byte_len())
        .ok_or_else(|| err(f, format!("needs bytes {}..{}, record has {}", f.offset, f.offset + f.byte_len(), record.len())))
}

fn le(bytes: &[u8]) -> u64 {
    bytes.iter().rev().fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
}

fn number(kind: FieldType, b: &[u8]) -> Value {
    match kind {
        FieldType::Short => Value::Number(i64::from(i16::from_le_bytes([b[0], b[1]]))),
        FieldType::Int => Value::Number(i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))),
        FieldType::Float => Value::Float(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        _ => Value::Number(le(b) as i64),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Decode field `f` from `record`.
pub fn read(record: &[u8], f: &FieldDef) -> Result<Value> {
    let b = slice(record, f)?;
    Ok(match f.kind {
        FieldType::Bit => {
            let word = le(b);
            let start = f.bit_start.unwrap_or(0);
            let mask = (1u64 << f.length) - 1;
            Value::Number(((word >> start) & mask) as i64)
        }
        FieldType::UnicodeString => {
            let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            match String::from_utf16(&units[..end]) {
                Ok(s) if units[end..].iter().all(|&u| u == 0) && !s.chars().any(char::is_control) => Value::Text(s),
                _ => Value::Raw { hex: hex(b) },
            }
        }
        FieldType::AsciiString => {
            let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            let text = &b[..end];
            if b[end..].iter().all(|&c| c == 0) && text.iter().all(|c| (0x20..0x7F).contains(c)) {
                Value::Text(String::from_utf8_lossy(text).into_owned())
            } else {
                Value::Raw { hex: hex(b) }
            }
        }
        kind if f.length == 1 => number(kind, b),
        kind => Value::List(b.chunks_exact(kind.element_size()).map(|c| number(kind, c)).collect()),
    })
}

fn put_number(f: &FieldDef, kind: FieldType, out: &mut [u8], v: &Value) -> Result<()> {
    let width = out.len();
    match (kind, v) {
        (FieldType::Float, Value::Float(x)) => out.copy_from_slice(&x.to_le_bytes()),
        (FieldType::Float, Value::Number(n)) => out.copy_from_slice(&(*n as f32).to_le_bytes()),
        (_, Value::Number(n)) => {
            let signed = matches!(kind, FieldType::Short | FieldType::Int);
            let bits = 8 * width as u32;
            let ok = if signed {
                let half = 1i64 << (bits - 1);
                (-half..half).contains(n)
            } else {
                *n >= 0 && (bits >= 63 || *n < (1i64 << bits))
            };
            if !ok {
                return Err(err(f, format!("{n} does not fit in {width} byte(s)")));
            }
            out.copy_from_slice(&n.to_le_bytes()[..width]);
        }
        _ => return Err(err(f, format!("expected a number, got {v:?}"))),
    }
    Ok(())
}

fn unhex(f: &FieldDef, s: &str, len: usize) -> Result<Vec<u8>> {
    let bytes: Option<Vec<u8>> =
        (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect();
    match bytes {
        Some(b) if b.len() == len => Ok(b),
        _ => Err(err(f, format!("raw value must be {len} bytes of hex"))),
    }
}

/// Encode `v` into field `f` of `record`.
pub fn write(record: &mut [u8], f: &FieldDef, v: &Value) -> Result<()> {
    let len = f.byte_len();
    if record.len() < f.offset + len {
        return Err(err(f, "record too short"));
    }
    let out = &mut record[f.offset..f.offset + len];
    match (f.kind, v) {
        (_, Value::Raw { hex }) => out.copy_from_slice(&unhex(f, hex, len)?),
        (FieldType::Bit, Value::Number(n)) => {
            let start = f.bit_start.unwrap_or(0);
            let mask = (1u64 << f.length) - 1;
            if *n < 0 || *n as u64 > mask {
                return Err(err(f, format!("{n} does not fit in {} bit(s)", f.length)));
            }
            let word = (le(out) & !(mask << start)) | ((*n as u64) << start);
            out.copy_from_slice(&word.to_le_bytes()[..len]);
        }
        (FieldType::UnicodeString, Value::Text(s)) => {
            let units: Vec<u16> = s.encode_utf16().collect();
            if units.len() > f.length {
                return Err(err(f, format!("text longer than {} characters", f.length)));
            }
            out.fill(0);
            for (i, u) in units.iter().enumerate() {
                out[2 * i..2 * i + 2].copy_from_slice(&u.to_le_bytes());
            }
        }
        (FieldType::AsciiString, Value::Text(s)) => {
            if !s.is_ascii() || s.len() > f.length {
                return Err(err(f, format!("text must be ASCII, at most {} characters", f.length)));
            }
            out.fill(0);
            out[..s.len()].copy_from_slice(s.as_bytes());
        }
        (kind, Value::List(items)) if f.length > 1 => {
            let size = kind.element_size();
            if items.len() != f.length {
                return Err(err(f, format!("expected {} values, got {}", f.length, items.len())));
            }
            for (item, chunk) in items.iter().zip(out.chunks_exact_mut(size)) {
                put_number(f, kind, chunk, item)?;
            }
        }
        (kind, v) if f.length == 1 => put_number(f, kind, out, v)?,
        (_, v) => return Err(err(f, format!("unsuitable value {v:?}"))),
    }
    Ok(())
}

/// A record of `size` bytes holding every field's RCDB default (`DefaultValueEn`: integers for
/// numbers and bits, hex bytes for strings). Bytes not covered by a field are 0.
pub fn default_record(defs: &[FieldDef], size: usize) -> Result<Vec<u8>> {
    let mut rec = vec![0u8; size];
    for f in defs {
        let text = f.default_en.trim();
        let value = match f.kind {
            FieldType::UnicodeString | FieldType::AsciiString => {
                let mut bytes = unhex(f, text, text.len() / 2)?;
                bytes.resize(f.byte_len(), 0);
                Value::Raw { hex: hex(&bytes) }
            }
            _ => {
                let n: i64 = if text.is_empty() { 0 } else { text.parse().map_err(|_| err(f, format!("default {text:?} is not a number")))? };
                if f.kind != FieldType::Bit && f.length > 1 {
                    Value::List(vec![Value::Number(n); f.length])
                } else {
                    Value::Number(n)
                }
            }
        };
        write(&mut rec, f, &value)?;
    }
    Ok(rec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(kind: FieldType, offset: usize, length: usize, bit: Option<(u32, usize)>) -> FieldDef {
        FieldDef {
            id: 1,
            name: "t".into(),
            file_id: 1,
            block_id: 1,
            block_type: 1,
            offset,
            kind,
            length,
            bit_start: bit.map(|b| b.0),
            bit_container: bit.map_or(0, |b| b.1),
            default_en: String::new(),
        }
    }

    #[test]
    fn decodes_recorded_channel_fields() {
        // Start of digital channel 0 as read from the BP515.
        let mut rec = vec![0u8; 76];
        for (i, u) in "APTLK_CH1".encode_utf16().enumerate() {
            rec[2 * i..2 * i + 2].copy_from_slice(&u.to_le_bytes());
        }
        rec[36..40].copy_from_slice(&[0x08, 0xf7, 0x81, 0x1a]);
        assert_eq!(read(&rec, &def(FieldType::UnicodeString, 0, 16, None)).unwrap(), Value::Text("APTLK_CH1".into()));
        assert_eq!(read(&rec, &def(FieldType::Int, 36, 1, None)).unwrap(), Value::Number(444_725_000));
    }

    #[test]
    fn bit_fields_round_trip_without_touching_neighbours() {
        let f = def(FieldType::Bit, 0, 12, Some((0, 2)));
        let g = def(FieldType::Bit, 0, 2, Some((14, 2)));
        let mut rec = vec![0xAB, 0xCD];
        assert_eq!(read(&rec, &f).unwrap(), Value::Number(0xDAB));
        assert_eq!(read(&rec, &g).unwrap(), Value::Number(3));
        write(&mut rec, &g, &Value::Number(1)).unwrap();
        assert_eq!(rec, vec![0xAB, 0x4D]);
        assert!(write(&mut rec, &g, &Value::Number(4)).is_err());
    }

    #[test]
    fn unclean_text_stays_raw() {
        let f = def(FieldType::UnicodeString, 0, 3, None);
        let rec = vec![0x41, 0, 0, 0, 0x42, 0]; // "A", NUL, then "B" after the terminator
        let v = read(&rec, &f).unwrap();
        assert_eq!(v, Value::Raw { hex: "410000004200".into() });
        let mut back = vec![0u8; 6];
        write(&mut back, &f, &v).unwrap();
        assert_eq!(back, rec);
    }
}
