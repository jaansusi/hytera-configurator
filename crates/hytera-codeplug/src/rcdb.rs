//! Access to the RCDB files that ship with Hytera CPS (loaded from disk, never bundled).

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Where CPS V1.5.24.000.0003 keeps its data on Windows. Other installs: pass the folder that
/// contains `RcdbXmls\` and `RcdbJsons\`.
pub const DEFAULT_CPS_DATA_DIR: &str =
    r"C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003";

/// The folders of a CPS install that hyt reads (file lists, layouts, linked lists).
pub const RCDB_FOLDERS: [&str; 3] = ["RcdbJsons", "RcdbXmls", "RCDB"];

/// The per-user copy of CPS's RCDB data made by [`import`] (`<data dir>/cps`).
pub fn user_copy_dir() -> Option<PathBuf> {
    Some(crate::options::data_dir()?.join("cps"))
}

/// Where to load RCDB data from when no folder is given: the CPS install if it is there, else the
/// per-user copy if that exists, else the CPS install path (so error messages name it).
pub fn default_root() -> PathBuf {
    let cps = PathBuf::from(DEFAULT_CPS_DATA_DIR);
    if cps.join("RcdbJsons").is_dir() {
        return cps;
    }
    match user_copy_dir() {
        Some(copy) if copy.join("RcdbJsons").is_dir() => copy,
        _ => cps,
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<usize> {
    std::fs::create_dir_all(dst).map_err(|source| Error::Io { path: dst.to_path_buf(), source })?;
    let mut n = 0;
    let entries = std::fs::read_dir(src).map_err(|source| Error::Io { path: src.to_path_buf(), source })?;
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io { path: src.to_path_buf(), source })?;
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if from.is_dir() {
            n += copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|source| Error::Io { path: from.clone(), source })?;
            n += 1;
        }
    }
    Ok(n)
}

/// Copy the RCDB folders hyt needs ([`RCDB_FOLDERS`]) from `src` (a CPS install or a copy of one)
/// to `dst`. Returns the number of files copied. Hytera data: keep it out of shared folders.
pub fn import(src: &Path, dst: &Path) -> Result<usize> {
    if !src.join("RcdbJsons").is_dir() {
        return Err(Error::Invalid(format!("{} has no RcdbJsons folder (expected a CPS install or a copy of one)", src.display())));
    }
    let mut n = 0;
    for folder in RCDB_FOLDERS {
        if src.join(folder).is_dir() {
            n += copy_tree(&src.join(folder), &dst.join(folder))?;
        }
    }
    Ok(n)
}

/// RCDB versions with a layout in `root` (`RcdbJsons\<version>.txt`), sorted.
pub fn versions(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root.join("RcdbJsons"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".txt").map(str::to_string))
        .collect();
    v.sort();
    v
}

/// One radio file as listed by the RCDB file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub id: u16,
    pub name: String,
}

/// The RCDB data folder (a CPS install, or a copy of its RCDB folders).
#[derive(Debug, Clone)]
pub struct RcdbRoot {
    root: PathBuf,
}

impl RcdbRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        RcdbRoot { root: root.into() }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The files CPS reads for `version` (e.g. `V1.5.24.000`), in CPS's order.
    ///
    /// Looks in `RcdbXmls\<version>.xml` (all versions) and then `RCDB\FileID\<version>.xml`
    /// (the install's own version). For V1.5.24.000 both are identical.
    pub fn file_list(&self, version: &str) -> Result<Vec<FileEntry>> {
        // The version comes from the radio and becomes part of a path: allow only "V1.5.24.000"-like text.
        let plausible = !version.is_empty()
            && !version.contains("..")
            && version.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
        if !plausible {
            return Err(Error::Invalid(format!("unexpected RCDB version string {version:?}")));
        }
        let tried = vec![
            self.root.join("RcdbXmls").join(format!("{version}.xml")),
            self.root.join("RCDB").join("FileID").join(format!("{version}.xml")),
        ];
        let Some(path) = tried.iter().find(|p| p.is_file()).cloned() else {
            return Err(Error::MissingVersion { version: version.into(), root: self.root.clone(), tried });
        };
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io { path: path.clone(), source })?;
        let list = match parse_file_list(&text) {
            Ok(list) => list,
            Err(Error::Xml { source, .. }) => return Err(Error::Xml { path, source }),
            Err(Error::Invalid(msg)) => return Err(Error::Invalid(format!("{}: {msg}", path.display()))),
            Err(other) => return Err(other),
        };
        if list.is_empty() {
            return Err(Error::Invalid(format!("{}: no <RcdbFile> entries", path.display())));
        }
        Ok(list)
    }
}

/// A two-level list file and its one-level companion (e.g. zones 6 and zone aliases 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableLink {
    pub two_dim: u16,
    pub one_dim: u16,
}

impl RcdbRoot {
    /// Pairs from `RCDB\TableLink\TableLink.xml` (only shipped for the CPS install's own version).
    pub fn table_links(&self) -> Result<Vec<TableLink>> {
        let path = self.root.join("RCDB").join("TableLink").join("TableLink.xml");
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io { path: path.clone(), source })?;
        parse_table_links(&text).map_err(|e| match e {
            Error::Xml { source, .. } => Error::Xml { path, source },
            other => other,
        })
    }
}

