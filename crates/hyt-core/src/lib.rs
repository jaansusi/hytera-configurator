//! Operations shared by the `hyt` command-line tool and the GUI, usable from tests with any
//! transport: read, save/load folders, decode, edit, write.

pub mod edit;
pub mod refs;
pub mod tables;
pub mod view;
pub mod workspace;
pub mod write_flow;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use hytera_codeplug::container::{CrcCheck, FileHeader, check_crc, prepare_for_write};
use hytera_codeplug::decode::{DecodedFile, decode};
use hytera_codeplug::layout::Layout;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{FileEntry, RcdbRoot, TableLink, write_order};
use hytera_models::{ModelProfile, profile_for};
use hytera_proto::session::item;
use hytera_proto::transport::Transport;
use hytera_proto::{RadioInfo, Session};
use serde::{Deserialize, Serialize};

/// Human-readable labels for information items (partly inferred, see docs/protocol.md).
pub fn item_label(id: u16) -> &'static str {
    match id {
        item::MODEL_CODE => "model code",
        item::FIRMWARE_VERSION => "firmware version",
        item::RCDB_VERSION => "RCDB version",
        0x0003 => "component version",
        item::HARDWARE_VERSION => "hardware version",
        0x000A => "radio ID / band (unconfirmed)",
        item::SERIAL_NUMBER => "serial number",
        item::MODEL_NAME => "model name",
        item::PRODUCT_TYPE => "product type",
        0xFFF6 => "date",
        _ => "",
    }
}

#[derive(Serialize, Deserialize)]
pub struct InfoEntry {
    pub item: String,
    pub label: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub hex: String,
}

pub fn info_entries(info: &RadioInfo) -> Vec<InfoEntry> {
    info.items
        .values()
        .map(|i| InfoEntry {
            item: format!("0x{:04X}", i.item),
            label: item_label(i.item).to_string(),
            status: i.status,
            text: (i.status == 0).then(|| i.text()).flatten(),
            hex: hex(&i.value),
        })
        .collect()
}

#[derive(Serialize, Deserialize)]
pub struct FileRecord {
    pub id: u16,
    pub name: String,
    pub file: String,
    pub size: usize,
    /// `body-crc32` or `body-crc32-before-stale-tail` when the header CRC matches a known rule,
    /// `unknown` otherwise.
    pub crc: String,
}

#[derive(Serialize, Deserialize)]
pub struct ReadManifest {
    pub tool: String,
    pub created_unix: u64,
    pub source: String,
    pub profile: String,
    pub model_name: Option<String>,
    pub serial_number: Option<String>,
    pub firmware_version: Option<String>,
    pub rcdb_version: String,
    #[serde(default)]
    pub info: Vec<InfoEntry>,
    pub files: Vec<FileRecord>,
}

/// Result of a full read.
pub struct ReadResult {
    pub info: RadioInfo,
    pub profile: &'static ModelProfile,
    pub rcdb_version: String,
    pub files: Vec<(FileEntry, Vec<u8>)>,
}

/// Identify the radio, pick its profile and file list, and read every file (CPS's sequence).
/// `on_file(entry, bytes, done, total)` is called after each file.
pub fn read_radio<T: Transport>(
    session: &mut Session<T>,
    rcdb: &RcdbRoot,
    mut on_file: impl FnMut(&FileEntry, &[u8], usize, usize),
) -> Result<ReadResult> {
    let info = session.identify().context("identifying the radio")?;
    let profile = profile_for(&info);
    let Some(rcdb_version) = info.rcdb_version() else {
        bail!("the radio did not report an RCDB version (info item 0x0002)");
    };
    if !profile.is_verified_rcdb(&rcdb_version) {
        tracing::warn!(
            "{} with RCDB {rcdb_version} has not been verified; reading anyway",
            profile.display_name
        );
    }
    let list = rcdb
        .file_list(&rcdb_version)
        .with_context(|| format!("loading the file list for RCDB {rcdb_version}"))?;
    let by_id: BTreeMap<u16, FileEntry> = list.iter().map(|e| (e.id, e.clone())).collect();
    let ids: Vec<u16> = list.iter().map(|e| e.id).collect();
    let (total, mut done) = (ids.len(), 0);
    let mut data = session
        .read_files(&ids, profile.chunk_size, |id, bytes| {
            done += 1;
            on_file(&by_id[&id], bytes, done, total)
        })
        .context("reading files")?;
    let files = list.into_iter().map(|e| { let d = data.remove(&e.id).unwrap_or_default(); (e, d) }).collect();
    Ok(ReadResult { info, profile, rcdb_version, files })
}

