//! CPS's help texts for parameters, from `CPSParams.xlsx` in the CPS install folder.
//!
//! Observed (CPS V1.5.24.000.0003): one worksheet, a header row in Chinese, then one row per
//! parameter with the parameter ID (the same `Id` as the RCDB layout's fields), the English and
//! Chinese names, and descriptions, notes and option explanations in Chinese and English, each for
//! "common/DMR" and for TETRA, plus a visibility class (`可视性`: shown by both CPS builds, Hytera's
//! internal build only, or neither). hyt uses the English common/DMR columns and the visibility. 584 of 1336 rows have an
//! English description. The ID → field mapping is the layout's `Id` (observed: names agree for all
//! but renamed fields such as "Use Scan List Roam List" / "Use Scan or Roam List").
//!
//! The texts are Hytera's and are read from the install at run time, never copied into hyt.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use serde::Serialize;

use crate::rcdb::RcdbRoot;
use crate::{Error, Result};

/// File name of the help workbook in the CPS install folder.
pub const HELP_FILE: &str = "CPSParams.xlsx";

/// CPS's help for one parameter (English). At least one of the texts is present.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ParamHelp {
    /// What the parameter does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// CPS's "notes" (precautions, dependencies).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// What each choice means.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
}

/// Which CPS builds show a parameter (the workbook's column `可视性`, observed values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildVisibility {
    /// `内外部版本都可见`: shown by the internal and the external (customer) build.
    Both,
    /// `仅内部版本可见`: only Hytera's internal build shows it.
    InternalOnly,
    /// `内外部版本都不可见`: no build shows it.
    Neither,
}

impl BuildVisibility {
    fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "内外部版本都可见" => Some(Self::Both),
            "仅内部版本可见" => Some(Self::InternalOnly),
            "内外部版本都不可见" => Some(Self::Neither),
            _ => None,
        }
    }
}

/// Help texts by parameter ID.
#[derive(Debug, Clone, Default)]
pub struct HelpCatalog {
    by_id: HashMap<u32, ParamHelp>,
    names: HashMap<u32, String>,
    visibility: HashMap<u32, BuildVisibility>,
}

impl HelpCatalog {
    /// Read `CPSParams.xlsx` (or a copy of it).
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
        Self::from_xlsx(&bytes).map_err(|e| match e {
            Error::Invalid(msg) => Error::Invalid(format!("{}: {msg}", path.display())),
            Error::Xml { source, .. } => Error::Xml { path: path.to_path_buf(), source },
            other => other,
        })
    }

    /// Parse the workbook's bytes.
    pub fn from_xlsx(bytes: &[u8]) -> Result<Self> {
        let rows = read_first_sheet(bytes)?;
        let mut rows = rows.into_iter();
        let header = rows.next().ok_or_else(|| Error::Invalid("empty worksheet".into()))?;
        let find = |want: &dyn Fn(&str) -> bool, fallback: &str| -> String {
            header.iter().find(|(_, text)| want(&normalize_header(text))).map_or_else(|| fallback.to_string(), |(col, _)| col.clone())
        };
        // Header texts as observed; the column letters are the fallback (D, E, J, N, R).
        let id_col = find(&|h| h.contains("参数ID"), "D");
        let name_col = find(&|h| h.starts_with("参数名(英"), "E");
        let english_dmr = |prefix: &'static str| move |h: &str| h.starts_with(prefix) && h.contains("英_公共");
        let text_col = find(&english_dmr("参数描述"), "J");
        let notes_col = find(&english_dmr("注意事项"), "N");
        let options_col = find(&english_dmr("选项说明"), "R");
        let visibility_col = find(&|h| h.starts_with("可视性"), "G");

        let mut catalog = HelpCatalog::default();
        for row in rows {
            let get = |col: &str| row.iter().find(|(c, _)| c == col).map(|(_, v)| v.as_str());
            let Some(id) = get(&id_col).and_then(|v| v.trim().parse::<u32>().ok()) else { continue };
            let text = |col: &str| get(col).and_then(clean);
            let help = ParamHelp { text: text(&text_col), notes: text(&notes_col), options: text(&options_col) };
            if let Some(name) = get(&name_col) {
                catalog.names.insert(id, name.trim().to_string());
            }
            if let Some(v) = get(&visibility_col).and_then(BuildVisibility::parse) {
                catalog.visibility.insert(id, v);
            }
            if help.text.is_some() || help.notes.is_some() || help.options.is_some() {
                catalog.by_id.insert(id, help);
            }
        }
        if catalog.names.is_empty() {
            return Err(Error::Invalid("no parameter rows found (unexpected workbook layout)".into()));
        }
        Ok(catalog)
    }

    /// Help for parameter `id` (the layout's field `Id`).
    pub fn get(&self, id: u32) -> Option<&ParamHelp> {
        self.by_id.get(&id)
    }

    /// The English parameter name the workbook gives for `id`.
    pub fn name(&self, id: u32) -> Option<&str> {
        self.names.get(&id).map(String::as_str)
    }

    /// Which CPS builds show parameter `id`, when the workbook says.
    pub fn visibility(&self, id: u32) -> Option<BuildVisibility> {
        self.visibility.get(&id).copied()
    }

    /// Number of parameters with help.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

