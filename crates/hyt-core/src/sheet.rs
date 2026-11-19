//! Spreadsheet (CSV) export and import of a table: every field of every record, with CPS's labels
//! and units, so lists can be edited in bulk in a spreadsheet program.
//!
//! Import goes through [`Workspace::set_value`] cell by cell (the same validation as typing in the
//! app). Rows are matched by their `No.`; rows beyond the current records add channels in the
//! channel tables (from CPS's template, see [`Workspace::add_channel`]) and are refused elsewhere.
//! Records missing from the file are kept (import never removes). Each refused cell is reported;
//! accepted cells stay applied.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

use crate::tables::{Table, record_detail, table};
use crate::workspace::{ChannelKind, Workspace};

/// Header of the record-number column.
pub const NO: &str = "No.";

/// What an import did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    /// Cells whose value changed.
    pub changed: usize,
    /// Records added.
    pub added: usize,
    /// Refused cells or rows, with the reason (`row N, column: reason`).
    pub errors: Vec<String>,
}

fn header_name(key: &str, unit: &str) -> String {
    if unit.is_empty() { key.to_string() } else { format!("{key} [{unit}]") }
}

/// The field key in a header cell (`"RX Frequency [MHz]"` → `"RX Frequency"`).
fn header_key(h: &str) -> &str {
    match h.rfind(" [") {
        Some(i) if h.ends_with(']') => &h[..i],
        _ => h,
    }
}

fn channel_kind(t: &Table) -> Option<ChannelKind> {
    match t.id.as_str() {
        "digital" => Some(ChannelKind::Digital),
        "analog" => Some(ChannelKind::Analog),
        _ => None,
    }
}

/// CSV text of table `id` with every field (in layout order) of every record.
pub fn export(ws: &Workspace, id: &str) -> Result<String> {
    let t = table(ws, id).with_context(|| format!("no table {id:?}"))?;
    let mut w = csv::Writer::from_writer(Vec::new());
    let Some(first) = t.rows.first() else {
        w.write_record([NO])?;
        return Ok(String::from_utf8(w.into_inner()?)?);
    };
    let columns = record_detail(ws, &first.record);
    let mut header = vec![NO.to_string()];
    header.extend(columns.iter().map(|d| header_name(&d.column.key, &d.column.unit)));
    w.write_record(&header)?;
    for row in &t.rows {
        let detail = record_detail(ws, &row.record);
        let mut rec = vec![row.no.to_string()];
        rec.extend(detail.iter().map(|d| d.cell.text.clone()));
        w.write_record(&rec)?;
    }
    Ok(String::from_utf8(w.into_inner()?)?)
}

