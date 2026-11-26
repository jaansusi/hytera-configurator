//! CPS template files (`.tpl`): reading them, importing one as a `hyt` folder for a given radio,
//! and exporting a `hyt` folder as a template CPS can open. Format: `docs/file-format.md` section 6.
//!
//! A `.tpl` is a ZIP archive: `tpHeader.data` (XML `TemplateBase`: product, model number, RCDB
//! version, serial number, name, …) followed by one `<FileName>.hps` per radio file. Each `.hps` is
//! itself a ZIP archive: `psHeader.data` (XML `ParamStore`: file ID as `Key`, RCDB version, times),
//! for a two-level list file its one-level companion (e.g. zone aliases inside `QEU_C_Zone.hps`),
//! and the radio file itself, in the container format `hyt read` saves (in the form CPS writes).

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset, Local, Timelike};
use hytera_codeplug::container::{FileHeader, prepare_for_write};
use hytera_codeplug::layout::Layout;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot, TableLink};
use hytera_proto::session::item;
use serde::{Deserialize, Serialize};

use crate::workspace::Workspace;
use crate::{ReadManifest, SavedRead, load_config, save_config_with_extras};

/// Name of the template header entry in a `.tpl`.
pub const TEMPLATE_HEADER: &str = "tpHeader.data";
/// Name of the header entry in each `.hps`.
pub const STORE_HEADER: &str = "psHeader.data";
/// File `hyt tpl import` adds to the folder: the template's headers, so an export can reproduce them.
pub const SIDECAR: &str = "template.json";

const XML_NAMESPACES: &str =
    r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema""#;

/// `tpHeader.data` (XML root `TemplateBase`). `None` = element absent; `Some("")` = empty element.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateHeader {
    pub operate_permission: Option<String>,
    pub operate_type: Option<String>,
    pub is_create: Option<String>,
    pub guid: Option<String>,
    pub description: Option<String>,
    pub product: Option<String>,
    pub model_number: Option<String>,
    pub name: Option<String>,
    pub rcdb_version: Option<String>,
    pub frequence: Option<String>,
    pub created_time: Option<String>,
    pub product_id: Option<String>,
    pub serial_number: Option<String>,
}

impl TemplateHeader {
    /// The elements in CPS's order (observed in all four of the owner's templates).
    fn elements(&self) -> [(&'static str, &Option<String>); 13] {
        [
            ("OperatePermission", &self.operate_permission),
            ("OperateType", &self.operate_type),
            ("IsCreate", &self.is_create),
            ("Guid", &self.guid),
            ("Description", &self.description),
            ("Product", &self.product),
            ("ModelNumber", &self.model_number),
            ("Name", &self.name),
            ("RcdbVersion", &self.rcdb_version),
            ("Frequence", &self.frequence),
            ("CreatedTime", &self.created_time),
            ("ProductId", &self.product_id),
            ("SerialNumber", &self.serial_number),
        ]
    }

    pub fn parse(xml: &[u8]) -> Result<Self> {
        let get = parse_flat_xml(xml, "TemplateBase")?;
        Ok(TemplateHeader {
            operate_permission: get("OperatePermission"),
            operate_type: get("OperateType"),
            is_create: get("IsCreate"),
            guid: get("Guid"),
            description: get("Description"),
            product: get("Product"),
            model_number: get("ModelNumber"),
            name: get("Name"),
            rcdb_version: get("RcdbVersion"),
            frequence: get("Frequence"),
            created_time: get("CreatedTime"),
            product_id: get("ProductId"),
            serial_number: get("SerialNumber"),
        })
    }

    /// The XML as CPS writes it (no BOM, CRLF line ends, two-space indent).
    pub fn to_xml(&self) -> Vec<u8> {
        write_flat_xml("TemplateBase", &self.elements())
    }
}

/// `psHeader.data` (XML root `ParamStore`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamStore {
    /// CPS's (Chinese) description of the file plus `1` (observed: `File.description` in
    /// `XMLToolCompactData.sdf`); absent for one-level files.
    pub seqo: Option<String>,
    pub rcdb_version: Option<String>,
    /// The radio file ID.
    pub key: Option<String>,
    pub type_: Option<String>,
    pub model_number: Option<String>,
    pub create_time: Option<String>,
    pub modified_time: Option<String>,
}

