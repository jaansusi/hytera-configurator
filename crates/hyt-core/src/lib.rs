//! Operations shared by the `hyt` command-line tool and the GUI, usable from tests with any
//! transport: read, save/load folders, decode, edit, write.

pub mod batch;
pub mod config;
pub mod copy;
pub mod edit;
pub mod licence;
pub mod link_rules;
pub mod lists;
pub mod quick;
pub mod refs;
pub mod settings;
pub mod sheet;
pub mod tables;
pub mod tpl;
pub mod view;
pub mod visibility;
pub mod workspace;
pub mod write_flow;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use hytera_codeplug::container::{CrcCheck, FileHeader, check_crc, prepare_for_write};
use hytera_codeplug::decode::{DecodedFile, decode};
use hytera_codeplug::layout::Layout;
use hytera_codeplug::models::ModelRules;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{FileEntry, RcdbRoot, TableLink, write_order};
use hytera_models::{ModelProfile, profile_for};
use hytera_proto::session::item;
use hytera_proto::transport::Transport;
use hytera_proto::{RadioInfo, Session};
use serde::{Deserialize, Serialize};

/// Human-readable labels for information items (CPS's names from `G2CPS.Protocol.dll`
/// `BaseInforItem`, see docs/protocol.md).
pub fn item_label(id: u16) -> &'static str {
    match id {
        item::MODEL_CODE => "model number",
        item::FIRMWARE_VERSION => "firmware version",
        item::RCDB_VERSION => "RCDB version",
        0x0003 => "bootloader (L3) version",
        item::HARDWARE_VERSION => "tuning data version",
        0x0007 => "fixed string version",
        0x0008 => "font version",
        0x0009 => "image version",
        0x000A => "TEI / ESN",
        item::SERIAL_NUMBER => "serial number",
        item::MODEL_NAME => "model name",
        0x0045 => "Bluetooth version",
        0x0065 => "bootloader L2 version",
        0x0071 => "fixed string languages",
        0x0100 => "animation version",
        licence::LICENCE_INFO_ITEM => "installed licences",
        item::PRODUCT_TYPE => "board type",
        0xFFF4 => "voice broadcast version",
        0xFFF6 => "RCDB build date",
        _ => "",
    }
}

#[derive(Clone, Serialize, Deserialize)]
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

#[derive(Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: u16,
    pub name: String,
    pub file: String,
    pub size: usize,
    /// `body-crc32` or `body-crc32-before-stale-tail` when the header CRC matches a known rule,
    /// `unknown` otherwise.
    pub crc: String,
}

#[derive(Clone, Serialize, Deserialize)]
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
    let info = hinted(session.identify(), "identifying the radio")?;
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
    let read = session.read_files(&ids, profile.chunk_size, |id, bytes| {
        done += 1;
        on_file(&by_id[&id], bytes, done, total)
    });
    let mut data = hinted(read, &format!("reading files (after {done} of {total})"))?;
    let files = list.into_iter().map(|e| { let d = data.remove(&e.id).unwrap_or_default(); (e, d) }).collect();
    Ok(ReadResult { info, profile, rcdb_version, files })
}

/// Advice added to errors talking to the radio, by kind.
fn hint(e: &hytera_proto::Error) -> &'static str {
    match e {
        hytera_proto::Error::Timeout { .. } => {
            " (no answer: is the radio switched on, and the programming cable plugged in firmly at both ends?)"
        }
        hytera_proto::Error::Status { .. } => " (the radio refused the request)",
        _ => "",
    }
}

/// `r` with `what` (plus advice for the error's kind) as context.
fn hinted<T>(r: hytera_proto::Result<T>, what: &str) -> Result<T> {
    r.map_err(|e| {
        let h = hint(&e);
        anyhow::Error::new(e).context(format!("{what}{h}"))
    })
}