/// Save a read to `out` (`NNN_Name.bin` per file plus `manifest.json`). `out` must be new or empty.
pub fn save_read(result: &ReadResult, out: &Path, source: &str) -> Result<ReadManifest> {
    if out.exists() && out.read_dir().with_context(|| format!("reading {}", out.display()))?.next().is_some() {
        bail!("{} already exists and is not empty", out.display());
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut files = Vec::new();
    for (entry, bytes) in &result.files {
        let file = format!("{:03}_{}.bin", entry.id, sanitize(&entry.name));
        write(&out.join(&file), bytes)?;
        let crc = match check_crc(bytes) {
            CrcCheck::Body => "body-crc32",
            CrcCheck::StaleTail { .. } => "body-crc32-before-stale-tail",
            CrcCheck::Unknown => "unknown",
        };
        files.push(FileRecord {
            id: entry.id,
            name: entry.name.clone(),
            file,
            size: bytes.len(),
            crc: crc.to_string(),
        });
    }
    let manifest = ReadManifest {
        tool: format!("hyt {}", env!("CARGO_PKG_VERSION")),
        created_unix: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        source: source.to_string(),
        profile: result.profile.key.to_string(),
        model_name: result.info.model_name(),
        serial_number: result.info.serial_number(),
        firmware_version: result.info.firmware_version(),
        rcdb_version: result.rcdb_version.clone(),
        info: info_entries(&result.info),
        files,
    };
    write(&out.join("manifest.json"), serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    Ok(manifest)
}

/// The option catalogue (CPS's labels, units and allowed values) for RCDB `version`, from
/// `explicit` or [`Options::default_path`]. `Err` explains why none is available; callers then
/// show raw values.
pub fn load_options(explicit: Option<&Path>, version: &str) -> std::result::Result<Options, String> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => Options::default_path(version).ok_or("no per-user data folder (LOCALAPPDATA / XDG_DATA_HOME / HOME)")?,
    };
    match Options::load(&path) {
        Ok(o) if o.rcdb_version == version => Ok(o),
        Ok(o) => Err(format!("{} is for RCDB {}, not {version}", path.display(), o.rcdb_version)),
        Err(e) => Err(format!("no option catalogue ({e}); run tools/export-rcdb-options.ps1")),
    }
}

/// The field layout for a saved read's RCDB version.
pub fn load_layout(saved: &SavedRead, rcdb: &RcdbRoot) -> Result<Layout> {
    rcdb.layout(&saved.manifest.rcdb_version)
        .with_context(|| format!("loading the field layout for RCDB {}", saved.manifest.rcdb_version))
}

/// Decode every file of a saved read into named fields.
pub fn decode_read(saved: &SavedRead, layout: &Layout) -> Result<Vec<DecodedFile>> {
    saved
        .manifest
        .files
        .iter()
        .map(|f| {
            let bytes = &saved.files[&f.id];
            decode(bytes, &f.name, layout).with_context(|| format!("decoding file {} ({})", f.id, f.name))
        })
        .collect()
}

/// A folder saved by [`save_read`]: its manifest and the file contents by ID.
pub struct SavedRead {
    pub manifest: ReadManifest,
    pub files: BTreeMap<u16, Vec<u8>>,
}

pub fn load_read(dir: &Path) -> Result<SavedRead> {
    let path = dir.join("manifest.json");
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: ReadManifest =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut files = BTreeMap::new();
    for f in &manifest.files {
        let p = dir.join(&f.file);
        let bytes = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
        if bytes.len() != f.size {
            bail!("{} is {} bytes, manifest says {}", p.display(), bytes.len(), f.size);
        }
        files.insert(f.id, bytes);
    }
    Ok(SavedRead { manifest, files })
}