impl ParamStore {
    fn elements(&self) -> [(&'static str, &Option<String>); 7] {
        [
            ("Seqo", &self.seqo),
            ("RcdbVersion", &self.rcdb_version),
            ("Key", &self.key),
            ("Type", &self.type_),
            ("ModelNumber", &self.model_number),
            ("CreateTime", &self.create_time),
            ("ModifiedTime", &self.modified_time),
        ]
    }

    pub fn parse(xml: &[u8]) -> Result<Self> {
        let get = parse_flat_xml(xml, "ParamStore")?;
        Ok(ParamStore {
            seqo: get("Seqo"),
            rcdb_version: get("RcdbVersion"),
            key: get("Key"),
            type_: get("Type"),
            model_number: get("ModelNumber"),
            create_time: get("CreateTime"),
            modified_time: get("ModifiedTime"),
        })
    }

    pub fn to_xml(&self) -> Vec<u8> {
        write_flat_xml("ParamStore", &self.elements())
    }

    /// `Key` as a file ID.
    pub fn file_id(&self) -> Option<u16> {
        self.key.as_deref()?.trim().parse().ok()
    }
}

/// Child elements (name → text) of a one-level XML document with root `root`.
fn parse_flat_xml(xml: &[u8], root: &str) -> Result<impl Fn(&str) -> Option<String>> {
    let text = std::str::from_utf8(xml).context("the header is not UTF-8 text")?;
    let text = text.trim_start_matches('\u{FEFF}');
    let doc = roxmltree::Document::parse(text).context("parsing the header XML")?;
    let r = doc.root_element();
    if !r.has_tag_name(root) {
        bail!("expected XML root <{root}>, found <{}>", r.tag_name().name());
    }
    let map: BTreeMap<String, String> = r
        .children()
        .filter(|c| c.is_element())
        .map(|c| (c.tag_name().name().to_string(), c.text().unwrap_or("").to_string()))
        .collect();
    Ok(move |name: &str| map.get(name).cloned())
}

/// XML in the form .NET's `XmlSerializer` writes it (as observed in CPS's files).
fn write_flat_xml(root: &str, elements: &[(&str, &Option<String>)]) -> Vec<u8> {
    let mut s = format!("<?xml version=\"1.0\"?>\r\n<{root} {XML_NAMESPACES}>\r\n");
    for (name, value) in elements {
        match value.as_deref() {
            None => {}
            Some("") => s.push_str(&format!("  <{name} />\r\n")),
            Some(v) => s.push_str(&format!("  <{name}>{}</{name}>\r\n", escape(v))),
        }
    }
    s.push_str(&format!("</{root}>\r\n"));
    s.into_bytes()
}

fn escape(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// One `.hps` entry: a parameter store holding one radio file (plus, for a two-level list file,
/// its one-level companion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateEntry {
    /// File name without `.hps` (= the radio file's RCDB name, e.g. `QEU_C_Zone`).
    pub name: String,
    pub store: ParamStore,
    /// The entries after `psHeader.data`, in archive order (name, contents).
    pub files: Vec<(String, Vec<u8>)>,
}

impl TemplateEntry {
    /// The radio file this entry is for (the inner file named like the entry).
    pub fn main_file(&self) -> Option<&[u8]> {
        self.files.iter().find(|(n, _)| *n == self.name).map(|(_, b)| b.as_slice())
    }
}

/// A whole `.tpl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub header: TemplateHeader,
    pub entries: Vec<TemplateEntry>,
}

/// One radio file in a template.
#[derive(Debug, Clone)]
pub struct TemplateFile {
    pub id: u16,
    pub name: String,
    pub bytes: Vec<u8>,
}