/// Open the programming cable's serial port, with advice when that fails.
pub fn open_port(name: &str) -> Result<hytera_proto::transport::SerialTransport> {
    hytera_proto::transport::SerialTransport::open(name).with_context(|| {
        format!("opening {name} (if CPS or another program is using the port, close it; check that the cable is plugged in)")
    })
}

/// A simulated radio holding the files and identity of a saved folder (see
/// [`hytera_proto::sim`]), for trying reads and writes without a radio.
pub fn sim_radio(saved: &SavedRead) -> hytera_proto::sim::SimRadio {
    let info = saved
        .manifest
        .info
        .iter()
        .filter_map(|e| {
            let item = u16::from_str_radix(e.item.trim_start_matches("0x"), 16).ok()?;
            let value = (0..e.hex.len() / 2).filter_map(|i| u8::from_str_radix(&e.hex[2 * i..2 * i + 2], 16).ok()).collect();
            Some((item, (e.status, value)))
        })
        .collect();
    hytera_proto::sim::SimRadio::new(info, saved.files.clone())
}

/// A folder being filled: files go into `<name>.partial` next to it, which is renamed to the final
/// name only by [`NewFolder::finish`], so a failed save never leaves a half-written folder. An
/// unfinished `.partial` folder is removed when dropped.
struct NewFolder {
    out: PathBuf,
    tmp: PathBuf,
    finished: bool,
}