impl RcdbRoot {
    /// CPS's help texts (`CPSParams.xlsx` in the RCDB root).
    pub fn help_catalog(&self) -> Result<HelpCatalog> {
        HelpCatalog::load(&self.path().join(HELP_FILE))
    }
}

/// Header text with full-width brackets made ASCII (the workbook mixes both).
fn normalize_header(text: &str) -> String {
    text.trim().replace('（', "(").replace('）', ")")
}

/// Trimmed text with Windows line ends and Excel's `_x000D_` escapes removed; `None` when empty.
fn clean(text: &str) -> Option<String> {
    let text = text.replace("_x000D_", "").replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line.trim_start());
    }
    let out = out.trim().to_string();
    (!out.is_empty() && out != "None" && out != "/").then_some(out)
}

type SheetRow = Vec<(String, String)>;

/// Rows of the workbook's first worksheet as (column letters, text) pairs.
fn read_first_sheet(bytes: &[u8]) -> Result<Vec<SheetRow>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Invalid(format!("not an .xlsx file: {e}")))?;
    let mut entry = |name: &str| -> Result<Option<String>> {
        let Ok(mut f) = zip.by_name(name) else { return Ok(None) };
        let mut text = String::new();
        f.read_to_string(&mut text).map_err(|e| Error::Invalid(format!("{name}: {e}")))?;
        Ok(Some(text))
    };
    let shared = match entry("xl/sharedStrings.xml")? {
        Some(xml) => shared_strings(&xml)?,
        None => Vec::new(),
    };
    let sheet = entry("xl/worksheets/sheet1.xml")?.ok_or_else(|| Error::Invalid("no xl/worksheets/sheet1.xml".into()))?;
    let doc = roxmltree::Document::parse(&sheet).map_err(xml_error)?;
    let mut rows = Vec::new();
    for row in doc.descendants().filter(|n| n.has_tag_name("row")) {
        let mut cells = Vec::new();
        for c in row.children().filter(|n| n.has_tag_name("c")) {
            let Some(col) = c.attribute("r").map(|r| r.trim_end_matches(|ch: char| ch.is_ascii_digit()).to_string()) else { continue };
            let value = c.children().find(|n| n.has_tag_name("v")).and_then(|v| v.text()).unwrap_or("");
            let text = match c.attribute("t") {
                Some("s") => value.trim().parse::<usize>().ok().and_then(|i| shared.get(i)).cloned().unwrap_or_default(),
                Some("inlineStr") => text_runs(c),
                _ => value.to_string(),
            };
            cells.push((col, text));
        }
        rows.push(cells);
    }
    Ok(rows)
}

fn shared_strings(xml: &str) -> Result<Vec<String>> {
    let doc = roxmltree::Document::parse(xml).map_err(xml_error)?;
    Ok(doc.root_element().children().filter(|n| n.has_tag_name("si")).map(text_runs).collect())
}

/// The text of a string item: its `<t>` elements, without phonetic runs (`<rPh>`).
fn text_runs(node: roxmltree::Node) -> String {
    node.descendants()
        .filter(|n| n.has_tag_name("t") && !n.ancestors().any(|a| a.has_tag_name("rPh")))
        .filter_map(|n| n.text())
        .collect()
}