impl Template {
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&bytes).with_context(|| format!("reading the CPS template {}", path.display()))
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let outer = unzip(bytes).context("the template is not a ZIP archive")?;
        let mut header = None;
        let mut entries = Vec::new();
        for (name, data) in outer {
            if name == TEMPLATE_HEADER {
                header = Some(TemplateHeader::parse(&data).context("reading tpHeader.data")?);
                continue;
            }
            let Some(stem) = name.strip_suffix(".hps") else {
                bail!("unexpected entry {name:?} in the template");
            };
            let inner = unzip(&data).with_context(|| format!("{name} is not a ZIP archive"))?;
            let mut store = None;
            let mut files = Vec::new();
            for (n, d) in inner {
                if n == STORE_HEADER {
                    store = Some(ParamStore::parse(&d).with_context(|| format!("reading {name}/{STORE_HEADER}"))?);
                } else {
                    files.push((n, d));
                }
            }
            let store = store.with_context(|| format!("{name} has no {STORE_HEADER}"))?;
            let entry = TemplateEntry { name: stem.to_string(), store, files };
            if entry.main_file().is_none() {
                bail!("{name} does not contain the file {stem}");
            }
            entries.push(entry);
        }
        let header = header.context("the template has no tpHeader.data")?;
        Ok(Template { header, entries })
    }

    /// The radio files, one per `.hps`, in archive order. The ID comes from the file's own header
    /// and must agree with the store's `Key`.
    pub fn files(&self) -> Result<Vec<TemplateFile>> {
        self.entries
            .iter()
            .map(|e| {
                let bytes = e.main_file().expect("checked in parse").to_vec();
                let h = FileHeader::parse(&bytes)
                    .with_context(|| format!("{} is too short to be a radio file", e.name))?;
                let id = h.file_id();
                if e.store.file_id() != Some(id) {
                    bail!("{}.hps: Key {:?} does not match the file's ID {id}", e.name, e.store.key);
                }
                Ok(TemplateFile { id, name: e.name.clone(), bytes })
            })
            .collect()
    }

    /// Things that look wrong but do not stop an import: stores for another RCDB version than the
    /// header's, and embedded one-level copies that differ from their own entry.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let own: BTreeMap<&str, &[u8]> =
            self.entries.iter().filter_map(|e| Some((e.name.as_str(), e.main_file()?))).collect();
        for e in &self.entries {
            if e.store.rcdb_version != self.header.rcdb_version {
                out.push(format!(
                    "{}.hps is for RCDB {:?}, the template for {:?}",
                    e.name, e.store.rcdb_version, self.header.rcdb_version
                ));
            }
            for (n, b) in e.files.iter().filter(|(n, _)| *n != e.name) {
                match own.get(n.as_str()) {
                    Some(o) if *o == b.as_slice() => {}
                    Some(_) => out.push(format!("{}.hps holds a copy of {n} that differs from {n}.hps (using {n}.hps)", e.name)),
                    None => out.push(format!("{}.hps holds {n}, which has no entry of its own (ignored)", e.name)),
                }
            }
        }
        out
    }

    /// The `.tpl` bytes in CPS's layout: `tpHeader.data`, then the `.hps` entries in order, each
    /// holding `psHeader.data` then its files; every entry deflated, stamped with `when`.
    pub fn to_bytes(&self, when: DateTime<FixedOffset>) -> Result<Vec<u8>> {
        let mut outer = Vec::new();
        outer.push((TEMPLATE_HEADER.to_string(), self.header.to_xml()));
        for e in &self.entries {
            let mut inner = vec![(STORE_HEADER.to_string(), e.store.to_xml())];
            inner.extend(e.files.iter().cloned());
            outer.push((format!("{}.hps", e.name), zip_entries(&inner, when)?));
        }
        zip_entries(&outer, when)
    }
}

/// All entries of a ZIP archive, in archive order.
fn unzip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut f = archive.by_index(i)?;
        let name = f.name()?.to_string();
        let mut data = Vec::with_capacity(usize::try_from(f.size()).unwrap_or(0).min(1 << 24));
        f.read_to_end(&mut data).with_context(|| format!("unpacking {name}"))?;
        out.push((name, data));
    }
    Ok(out)
}