impl NewFolder {
    /// `out` must not exist or be empty.
    fn create(out: &Path) -> Result<Self> {
        if out.exists() && out.read_dir().with_context(|| format!("reading {}", out.display()))?.next().is_some() {
            bail!("{} already exists and is not empty", out.display());
        }
        let name = out.file_name().with_context(|| format!("{} has no folder name", out.display()))?;
        let tmp = out.with_file_name(format!("{}.partial", name.to_string_lossy()));
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp).with_context(|| format!("removing the leftover {}", tmp.display()))?;
        }
        std::fs::create_dir_all(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        Ok(NewFolder { out: out.to_path_buf(), tmp, finished: false })
    }

    fn file(&self, name: &str) -> PathBuf {
        self.tmp.join(name)
    }

    fn finish(mut self) -> Result<()> {
        if self.out.exists() {
            std::fs::remove_dir(&self.out).with_context(|| format!("replacing the empty {}", self.out.display()))?;
        }
        std::fs::rename(&self.tmp, &self.out)
            .with_context(|| format!("renaming {} to {}", self.tmp.display(), self.out.display()))?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for NewFolder {
    fn drop(&mut self) {
        if !self.finished {
            let _ = std::fs::remove_dir_all(&self.tmp);
        }
    }
}

/// A configuration folder's data file (see [`ConfigurationFile`]).
pub const CONFIGURATION_FILE: &str = "configuration.json";
/// A configuration folder's sub-folder with the untouched radio read it was made from
/// (`manifest.json` plus `NNN_Name.bin` per file, as the radio sent them).
pub const RADIO_DIR: &str = "radio";
const CONFIGURATION_FORMAT: &str = "hyt-configuration";

/// The data of a configuration: the value of every setting, as decoded with the layout of the
/// radio it was read from. Writing rebuilds any radio's files from it ([`config::overlay`]), so it
/// is not tied to the files of one radio or firmware.
#[derive(Serialize, Deserialize)]
pub struct ConfigurationFile {
    pub format: String,
    pub format_version: u32,
    pub tool: String,
    pub created_unix: u64,
    /// How this configuration came about (read from a radio, edited, copied, …).
    pub source: String,
    /// The RCDB layout whose file, block and field names `files` uses.
    pub layout: String,
    /// The radio it was read from or made for (for reference; `radio/manifest.json` decides).
    pub radio: RadioSummary,
    pub files: Vec<DecodedFile>,
}

#[derive(Serialize, Deserialize)]
pub struct RadioSummary {
    pub model_name: Option<String>,
    pub model_number: Option<String>,
    pub serial_number: Option<String>,
    pub firmware_version: Option<String>,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn crc_rule(bytes: &[u8]) -> &'static str {
    match check_crc(bytes) {
        CrcCheck::Body => "body-crc32",
        CrcCheck::StaleTail { .. } => "body-crc32-before-stale-tail",
        CrcCheck::Unknown => "unknown",
    }
}

impl SavedRead {
    /// A radio read in memory, as [`save_read`] would save it.
    pub fn from_result(result: &ReadResult, source: &str) -> SavedRead {
        let files: Vec<FileRecord> = result
            .files
            .iter()
            .map(|(entry, bytes)| FileRecord {
                id: entry.id,
                name: entry.name.clone(),
                file: format!("{:03}_{}.bin", entry.id, sanitize(&entry.name)),
                size: bytes.len(),
                crc: crc_rule(bytes).to_string(),
            })
            .collect();
        let manifest = ReadManifest {
            tool: format!("hyt {}", env!("CARGO_PKG_VERSION")),
            created_unix: now_unix(),
            source: source.to_string(),
            profile: result.profile.key.to_string(),
            model_name: result.info.model_name(),
            serial_number: result.info.serial_number(),
            firmware_version: result.info.firmware_version(),
            rcdb_version: result.rcdb_version.clone(),
            info: info_entries(&result.info),
            files,
        };
        let files = result.files.iter().map(|(e, b)| (e.id, b.clone())).collect();
        SavedRead { manifest, files, radio: None }
    }

    /// The untouched radio read this configuration was made from (itself, for a read).
    pub fn radio_read(&self) -> &SavedRead {
        self.radio.as_deref().unwrap_or(self)
    }
}

/// Write a radio read's files and `manifest.json` into `dir` (which exists).
fn write_raw(dir: &Path, read: &SavedRead) -> Result<()> {
    let mut manifest = read.manifest.clone();
    for f in &mut manifest.files {
        let bytes = read.files.get(&f.id).with_context(|| format!("file {} missing", f.id))?;
        write(&dir.join(&f.file), bytes)?;
        f.size = bytes.len();
        f.crc = crc_rule(bytes).to_string();
    }
    write(&dir.join("manifest.json"), serde_json::to_string_pretty(&manifest)?.as_bytes())
}

/// Save a radio read as a new configuration folder `out`: `radio/` with the files as read, and
/// [`CONFIGURATION_FILE`] with their data. Without the layout for the radio's RCDB version only
/// `radio/` is saved (it can still be opened and written back). `out` must be new or empty; it
/// appears only once everything is written.
pub fn save_read(result: &ReadResult, out: &Path, source: &str, rcdb: &RcdbRoot) -> Result<ReadManifest> {
    let saved = SavedRead::from_result(result, source);
    let folder = NewFolder::create(out)?;
    let radio = folder.file(RADIO_DIR);
    std::fs::create_dir_all(&radio).with_context(|| format!("creating {}", radio.display()))?;
    write_raw(&radio, &saved)?;
    match rcdb.layout(&saved.manifest.rcdb_version) {
        Ok(layout) => write_configuration(&folder, &saved, &saved.files, source, &layout)?,
        Err(e) => tracing::warn!("saving only the radio's files: no layout for RCDB {} ({e})", saved.manifest.rcdb_version),
    }
    folder.finish()?;
    Ok(saved.manifest)
}

fn write_configuration(
    folder: &NewFolder,
    base: &SavedRead,
    files: &BTreeMap<u16, Vec<u8>>,
    source: &str,
    layout: &Layout,
) -> Result<()> {
    let m = &base.manifest;
    let decoded = m
        .files
        .iter()
        .map(|f| {
            let bytes = files.get(&f.id).with_context(|| format!("file {} missing", f.id))?;
            decode(bytes, &f.name, layout).with_context(|| format!("decoding file {} ({})", f.id, f.name))
        })
        .collect::<Result<Vec<_>>>()?;
    let config = ConfigurationFile {
        format: CONFIGURATION_FORMAT.into(),
        format_version: 1,
        tool: format!("hyt {}", env!("CARGO_PKG_VERSION")),
        created_unix: now_unix(),
        source: source.to_string(),
        layout: m.rcdb_version.clone(),
        radio: RadioSummary {
            model_name: m.model_name.clone(),
            model_number: model_number(m).map(str::to_string),
            serial_number: m.serial_number.clone(),
            firmware_version: m.firmware_version.clone(),
        },
        files: decoded,
    };
    write(&folder.file(CONFIGURATION_FILE), serde_json::to_string_pretty(&config)?.as_bytes())
}

/// The option catalogue (CPS's labels, units and allowed values) for RCDB `version`, from
/// `explicit` or [`Options::default_path`]. `Err` explains why none is available; callers then
/// show raw values.
pub fn load_options(explicit: Option<&Path>, version: &str) -> std::result::Result<Options, String> {
    let data = data_version(version);
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => Options::default_path(&data).ok_or("no per-user data folder (LOCALAPPDATA / XDG_DATA_HOME / HOME)")?,
    };
    match Options::load(&path) {
        Ok(o) if o.rcdb_version == version || o.rcdb_version == data => Ok(o),
        Ok(o) => Err(format!("{} is for RCDB {}, not {version}", path.display(), o.rcdb_version)),
        Err(e) => Err(format!("no option catalogue ({e}); run tools/export-rcdb-options.ps1")),
    }
}