/// Apply CSV `text` (as written by [`export`], possibly edited) to table `id`.
pub fn import(ws: &mut Workspace, id: &str, text: &str) -> Result<ImportReport> {
    let mut r = csv::ReaderBuilder::new().flexible(true).from_reader(text.trim_start_matches('\u{FEFF}').as_bytes());
    let headers: Vec<String> = r.headers()?.iter().map(|h| header_key(h.trim()).to_string()).collect();
    let no_col = headers.iter().position(|h| h == NO).with_context(|| format!("the first row has no {NO:?} column"))?;
    let mut report = ImportReport::default();
    for (line, rec) in r.records().enumerate() {
        let rec = rec?;
        let row_label = format!("row {}", line + 2);
        let t = table(ws, id).with_context(|| format!("no table {id:?}"))?;
        let no: usize = match rec.get(no_col).map(str::trim) {
            Some("") | None => t.rows.len() + 1,
            Some(n) => match n.parse() {
                Ok(n) if n >= 1 => n,
                _ => {
                    report.errors.push(format!("{row_label}: {NO} {n:?} is not a record number"));
                    continue;
                }
            },
        };
        if no > t.rows.len() {
            let Some(kind) = channel_kind(&t) else {
                report.errors.push(format!("{row_label}: record {no} does not exist (adding records here needs a CPS recording)"));
                continue;
            };
            if no != t.rows.len() + 1 {
                report.errors.push(format!("{row_label}: new channels must follow the last one ({})", t.rows.len() + 1));
                continue;
            }
            match ws.add_channel(kind) {
                Ok(_) => report.added += 1,
                Err(e) => {
                    report.errors.push(format!("{row_label}: {e:#}"));
                    continue;
                }
            }
        }
        let t = table(ws, id).context("table disappeared")?;
        let row = &t.rows[no - 1];
        let detail = record_detail(ws, &row.record);
        for (i, key) in headers.iter().enumerate() {
            if i == no_col {
                continue;
            }
            let Some(value) = rec.get(i).map(str::trim) else { continue };
            let Some(field) = detail.iter().find(|d| &d.column.key == key) else {
                if line == 0 {
                    report.errors.push(format!("column {key:?}: no such field; ignored"));
                }
                continue;
            };
            if value == field.cell.text {
                continue;
            }
            if !field.cell.editable {
                report.errors.push(format!("{row_label}, {key}: cannot be changed here"));
                continue;
            }
            let Some(at) = &field.cell.field else { continue };
            match ws.set_value(at, value) {
                Ok(()) => report.changed += 1,
                Err(e) => {
                    // The workspace's message usually starts with the field name already.
                    let msg = format!("{e:#}");
                    let msg = msg.strip_prefix(&format!("{key}: ")).unwrap_or(&msg);
                    report.errors.push(format!("{row_label}, {key}: {msg}"));
                }
            }
        }
    }
    if report.changed == 0 && report.added == 0 && report.errors.is_empty() {
        return Ok(report);
    }
    if report.changed == 0 && report.added == 0 {
        bail!("nothing imported:\n  {}", report.errors.join("\n  "));
    }
    Ok(report)
}

/// Write table `id` as CSV to `path` (replacing it; a save dialog has asked). Returns the number
/// of records written.
pub fn export_to_file(ws: &Workspace, id: &str, path: &Path) -> Result<usize> {
    let text = export(ws, id)?;
    std::fs::write(path, &text).with_context(|| format!("writing {}", path.display()))?;
    Ok(table(ws, id).map_or(0, |t| t.rows.len()))
}

/// [`import`] the CSV file at `path` into table `id`.
pub fn import_file(ws: &mut Workspace, id: &str, path: &Path) -> Result<ImportReport> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow!("{} is not UTF-8 text; save it from the spreadsheet as \"CSV UTF-8\"", path.display()))?;
    import(ws, id, &text)
}

/// A file name for table `id` of the open folder: `<folder name>-<table>.csv` (lists by title).
pub fn suggested_file_name(ws: &Workspace, id: &str) -> String {
    let folder = ws.folder.file_name().map_or_else(|| "hyt".to_string(), |n| n.to_string_lossy().into_owned());
    let table_name = match id.strip_prefix(crate::lists::ID_PREFIX) {
        Some(file) => table(ws, id).map_or_else(|| file.to_string(), |t| t.title.to_lowercase()),
        None => id.to_string(),
    };
    format!("{}-{}.csv", file_safe(&folder), file_safe(&table_name))
}

/// `name` with characters that are not allowed in file names (or awkward in them) replaced by `-`.
fn file_safe(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' })
        .collect();
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    let out = out.trim_matches(|c| c == '-' || c == '.').to_string();
    if out.is_empty() { "table".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_safe("Scan Lists"), "Scan-Lists");
        assert_eq!(file_safe("read_2026-11-19"), "read_2026-11-19");
        assert_eq!(file_safe("a: b/c"), "a-b-c");
        assert_eq!(file_safe(" ..? "), "table");
    }

    #[test]
    fn headers_carry_units() {
        assert_eq!(header_name("RX Frequency", "MHz"), "RX Frequency [MHz]");
        assert_eq!(header_key("RX Frequency [MHz]"), "RX Frequency");
        assert_eq!(header_key("Channel Alias"), "Channel Alias");
        assert_eq!(header_key("Odd [name"), "Odd [name");
    }
}