/// A ZIP archive like DotNetZip writes for CPS: deflate, DOS attributes (archive bit), local
/// DOS time and an NTFS time extra field.
fn zip_entries(entries: &[(String, Vec<u8>)], when: DateTime<FixedOffset>) -> Result<Vec<u8>> {
    let local = when.naive_local();
    let dos = zip::DateTime::from_date_and_time(
        u16::try_from(chrono::Datelike::year(&local)).unwrap_or(1980),
        chrono::Datelike::month(&local) as u8,
        chrono::Datelike::day(&local) as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .map_err(|e| anyhow::anyhow!("time {when} does not fit a ZIP timestamp: {e}"))?;
    let ntfs = filetime(when);
    let options = zip::write::FullFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .system(zip::System::Dos)
        .external_attributes(0x20)
        .last_modified_time(dos)
        .with_ntfs_times(ntfs, ntfs, ntfs)?;
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        w.start_file(name.as_str(), options.clone())?;
        w.write_all(data)?;
    }
    Ok(w.finish()?.into_inner())
}

/// Windows FILETIME (100 ns since 1601-01-01 UTC).
fn filetime(t: DateTime<FixedOffset>) -> u64 {
    let secs = t.timestamp() + 11_644_473_600;
    (secs.max(0) as u64) * 10_000_000 + u64::from(t.timestamp_subsec_nanos() / 100)
}

/// A time as .NET's `XmlSerializer` writes a local `DateTime` (as in CPS's headers): up to seven
/// fraction digits without trailing zeros, and the UTC offset, e.g. `2026-11-18T19:17:51.054579+02:00`.
pub fn dotnet_time(t: DateTime<FixedOffset>) -> String {
    let ticks = t.timestamp_subsec_nanos() / 100;
    let frac = format!("{ticks:07}");
    let frac = frac.trim_end_matches('0');
    let base = t.format("%Y-%m-%dT%H:%M:%S");
    let off = t.format("%:z");
    if frac.is_empty() { format!("{base}{off}") } else { format!("{base}.{frac}{off}") }
}

/// The current local time.
pub fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

// ---------------------------------------------------------------------------------------------
// Import

/// What [`import`] did.
#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    /// Files taken from the template.
    pub from_template: usize,
    /// Files the template lacks, kept from the reference folder (id, name).
    pub kept_from_reference: Vec<(u16, String)>,
    /// Template files the reference folder does not have (id, name); not imported.
    pub ignored: Vec<(u16, String)>,
    /// Differences that do not stop the import (e.g. another serial number or model number).
    pub warnings: Vec<String>,
}

/// What `hyt tpl import` stores next to the files ([`SIDECAR`]): the template's headers and a
/// checksum of each imported file, so [`export`] can reproduce the headers of unchanged files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sidecar {
    pub source: String,
    pub header: TemplateHeader,
    pub stores: Vec<SidecarStore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarStore {
    pub id: u16,
    pub store: ParamStore,
    /// CRC-32 of the file as imported.
    pub crc32: u32,
}

/// Read the template file `path` and [`import`] it for the radio read in `like` into the new
/// folder `out`.
pub fn import_file(path: &Path, like: &SavedRead, out: &Path, layout: &Layout) -> Result<(Template, ImportReport)> {
    let t = Template::read(path)?;
    let name = t.header.name.clone().unwrap_or_default();
    let source = format!("CPS template {} (\"{name}\")", path.display());
    let report = import(&t, like, out, &source, layout)?;
    Ok((t, report))
}