/// Save `files` (e.g. after [`edit::apply_edits`]) as a new folder in the same layout as
/// [`save_read`], keeping the radio identity of `base` so `hyt write` accepts it for that radio.
pub fn save_edited(base: &SavedRead, files: &BTreeMap<u16, Vec<u8>>, out: &Path, source: &str) -> Result<ReadManifest> {
    if out.exists() && out.read_dir().with_context(|| format!("reading {}", out.display()))?.next().is_some() {
        bail!("{} already exists and is not empty", out.display());
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut records = Vec::new();
    for f in &base.manifest.files {
        let bytes = files.get(&f.id).with_context(|| format!("file {} missing", f.id))?;
        write(&out.join(&f.file), bytes)?;
        let crc = match check_crc(bytes) {
            CrcCheck::Body => "body-crc32",
            CrcCheck::StaleTail { .. } => "body-crc32-before-stale-tail",
            CrcCheck::Unknown => "unknown",
        };
        records.push(FileRecord { id: f.id, name: f.name.clone(), file: f.file.clone(), size: bytes.len(), crc: crc.into() });
    }
    let m = &base.manifest;
    let manifest = ReadManifest {
        tool: format!("hyt {}", env!("CARGO_PKG_VERSION")),
        created_unix: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        source: source.to_string(),
        profile: m.profile.clone(),
        model_name: m.model_name.clone(),
        serial_number: m.serial_number.clone(),
        firmware_version: m.firmware_version.clone(),
        rcdb_version: m.rcdb_version.clone(),
        info: serde_json::from_value(serde_json::to_value(&m.info)?)?,
        files: records,
    };
    write(&out.join("manifest.json"), serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    Ok(manifest)
}

/// Files to write, in CPS's order, each prepared with [`prepare_for_write`].
pub fn plan_write(
    files: &BTreeMap<u16, Vec<u8>>,
    file_list: &[FileEntry],
    links: &[TableLink],
    profile: &ModelProfile,
) -> Result<Vec<(FileEntry, Vec<u8>)>> {
    let read_order: Vec<u16> = file_list.iter().map(|e| e.id).collect();
    let order = write_order(&read_order, links, profile.write_excluded_files);
    order
        .into_iter()
        .map(|id| {
            let entry = file_list.iter().find(|e| e.id == id).expect("id from list").clone();
            let source = files.get(&id).with_context(|| format!("file {id} ({}) is missing", entry.name))?;
            let prepared = prepare_for_write(source)
                .with_context(|| format!("file {id} ({}) is too short to be a radio file", entry.name))?;
            let header = FileHeader::parse(&prepared).expect("prepared file has a header");
            if header.file_id() != id {
                bail!("file {id} ({}) has ID {} in its header", entry.name, header.file_id());
            }
            Ok((entry, prepared))
        })
        .collect()
}

/// Write planned files with CPS's sequence. Changes the radio's configuration.
pub fn write_radio<T: Transport>(
    session: &mut Session<T>,
    plan: &[(FileEntry, Vec<u8>)],
    chunk_size: u16,
    mut on_file: impl FnMut(&FileEntry),
) -> Result<()> {
    let files: Vec<(u16, &[u8])> = plan.iter().map(|(e, b)| (e.id, b.as_slice())).collect();
    session
        .write_files(&files, chunk_size, |id| {
            if let Some((e, _)) = plan.iter().find(|(e, _)| e.id == id) {
                on_file(e);
            }
        })
        .context("writing files")
}

/// Files whose read-back content differs from what was written (compared after preparing both).
pub fn verify_write(plan: &[(FileEntry, Vec<u8>)], read_back: &[(FileEntry, Vec<u8>)]) -> Vec<FileEntry> {
    plan.iter()
        .filter(|(entry, written)| {
            let back = read_back.iter().find(|(e, _)| e.id == entry.id).map(|(_, b)| b);
            back.and_then(|b| prepare_for_write(b)).as_ref() != Some(written)
        })
        .map(|(e, _)| e.clone())
        .collect()
}

fn write(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