fn xml_error(source: roxmltree::Error) -> Error {
    Error::Xml { path: HELP_FILE.into(), source }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_texts() {
        assert_eq!(clean("  Set it.\r\n"), Some("Set it.".into()));
        assert_eq!(clean("A: one. \r\nB: two._x000D_\r\n\r\n\r\nNote."), Some("A: one.\nB: two.\n\nNote.".into()));
        assert_eq!(clean(" \r\n "), None);
        assert_eq!(clean("None"), None);
    }

    fn workbook(shared: &[&str], rows: &[&[(&str, usize)]]) -> Vec<u8> {
        use std::io::Write;
        let mut sst = String::from(r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#);
        for s in shared {
            sst.push_str(&format!("<si><t xml:space=\"preserve\">{s}</t><rPh><t>x</t></rPh></si>"));
        }
        sst.push_str("</sst>");
        let mut sheet = String::from(r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#);
        for (i, row) in rows.iter().enumerate() {
            sheet.push_str(&format!("<row r=\"{}\">", i + 1));
            for (col, s) in *row {
                sheet.push_str(&format!("<c r=\"{col}{}\" t=\"s\"><v>{s}</v></c>", i + 1));
            }
            sheet.push_str("</row>");
        }
        sheet.push_str("</sheetData></worksheet>");
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("xl/sharedStrings.xml", opts).unwrap();
        w.write_all(sst.as_bytes()).unwrap();
        w.start_file("xl/worksheets/sheet1.xml", opts).unwrap();
        w.write_all(sheet.as_bytes()).unwrap();
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_english_columns_by_header() {
        let shared = [
            "参数ID",
            "参数名(英)",
            "参数描述(英_公共/DMR定制)",
            "注意事项（英_公共/DMR定制）",
            "选项说明（英_公共/DMR定制）",
            "参数描述（中_公共/DMR定制）",
            "7",
            "RX Frequency",
            "Set the RX frequency.",
            "中文",
            "9",
            "TX Admit",
            "Always Allow: any time.\r\nChannel Free: when free.",
            "10",
        ];
        // Columns deliberately not at the observed letters: the header decides.
        let rows: &[&[(&str, usize)]] = &[
            &[("A", 0), ("B", 1), ("C", 5), ("D", 2), ("F", 3), ("G", 4)],
            &[("A", 6), ("B", 7), ("C", 9), ("D", 8)],
            &[("A", 10), ("B", 11), ("G", 12)],
            &[("A", 13), ("B", 11)],
        ];
        let c = HelpCatalog::from_xlsx(&workbook(&shared, rows)).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(7).unwrap().text.as_deref(), Some("Set the RX frequency."));
        assert_eq!(c.get(9).unwrap().options.as_deref(), Some("Always Allow: any time.\nChannel Free: when free."));
        assert_eq!(c.get(9).unwrap().text, None);
        assert!(c.get(10).is_none(), "rows without English help are left out");
        assert_eq!(c.name(10), Some("TX Admit"));
    }

    #[test]
    fn loads_cps_help_when_installed() {
        let root = RcdbRoot::new(crate::rcdb::DEFAULT_CPS_DATA_DIR);
        if !root.path().join(HELP_FILE).is_file() {
            eprintln!("skipping: no CPS install");
            return;
        }
        let c = root.help_catalog().unwrap();
        assert!(c.len() > 500, "{} parameters with help", c.len());
        // Channel Alias (id 1) and TX Admit (id 9) as observed in CPS V1.5.24.000.0003.
        assert_eq!(c.get(1).and_then(|h| h.text.as_deref()), Some("Set the unique name of the channel."));
        assert!(c.get(9).and_then(|h| h.options.as_deref()).is_some_and(|o| o.contains("Channel Free")));
        // Visibility column: Encrypt (32) both builds, Encrypt Type (33) neither, card authentication internal only.
        assert_eq!(c.visibility(32), Some(BuildVisibility::Both));
        assert_eq!(c.visibility(33), Some(BuildVisibility::Neither));
        assert_eq!(c.visibility(4295), Some(BuildVisibility::InternalOnly));
    }
}