/// Save the template's radio files as a new `hyt` folder at `out`, with the radio identity
/// (model, serial, RCDB version, info) of `like` (a read of the radio it is meant for), so that
/// `hyt write` accepts it for that radio. Refuses a template for another RCDB version or model.
/// Files the template lacks are kept from `like`.
pub fn import(tpl: &Template, like: &SavedRead, out: &Path, source: &str, layout: &Layout) -> Result<ImportReport> {
    let m = &like.manifest;
    let h = &tpl.header;
    match h.rcdb_version.as_deref() {
        Some(v) if v == m.rcdb_version => {}
        other => bail!(
            "the template is for RCDB {}, the reference read uses {}; converting between RCDB versions is not supported",
            other.unwrap_or("(unknown)"),
            m.rcdb_version
        ),
    }
    match (h.product.as_deref(), m.model_name.as_deref()) {
        (Some(t), Some(r)) if t == r => {}
        (t, r) => bail!(
            "the template is for model {}, the reference read is a {}; refusing to import",
            t.unwrap_or("(unknown)"),
            r.unwrap_or("(unknown model)")
        ),
    }
    let mut report = ImportReport { warnings: tpl.warnings(), ..Default::default() };
    let radio_model_number = m.info.iter().find(|e| e.item == format!("0x{:04X}", item::MODEL_CODE)).and_then(|e| e.text.clone());
    if let (Some(t), Some(r)) = (h.model_number.as_deref(), radio_model_number.as_deref())
        && t != r
    {
        report.warnings.push(format!("model number differs: template {t}, radio {r}"));
    }
    if let (Some(t), Some(r)) = (h.serial_number.as_deref(), m.serial_number.as_deref())
        && t != r
    {
        report.warnings.push(format!("the template was saved from radio {t}; importing it for radio {r}"));
    }
    let tfiles = tpl.files()?;
    let mut files = like.files.clone();
    let mut stores = Vec::new();
    for f in &tfiles {
        if let Some(e) = tpl.entries.iter().find(|e| e.name == f.name)
            && e.store.rcdb_version.as_deref() != Some(m.rcdb_version.as_str())
        {
            bail!("{}.hps is for RCDB {:?}, not {}", f.name, e.store.rcdb_version, m.rcdb_version);
        }
        match m.files.iter().find(|r| r.id == f.id) {
            Some(r) if r.name != f.name => bail!("template file {} has ID {}, which is {} on this radio", f.name, f.id, r.name),
            Some(_) => {
                files.insert(f.id, f.bytes.clone());
                report.from_template += 1;
                let store = tpl.entries.iter().find(|e| e.name == f.name).expect("entry").store.clone();
                stores.push(SidecarStore { id: f.id, store, crc32: crc32fast::hash(&f.bytes) });
            }
            None => report.ignored.push((f.id, f.name.clone())),
        }
    }
    for r in &m.files {
        if !tfiles.iter().any(|f| f.id == r.id) {
            report.kept_from_reference.push((r.id, r.name.clone()));
        }
    }
    let sidecar = Sidecar { source: source.to_string(), header: h.clone(), stores };
    let extras = [(SIDECAR, serde_json::to_vec_pretty(&sidecar)?)];
    // The template's files (with the reference radio's identity) are this configuration's base, so
    // bytes outside settings (e.g. unused member slots) stay the template's and export reproduces it.
    let base = SavedRead { manifest: like.manifest.clone(), files: files.clone(), radio: None };
    save_config_with_extras(&base, &files, out, source, layout, &extras)?;
    Ok(report)
}

// ---------------------------------------------------------------------------------------------
// Export

/// Settings for [`build`].
pub struct ExportOptions<'a> {
    /// Template name (default: the imported template's name, else `default_name`).
    pub name: Option<String>,
    pub default_name: String,
    pub description: Option<String>,
    /// Headers saved by [`import`] in the folder, if any.
    pub sidecar: Option<Sidecar>,
    /// A CPS template of the same RCDB version to take each file's `Seqo` from (CPS's names for
    /// the files are in its database, not in `hyt`); used for files the sidecar does not cover.
    pub seqo_from: Option<&'a Template>,
    pub when: DateTime<FixedOffset>,
}

/// What [`build`] did.
#[derive(Debug, Default, Serialize)]
pub struct ExportReport {
    pub files: usize,
    /// Files whose store headers came from the import sidecar unchanged.
    pub headers_kept: usize,
    /// Things the user should know (e.g. files written without a CPS file description).
    pub warnings: Vec<String>,
}