/// Parse `TableLink.xml` (`<Data><Name TwoDimID=".." OneDimID=".." …/>…</Data>`).
pub fn parse_table_links(xml: &str) -> Result<Vec<TableLink>> {
    let xml = xml.trim_start_matches('\u{FEFF}');
    let doc = roxmltree::Document::parse(xml)
        .map_err(|source| Error::Xml { path: PathBuf::new(), source })?;
    let attr = |n: &roxmltree::Node, a: &str| n.attribute(a).and_then(|v| v.trim().parse::<u16>().ok());
    Ok(doc
        .descendants()
        .filter_map(|n| Some(TableLink { two_dim: attr(&n, "TwoDimID")?, one_dim: attr(&n, "OneDimID")? }))
        .collect())
}

/// The order CPS writes files in: the read order, minus `excluded`, with each one-level file
/// moved to just before its two-level partner (observed on 2026-11-18, e.g. 7 before 6).
pub fn write_order(read_order: &[u16], links: &[TableLink], excluded: &[u16]) -> Vec<u16> {
    let present = |id: u16| read_order.contains(&id) && !excluded.contains(&id);
    let partner_of_two_dim = |id: u16| links.iter().find(|l| l.two_dim == id && present(l.one_dim));
    let is_moved_one_dim = |id: u16| links.iter().any(|l| l.one_dim == id && present(l.two_dim));
    let mut out = Vec::new();
    for &id in read_order.iter().filter(|&&id| present(id)) {
        if is_moved_one_dim(id) {
            continue;
        }
        if let Some(link) = partner_of_two_dim(id) {
            out.push(link.one_dim);
        }
        out.push(id);
    }
    out
}

/// Parse a file-list XML (`<root><DMR><RcdbFile><FileId/><FileName/></RcdbFile>…`).
pub fn parse_file_list(xml: &str) -> Result<Vec<FileEntry>> {
    let xml = xml.trim_start_matches('\u{FEFF}');
    let doc = roxmltree::Document::parse(xml)
        .map_err(|source| Error::Xml { path: PathBuf::new(), source })?;
    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("RcdbFile")) {
        let child = |tag: &str| {
            node.children().find(|c| c.has_tag_name(tag)).and_then(|c| c.text()).map(str::trim)
        };
        let id = child("FileId")
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| Error::Invalid("RcdbFile without a valid FileId".into()))?;
        let name = child("FileName").unwrap_or_default().to_string();
        out.push(FileEntry { id, name });
    }
    Ok(out)
}

#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn import_copies_the_rcdb_folders() {
        let base = std::env::temp_dir().join(format!("hyt-rcdb-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("cps");
        for (dir, file) in [("RcdbJsons", "V1.txt"), ("RCDB/TableLink", "TableLink.xml"), ("RCDB/FileID", "V1.xml"), ("Other", "x.dll")] {
            std::fs::create_dir_all(src.join(dir)).unwrap();
            std::fs::write(src.join(dir).join(file), b"x").unwrap();
        }
        let dst = base.join("copy");
        assert_eq!(import(&src, &dst).unwrap(), 3);
        assert!(dst.join("RCDB/TableLink/TableLink.xml").is_file());
        assert!(!dst.join("Other").exists(), "only the RCDB folders are copied");
        assert_eq!(versions(&dst), ["V1"]);
        assert!(import(&base.join("nothing"), &dst).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_order_matches_cps() {
        let links = [
            TableLink { two_dim: 6, one_dim: 7 },
            TableLink { two_dim: 80, one_dim: 81 },
            TableLink { two_dim: 166, one_dim: 165 },
        ];
        let read = [5, 6, 74, 80, 81, 7, 8, 165, 166, 242];
        assert_eq!(write_order(&read, &links, &[242]), vec![5, 7, 6, 74, 81, 80, 8, 165, 166]);
    }

    #[test]
    fn parses_table_links() {
        let xml = r#"<Data ver="V1.5.24.000"><QEU_C_Zone TwoDimID="6" OneDimID="7" OneDimName="QEU_C_ZoneAlias" ProductName="DMR" /></Data>"#;
        assert_eq!(parse_table_links(xml).unwrap(), vec![TableLink { two_dim: 6, one_dim: 7 }]);
    }

    #[test]
    fn parses_file_list() {
        let xml = "\u{FEFF}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<root ver=\"V1.5.24.000\"><DMR FileCnt=\"2\">\
            <RcdbFile><FileId>18</FileId><FileName>Comm_Basic</FileName></RcdbFile>\
            <RcdbFile><FileId>3</FileId><FileName>QEU_C_DChannel</FileName></RcdbFile></DMR></root>";
        let list = parse_file_list(xml).unwrap();
        assert_eq!(
            list,
            vec![
                FileEntry { id: 18, name: "Comm_Basic".into() },
                FileEntry { id: 3, name: "QEU_C_DChannel".into() }
            ]
        );
    }
}