/// The exported CPS data version (option catalogue, screens, model rules, visibility, link rules)
/// used for a radio with RCDB `version`; see [`hytera_codeplug::options::data_version`].
pub fn data_version(version: &str) -> String {
    hytera_codeplug::options::data_version(version)
}

/// Why labels and rules come from another RCDB version than the radio's, when they do.
pub fn data_version_note(version: &str) -> Option<String> {
    let data = data_version(version);
    (data != version).then(|| {
        format!(
            "this radio has RCDB {version}: its fields come from that layout, but names, value ranges and \
             rules are CPS's for RCDB {data} (CPS has one parameter database for all versions)"
        )
    })
}

/// The radio's model number (info item 0x0000, e.g. `55P10B00-1L0BAPU1U1…`) saved with a read.
pub fn model_number(manifest: &ReadManifest) -> Option<&str> {
    manifest
        .info
        .iter()
        .find(|i| i.item == "0x0000" && i.status == 0)
        .and_then(|i| i.text.as_deref())
        .filter(|t| !t.trim().is_empty())
}

/// [`load_options`] for a saved read, narrowed to its radio's model number with CPS's model-number
/// rules (`models.json` next to the catalogue; see `hytera_codeplug::models`), so out-of-band
/// values are refused like CPS does. The note says why the generic ranges are used instead, when
/// the rules or the model number are missing.
pub fn load_options_for_read(
    explicit: Option<&Path>,
    manifest: &ReadManifest,
) -> std::result::Result<(Options, Option<String>), String> {
    let version = &manifest.rcdb_version;
    let options = load_options(explicit, version)?;
    let data = options.rcdb_version.clone();
    let other_version = (data != *version).then(|| data_version_note(version)).flatten();
    let with_version_note = |note: Option<String>| match (other_version.clone(), note) {
        (Some(a), Some(b)) => Some(format!("{a}; {b}")),
        (a, b) => a.or(b),
    };
    let generic = |why: String| format!("{why}; using CPS's generic value ranges, not this model's");
    let Some(model) = model_number(manifest) else {
        let note = generic("the read has no model number (info item 0x0000)".into());
        return Ok((options, with_version_note(Some(note))));
    };
    let path = match explicit {
        Some(p) => Some(p.with_file_name("models.json")),
        None => ModelRules::default_path(&data),
    };
    let Some(path) = path else {
        let note = generic("no per-user data folder for the model-number rules".into());
        return Ok((options, with_version_note(Some(note))));
    };
    match ModelRules::load(&path) {
        Ok(rules) if rules.rcdb_version == data => Ok((options.for_model(&rules, model), with_version_note(None))),
        Ok(rules) => {
            let note = generic(format!("{} is for RCDB {}", path.display(), rules.rcdb_version));
            Ok((options, with_version_note(Some(note))))
        }
        Err(e) => {
            let note = generic(format!("no model-number rules ({e}); run tools/export-rcdb-options.ps1"));
            Ok((options, with_version_note(Some(note))))
        }
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

/// A configuration in memory, as the files of one radio layout: from a folder ([`load_config`]),
/// a radio read ([`load_read`], [`SavedRead::from_result`]) or an edit. `manifest` says which radio
/// and layout the files are for.
#[derive(Clone)]
pub struct SavedRead {
    pub manifest: ReadManifest,
    pub files: BTreeMap<u16, Vec<u8>>,
    /// The radio read the configuration was made from, untouched (a configuration folder's `radio/`).
    /// `None` when `files` are a read themselves (or come from a folder of an earlier hyt version).
    pub radio: Option<Box<SavedRead>>,
}

/// Load a radio read saved as files (`manifest.json` plus `NNN_Name.bin`): a configuration
/// folder's `radio/`, or a folder of an earlier hyt version. See [`load_config`] for folders.
pub fn load_read(dir: &Path) -> Result<SavedRead> {
    let path = dir.join("manifest.json");
    if !path.is_file() && dir.join(CONFIGURATION_FILE).is_file() {
        bail!("{} is a configuration folder; open it with load_config", dir.display());
    }
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
    Ok(SavedRead { manifest, files, radio: None })
}

/// Whether `dir` holds a configuration [`load_config`] can open.
pub fn is_config_folder(dir: &Path) -> bool {
    dir.join(CONFIGURATION_FILE).is_file()
        || dir.join(RADIO_DIR).join("manifest.json").is_file()
        || dir.join("manifest.json").is_file()
}

/// Open a configuration folder: its data ([`CONFIGURATION_FILE`]) made into the files of the radio
/// read in its `radio/` (see [`config::overlay`]), in the form CPS writes them. Folders of earlier
/// hyt versions (files only) open as they are.
pub fn load_config(dir: &Path, rcdb: &RcdbRoot) -> Result<SavedRead> {
    let path = dir.join(CONFIGURATION_FILE);
    if !path.is_file() {
        let radio = dir.join(RADIO_DIR);
        return load_read(if radio.join("manifest.json").is_file() { &radio } else { dir });
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let config: ConfigurationFile = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if config.format != CONFIGURATION_FORMAT || config.format_version != 1 {
        bail!("{}: not a hyt configuration of a known version", path.display());
    }
    let base = load_read(&dir.join(RADIO_DIR))?;
    let base_layout = rcdb
        .layout(&base.manifest.rcdb_version)
        .with_context(|| format!("loading the field layout for RCDB {}", base.manifest.rcdb_version))?;
    let config_layout = if config.layout == base_layout.version {
        None
    } else {
        Some(rcdb.layout(&config.layout).with_context(|| format!("loading the field layout for RCDB {}", config.layout))?)
    };
    let made = config::overlay(&config.files, config_layout.as_ref().unwrap_or(&base_layout), &base, &base_layout, &[])
        .with_context(|| format!("making {} into the files of {}", path.display(), dir.join(RADIO_DIR).display()))?;
    if !made.report.is_exact() {
        tracing::warn!("{}: {}", dir.display(), made.report.lines().join("; "));
    }
    // In the form CPS writes them (header CRC, no stale tail), like a template's files.
    let files: BTreeMap<u16, Vec<u8>> = made
        .files
        .into_iter()
        .map(|(id, bytes)| {
            let prepared = prepare_for_write(&bytes).with_context(|| format!("file {id} is too short"))?;
            Ok((id, prepared))
        })
        .collect::<Result<_>>()?;
    let mut manifest = base.manifest.clone();
    manifest.tool = config.tool;
    manifest.created_unix = config.created_unix;
    manifest.source = config.source;
    for f in &mut manifest.files {
        if let Some(bytes) = files.get(&f.id) {
            f.size = bytes.len();
            f.crc = crc_rule(bytes).to_string();
        }
    }
    Ok(SavedRead { manifest, files, radio: Some(Box::new(base)) })
}

/// [`load_config`] with the RCDB data from `HYT_RCDB` or its default location (a CPS install or
/// the per-user copy); for tools and tests without an `--rcdb` choice.
pub fn load_config_default(dir: &Path) -> Result<SavedRead> {
    let root = std::env::var_os("HYT_RCDB").map_or_else(hytera_codeplug::rcdb::default_root, PathBuf::from);
    load_config(dir, &RcdbRoot::new(root))
}

/// Save `files` (the configuration `base` after edits, in `base`'s layout) as a new configuration
/// folder `out`: [`CONFIGURATION_FILE`] with their data, and `radio/` with the radio read `base`
/// was made from ([`SavedRead::radio_read`]). Returns the configuration's manifest.
pub fn save_config(
    base: &SavedRead,
    files: &BTreeMap<u16, Vec<u8>>,
    out: &Path,
    source: &str,
    layout: &Layout,
) -> Result<ReadManifest> {
    save_config_with_extras(base, files, out, source, layout, &[])
}

/// [`save_config`] plus extra files (`name`, contents) written into the same new folder.
pub(crate) fn save_config_with_extras(
    base: &SavedRead,
    files: &BTreeMap<u16, Vec<u8>>,
    out: &Path,
    source: &str,
    layout: &Layout,
    extras: &[(&str, Vec<u8>)],
) -> Result<ReadManifest> {
    if layout.version != base.manifest.rcdb_version {
        bail!("saving: the layout is RCDB {}, the configuration is for RCDB {}", layout.version, base.manifest.rcdb_version);
    }
    let folder = NewFolder::create(out)?;
    for (name, bytes) in extras {
        write(&folder.file(name), bytes)?;
    }
    let radio = folder.file(RADIO_DIR);
    std::fs::create_dir_all(&radio).with_context(|| format!("creating {}", radio.display()))?;
    write_raw(&radio, base.radio_read())?;
    write_configuration(&folder, base, files, source, layout)?;
    folder.finish()?;
    let mut manifest = base.manifest.clone();
    manifest.tool = format!("hyt {}", env!("CARGO_PKG_VERSION"));
    manifest.created_unix = now_unix();
    manifest.source = source.to_string();
    for f in &mut manifest.files {
        if let Some(bytes) = files.get(&f.id) {
            f.size = bytes.len();
            f.crc = crc_rule(bytes).to_string();
        }
    }
    Ok(manifest)
}

/// Set `Comm_Basic`'s `Programming Time` (ASCII `YYYY-MM-DD`) to `date`, as CPS does on every write
/// (observed: docs/protocol.md, info item 0xFFF6). Returns whether the file had the field.
pub fn stamp_programming_date(saved: &SavedRead, files: &mut BTreeMap<u16, Vec<u8>>, layout: &Layout, date: &str) -> Result<bool> {
    let Some(entry) = saved.manifest.files.iter().find(|f| f.name == "Comm_Basic") else { return Ok(false) };
    let Some(bytes) = files.get(&entry.id) else { return Ok(false) };
    let mut decoded = decode(bytes, &entry.name, layout)?;
    let mut found = false;
    for b in &mut decoded.blocks {
        for r in &mut b.records {
            if let Some(f) = &mut r.fields
                && f.0.contains_key("Programming Time")
            {
                f.0.insert("Programming Time".into(), serde_json::Value::String(date.to_string()));
                found = true;
            }
        }
    }
    if found {
        let stamped = hytera_codeplug::decode::apply(bytes, &decoded, layout)?;
        files.insert(entry.id, stamped);
    }
    Ok(found)
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