/// Build a template from a saved folder: every file in the form CPS writes
/// ([`prepare_for_write`]), in CPS's entry order, two-level list files carrying their one-level
/// companion (`links`), headers filled from the folder's radio identity.
pub fn build(
    m: &ReadManifest,
    files: &BTreeMap<u16, Vec<u8>>,
    links: &[TableLink],
    opts: &ExportOptions,
) -> Result<(Template, ExportReport)> {
    let now = dotnet_time(opts.when);
    let side = opts.sidecar.as_ref();
    let mut report = ExportReport::default();
    let model_number = m.info.iter().find(|e| e.item == format!("0x{:04X}", item::MODEL_CODE)).and_then(|e| e.text.clone());
    let frequence = m.model_name.as_deref().and_then(|n| n.rsplit_once(' ')).map(|(_, f)| f.to_string());
    let sh = side.map(|s| &s.header);
    let keep = |f: fn(&TemplateHeader) -> &Option<String>, default: &str| {
        sh.and_then(|h| f(h).clone()).unwrap_or_else(|| default.to_string())
    };
    let header = TemplateHeader {
        operate_permission: Some(keep(|h| &h.operate_permission, "All")),
        operate_type: Some(keep(|h| &h.operate_type, "Professional")),
        is_create: Some(keep(|h| &h.is_create, "false")),
        guid: Some(uuid::Uuid::new_v4().to_string()),
        description: Some(opts.description.clone().unwrap_or_else(|| keep(|h| &h.description, ""))),
        product: m.model_name.clone(),
        model_number,
        name: Some(opts.name.clone().or_else(|| sh.and_then(|h| h.name.clone())).unwrap_or_else(|| opts.default_name.clone())),
        rcdb_version: Some(m.rcdb_version.clone()),
        frequence: sh.and_then(|h| h.frequence.clone()).or(frequence),
        created_time: Some(now.clone()),
        product_id: Some(keep(|h| &h.product_id, "0")),
        serial_number: m.serial_number.clone(),
    };

    let prepared: BTreeMap<u16, Vec<u8>> = files
        .iter()
        .map(|(id, b)| Ok((*id, prepare_for_write(b).with_context(|| format!("file {id} is too short to be a radio file"))?)))
        .collect::<Result<_>>()?;
    let name_of = |id: u16| m.files.iter().find(|f| f.id == id).map(|f| f.name.clone());
    if let Some(f) = m.files.iter().find(|f| !prepared.contains_key(&f.id)) {
        bail!("file {} ({}) is missing", f.id, f.name);
    }
    let borrowed: BTreeMap<u16, Option<String>> = opts
        .seqo_from
        .filter(|t| t.header.rcdb_version.as_deref() == Some(m.rcdb_version.as_str()))
        .map(|t| t.entries.iter().filter_map(|e| Some((e.store.file_id()?, e.store.seqo.clone()))).collect())
        .unwrap_or_default();
    let mut no_seqo = Vec::new();

    let mut entries = Vec::new();
    for id in entry_order(&m.files.iter().map(|f| f.id).collect::<Vec<_>>(), links) {
        let name = name_of(id).expect("id from manifest");
        let bytes = prepared[&id].clone();
        let kept = side.and_then(|s| s.stores.iter().find(|s| s.id == id));
        let store = match kept {
            Some(k) if k.crc32 == crc32fast::hash(&bytes) => {
                report.headers_kept += 1;
                k.store.clone()
            }
            _ => {
                let seqo = match (kept, borrowed.get(&id)) {
                    (Some(k), _) => k.store.seqo.clone(),
                    (None, Some(s)) => s.clone(),
                    (None, None) => {
                        no_seqo.push(name.clone());
                        None
                    }
                };
                ParamStore {
                    seqo,
                    rcdb_version: Some(m.rcdb_version.clone()),
                    key: Some(id.to_string()),
                    type_: Some(kept.and_then(|k| k.store.type_.clone()).unwrap_or_else(|| "0".into())),
                    model_number: Some(kept.and_then(|k| k.store.model_number.clone()).unwrap_or_default()),
                    create_time: Some(kept.and_then(|k| k.store.create_time.clone()).unwrap_or_else(|| now.clone())),
                    modified_time: Some(now.clone()),
                }
            }
        };
        let mut files = Vec::new();
        if let Some(l) = links.iter().find(|l| l.two_dim == id)
            && let Some(one) = prepared.get(&l.one_dim)
        {
            files.push((name_of(l.one_dim).expect("id from manifest"), one.clone()));
        }
        files.push((name.clone(), bytes));
        entries.push(TemplateEntry { name, store, files });
    }
    if !no_seqo.is_empty() {
        report.warnings.push(format!(
            "no CPS file description (Seqo) known for {} files (e.g. {}); they are written without one. \
             Pass a CPS template of RCDB {} to take them from.",
            no_seqo.len(),
            no_seqo[0],
            m.rcdb_version
        ));
    }
    report.files = entries.len();
    Ok((Template { header, entries }, report))
}

/// CPS's entry order (observed): the files in read order, except that the one-level companions of
/// two-level list files move to the end, in the order of their partners.
pub fn entry_order(read_order: &[u16], links: &[TableLink]) -> Vec<u16> {
    let moved = |id: u16| links.iter().any(|l| l.one_dim == id && read_order.contains(&l.two_dim));
    let mut out: Vec<u16> = read_order.iter().copied().filter(|&id| !moved(id)).collect();
    for &id in read_order {
        if let Some(l) = links.iter().find(|l| l.two_dim == id && read_order.contains(&l.one_dim)) {
            out.push(l.one_dim);
        }
    }
    out
}

/// Export the folder `folder` as the template file `out` (which must not exist; written whole or
/// not at all).
pub fn export(folder: &Path, out: &Path, rcdb: &RcdbRoot, links: &[TableLink], opts: ExportOptions) -> Result<(Template, ExportReport)> {
    let saved = load_config(folder, rcdb)?;
    export_files(&saved.manifest, &saved.files, folder, out, links, opts)
}

/// Export `files` (with the radio identity in `manifest`) as the new template file `out`, taking
/// the import sidecar from `folder` when `opts` has none.
pub fn export_files(
    manifest: &ReadManifest,
    files: &BTreeMap<u16, Vec<u8>>,
    folder: &Path,
    out: &Path,
    links: &[TableLink],
    opts: ExportOptions,
) -> Result<(Template, ExportReport)> {
    let sidecar = match opts.sidecar.clone() {
        Some(s) => Some(s),
        None => read_sidecar(folder)?,
    };
    let when = opts.when;
    let opts = ExportOptions { sidecar, ..opts };
    let (tpl, report) = build(manifest, files, links, &opts)?;
    let bytes = tpl.to_bytes(when)?;
    write_new_file(out, &bytes)?;
    Ok((tpl, report))
}

/// The headers [`import`] saved in `folder`, if it holds them.
pub fn read_sidecar(folder: &Path) -> Result<Option<Sidecar>> {
    let path = folder.join(SIDECAR);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?))
}

/// The template name to offer for `folder`: the imported template's name, else the folder name.
pub fn suggested_name(folder: &Path) -> String {
    read_sidecar(folder)
        .ok()
        .flatten()
        .and_then(|s| s.header.name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| folder.file_name().map_or_else(|| "hyt".to_string(), |n| n.to_string_lossy().into_owned()))
}

/// A CPS template for the radio in `m` from CPS's `Templates` folder (next to the RCDB data, or in
/// the default CPS install), to take file descriptions (`Seqo`) from.
pub fn cps_template_for(rcdb: &RcdbRoot, m: &ReadManifest) -> Option<Template> {
    [rcdb.path().join("Templates"), Path::new(DEFAULT_CPS_DATA_DIR).join("Templates")]
        .iter()
        .find_map(|d| find_template(d, &m.rcdb_version, m.model_name.as_deref()))
}

/// Export the open configuration including unsaved edits (what the GUI shows) as the new template
/// file `out`, named `name` (default: [`suggested_name`] of the open folder). File descriptions
/// come from a CPS template found by [`cps_template_for`].
pub fn export_workspace(ws: &Workspace, rcdb: &RcdbRoot, out: &Path, name: Option<String>) -> Result<(Template, ExportReport)> {
    let m = ws.manifest();
    let seqo = cps_template_for(rcdb, m);
    let opts = ExportOptions {
        name: name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        default_name: suggested_name(&ws.folder),
        description: None,
        sidecar: None,
        seqo_from: seqo.as_ref(),
        when: now(),
    };
    export_files(m, ws.files(), &ws.folder, out, ws.links(), opts)
}

/// Write `bytes` to the new file `out` via `<out>.partial` and a rename.
fn write_new_file(out: &Path, bytes: &[u8]) -> Result<()> {
    if out.exists() {
        bail!("{} already exists", out.display());
    }
    let name = out.file_name().with_context(|| format!("{} has no file name", out.display()))?;
    let tmp: PathBuf = out.with_file_name(format!("{}.partial", name.to_string_lossy()));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, out).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::Error::new(e).context(format!("renaming {} to {}", tmp.display(), out.display()))
    })
}

/// A CPS template of RCDB `version` in `dir` (e.g. CPS's `Templates` folder), preferring one for
/// `product`, to take file descriptions (`Seqo`) from.
pub fn find_template(dir: &Path, version: &str, product: Option<&str>) -> Option<Template> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tpl")))
        .collect();
    paths.sort();
    let found: Vec<Template> = paths
        .iter()
        .filter_map(|p| Template::read(p).ok())
        .filter(|t| t.header.rcdb_version.as_deref() == Some(version))
        .collect();
    let same_product = found.iter().position(|t| product.is_some() && t.header.product.as_deref() == product);
    let i = same_product.or(if found.is_empty() { None } else { Some(0) })?;
    found.into_iter().nth(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotnet_time_trims_fraction() {
        let t = DateTime::parse_from_rfc3339("2026-11-18T19:17:51.054579+02:00").unwrap();
        assert_eq!(dotnet_time(t), "2026-11-18T19:17:51.054579+02:00");
        let t = DateTime::parse_from_rfc3339("2026-11-18T19:17:15.0656269+02:00").unwrap();
        assert_eq!(dotnet_time(t), "2026-11-18T19:17:15.0656269+02:00");
        let t = DateTime::parse_from_rfc3339("2026-11-18T19:17:15+02:00").unwrap();
        assert_eq!(dotnet_time(t), "2026-11-18T19:17:15+02:00");
    }

    #[test]
    fn header_xml_round_trip() {
        let h = ParamStore {
            seqo: None,
            rcdb_version: Some("V1.5.24.000".into()),
            key: Some("7".into()),
            type_: Some("0".into()),
            model_number: Some(String::new()),
            create_time: Some("2026-11-18T19:18:09.9756565+02:00".into()),
            modified_time: Some("2026-11-18T19:18:09.9756565+02:00".into()),
        };
        let xml = h.to_xml();
        let text = String::from_utf8(xml.clone()).unwrap();
        assert!(text.starts_with("<?xml version=\"1.0\"?>\r\n<ParamStore xmlns:xsi="));
        assert!(text.contains("\r\n  <ModelNumber />\r\n"));
        assert!(!text.contains("Seqo"));
        assert_eq!(ParamStore::parse(&xml).unwrap(), h);
        let t = TemplateHeader { name: Some("a & <b>".into()), ..Default::default() };
        assert_eq!(TemplateHeader::parse(&t.to_xml()).unwrap(), t);
    }

    #[test]
    fn entry_order_moves_one_level_files_to_the_end() {
        let links = [TableLink { two_dim: 6, one_dim: 7 }, TableLink { two_dim: 80, one_dim: 81 }];
        assert_eq!(entry_order(&[18, 3, 6, 7, 80, 81, 8], &links), vec![18, 3, 6, 80, 8, 7, 81]);
        // A one-level file without its partner stays in place.
        assert_eq!(entry_order(&[18, 7, 8], &links), vec![18, 7, 8]);
    }

    #[test]
    fn template_round_trip_in_memory() {
        let mut file = vec![0u8; 24];
        file[0] = 18;
        let tpl = Template {
            header: TemplateHeader { rcdb_version: Some("V1".into()), ..Default::default() },
            entries: vec![TemplateEntry {
                name: "Comm_Basic".into(),
                store: ParamStore { key: Some("18".into()), rcdb_version: Some("V1".into()), ..Default::default() },
                files: vec![("Comm_Basic".into(), file)],
            }],
        };
        let bytes = tpl.to_bytes(now()).unwrap();
        let back = Template::parse(&bytes).unwrap();
        assert_eq!(back, tpl);
        assert_eq!(back.files().unwrap()[0].id, 18);
        assert!(back.warnings().is_empty());
    }
}
