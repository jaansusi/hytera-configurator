//! Making a configuration into one radio's files, for writing: back to the radio it came from, or
//! onto another radio of the same model and band (e.g. a fleet from one template), with any RCDB
//! layout (firmware). The files are rebuilt from the configuration's data with the target radio's
//! own layout ([`crate::config::overlay`]).
//!
//! What belongs to the target radio is kept:
//! - its identity in the folder (model, serial number, RCDB version, info items), so the usual
//!   same-radio check of the write passes for this radio only;
//! - the files CPS never writes (E2EE keys, Bluetooth pairings, radio info; per model profile);
//! - the radio data block of `Comm_Basic` (serial number, frequency range, Bluetooth MAC, licensed
//!   feature flags, …): *inferred* to be the radio's own; CPS's handling of it when writing a
//!   template to another radio has not been recorded;
//! - on another radio, its Radio ID and Radio Alias, unless new ones are given (on the radio the
//!   configuration came from, [`CopyOptions::keep_config_identity`], they are the configuration's).
//!   Radio IP follows Radio ID like CPS ([`crate::workspace::radio_ip`]).
//!
//! The target must be the same model name and model number (frequency band) as the configuration.
//! Changing the target's Radio ID to the configuration's own is refused unless allowed: two radios
//! with one ID break private calls and caller display.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hytera_codeplug::container::prepare_for_write;
use hytera_codeplug::decode::decode;
use hytera_codeplug::rcdb::RcdbRoot;
use hytera_models::{GENERIC_G2, ModelProfile, PROFILES};
use hytera_proto::Session;
use hytera_proto::transport::Transport;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{OverlayReport, overlay};
use crate::workspace::{Workspace, single_field, value_at};
use crate::write_flow::{WriteEvent, WriteReport, WriteSettings, write_prepared};
use crate::{ReadManifest, SavedRead, decode_read, load_config, model_number, read_radio, save_config, save_read};

const BASIC: &str = "Comm_Basic";
const CONFIG: &str = "QEU_C_Config";
/// A field of `Comm_Basic`'s radio data block, which marks that block.
const RADIO_DATA_KEY: &str = "Serial Number";

/// What to set on the target radio.
#[derive(Debug, Clone, Default)]
pub struct CopyOptions {
    /// New Radio ID (as typed, e.g. `"2470002"`); `None` keeps the target radio's own.
    pub radio_id: Option<String>,
    /// New Radio Alias; `None` keeps the target radio's own.
    pub alias: Option<String>,
    /// Allow the target to end up with the configuration's own Radio ID.
    pub allow_duplicate_id: bool,
    /// The target is the radio the configuration is for: write its Radio ID and alias too.
    pub keep_config_identity: bool,
}

/// Who a radio is, as shown to the user.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub serial: Option<String>,
    pub radio_id: Option<String>,
    pub alias: Option<String>,
}

/// What a copy does to the target radio.
#[derive(Debug, Clone, Serialize)]
pub struct CopyPlan {
    /// The radio the configuration came from.
    pub source: Identity,
    /// The target radio before the copy.
    pub before: Identity,
    /// The target radio after the copy.
    pub after: Identity,
    /// Radio IP after the copy (dotted), when known.
    pub radio_ip: Option<String>,
    /// What was kept from the target radio rather than copied.
    pub kept: Vec<String>,
    /// Settings that will differ from what the target radio has now.
    pub changed_settings: usize,
    /// How the configuration's layout was made into the target's (empty for the same layout).
    pub conversion: OverlayReport,
}

impl CopyPlan {
    /// Whether the configuration goes onto another radio than its own.
    pub fn other_radio(&self) -> bool {
        self.before.serial != self.source.serial
    }

    /// Whether the target ends up with the Radio ID of the radio the configuration came from.
    pub fn shares_id_with_source(&self) -> bool {
        self.other_radio() && self.after.radio_id.is_some() && self.after.radio_id == self.source.radio_id
    }
}

fn profile_of(m: &ReadManifest) -> &'static ModelProfile {
    PROFILES.iter().copied().find(|p| p.key == m.profile).unwrap_or(&GENERIC_G2)
}

fn same_or_refuse(what: &str, config: Option<&str>, target: Option<&str>) -> Result<()> {
    match (config, target) {
        (Some(c), Some(t)) if c == t => Ok(()),
        (c, t) => bail!(
            "the configuration is for {what} {}, this radio is {}; it can only go onto the same {what}",
            c.unwrap_or("(unknown)"),
            t.unwrap_or("(unknown)")
        ),
    }
}

/// Dotted form of a stored Radio IP (little-endian u32: first byte is the first field).
pub fn dotted_ip(raw: i64) -> Option<String> {
    let v = u32::try_from(raw).ok()?;
    Some(v.to_le_bytes().map(|b| b.to_string()).join("."))
}

/// Display text of the single-record field `key` of file `file`.
fn shown(ws: &Workspace, files: &[hytera_codeplug::decode::DecodedFile], file: &str, key: &str) -> Option<String> {
    let at = single_field(files, file, key)?;
    let v = value_at(files, &at)?;
    Some(ws.labeler().text(at.file, at.block, key, v))
}

/// Who the radio of `files` (decoded, of the same model as `ws`) with manifest `m` is.
pub(crate) fn identity_of(ws: &Workspace, files: &[hytera_codeplug::decode::DecodedFile], m: &ReadManifest) -> Identity {
    Identity {
        serial: m.serial_number.clone(),
        radio_id: shown(ws, files, CONFIG, "Radio ID"),
        alias: shown(ws, files, BASIC, "Radio Alias"),
    }
}

/// Build the target radio's configuration: `config`'s settings on the radio read in `target`
/// (identity, own files and values kept as described in the module docs). `target_folder` names
/// where `target` came from. The result is a workspace whose files are what will be written.
pub fn prepare(
    config: &SavedRead,
    target: &SavedRead,
    target_folder: &Path,
    rcdb: &RcdbRoot,
    options_path: Option<&Path>,
    opts: &CopyOptions,
) -> Result<(Workspace, CopyPlan)> {
    let (c, t) = (&config.manifest, &target.manifest);
    same_or_refuse("model", c.model_name.as_deref(), t.model_name.as_deref())?;
    same_or_refuse("model number (frequency band)", model_number(c), model_number(t))?;

    let layout = crate::load_layout(target, rcdb)?;
    let config_layout = if c.rcdb_version == layout.version { None } else { Some(crate::load_layout(config, rcdb)?) };
    let config_layout = config_layout.as_ref().unwrap_or(&layout);
    let excluded = profile_of(t).write_excluded_files;
    let config_view = decode_read(config, config_layout)?;
    let made = overlay(&config_view, config_layout, target, &layout, excluded)?;
    let mut files = made.files;
    let mut kept: Vec<String> = t
        .files
        .iter()
        .filter(|f| excluded.contains(&f.id))
        .map(|f| format!("file {} {} (never written)", f.id, f.name))
        .collect();

    // Comm_Basic's radio data block stays the target's.
    if let Some(entry) = t.files.iter().find(|f| f.name == BASIC) {
        let own = decode(&target.files[&entry.id], BASIC, &layout)?;
        let mut copied = decode(&files[&entry.id], BASIC, &layout)?;
        let own_block = own.blocks.iter().find(|b| {
            b.records.len() == 1 && b.records[0].fields.as_ref().is_some_and(|f| f.0.contains_key(RADIO_DATA_KEY))
        });
        if let Some(own_block) = own_block
            && let Some(block) = copied.blocks.iter_mut().find(|b| b.id == own_block.id)
            && block.records != own_block.records
        {
            block.records = own_block.records.clone();
            let rebuilt = hytera_codeplug::decode::apply(&files[&entry.id], &copied, &layout)?;
            let prepared = prepare_for_write(&rebuilt).context("preparing Comm_Basic")?;
            files.insert(entry.id, prepared);
        }
        kept.push("radio data in Comm_Basic (serial number, frequency range, Bluetooth MAC, licensed features)".into());
    }

    // On another radio, its programming passwords (and other secret fields) stay its own: hyt never
    // shows or changes them, so they must not arrive from the radio the configuration came from.
    if !opts.keep_config_identity {
        let mut any = false;
        for f in &t.files {
            if excluded.contains(&f.id) {
                continue;
            }
            let own = decode(&target.files[&f.id], &f.name, &layout)?;
            let mut copied = decode(&files[&f.id], &f.name, &layout)?;
            let mut changed = false;
            for (ob, cb) in own.blocks.iter().zip(&mut copied.blocks) {
                for (orec, crec) in ob.records.iter().zip(&mut cb.records) {
                    let (Some(of), Some(cf)) = (&orec.fields, &mut crec.fields) else { continue };
                    for (k, v) in &of.0 {
                        if crate::view::is_secret(k) && cf.0.get(k) != Some(v) {
                            cf.0.insert(k.clone(), v.clone());
                            changed = true;
                        }
                    }
                }
            }
            if changed {
                let rebuilt = hytera_codeplug::decode::apply(&files[&f.id], &copied, &layout)?;
                files.insert(f.id, prepare_for_write(&rebuilt).with_context(|| format!("preparing {}", f.name))?);
                any = true;
            }
        }
        if any {
            kept.push("programming passwords (this radio's own)".into());
        }
    }

    let manifest: ReadManifest = t.clone();
    let retargeted = SavedRead { manifest, files, radio: Some(Box::new(target.radio_read().clone())) };
    let mut ws = Workspace::from_saved(retargeted, target_folder, rcdb, options_path)?;
    let target_view = decode_read(target, &layout)?;

    // Radio ID and Radio Alias: given, the configuration's (its own radio), or the target's own.
    let identity: &[(&str, &str, &Option<String>)] = if opts.keep_config_identity {
        &[]
    } else {
        &[(CONFIG, "Radio ID", &opts.radio_id), (BASIC, "Radio Alias", &opts.alias)]
    };
    for &(file, key, given) in identity {
        let at = single_field(ws.view(), file, key).with_context(|| format!("this configuration has no {key}"))?;
        match given {
            Some(text) => ws.set_value(&at, text).with_context(|| format!("{key} {text:?}"))?,
            None => {
                let own = value_at(&target_view, &at).with_context(|| format!("this radio has no {key}"))?.clone();
                if value_at(ws.view(), &at) != Some(&own) {
                    ws.set_raw(&at, own).with_context(|| format!("keeping this radio's {key}"))?;
                }
                kept.push(format!("{key} (this radio's own)"));
            }
        }
    }

    let source = identity_of(&ws, &config_view, c);
    let before = identity_of(&ws, &target_view, t);
    let after = identity_of(&ws, ws.view(), t);
    if after.radio_id == source.radio_id && before.serial != source.serial && !opts.allow_duplicate_id {
        bail!(
            "this radio would get Radio ID {}, the same as the radio the configuration came from (serial {}); give it its \
             own Radio ID (two radios with one ID break private calls and caller display), or allow the same ID",
            after.radio_id.as_deref().unwrap_or("?"),
            source.serial.as_deref().unwrap_or("?")
        );
    }
    let radio_ip = single_field(ws.view(), CONFIG, "Radio IP")
        .and_then(|at| value_at(ws.view(), &at).and_then(Value::as_i64))
        .and_then(dotted_ip);
    let mut changed_settings = 0;
    for (old, new) in target_view.iter().zip(ws.view()).filter(|(o, n)| o != n) {
        changed_settings += crate::edit::changes(old, new)?.len();
    }
    let plan = CopyPlan { source, before, after, radio_ip, kept, changed_settings, conversion: made.report };
    Ok((ws, plan))
}

/// [`prepare`] for the radio read into the folder `target`, saving the result as the new folder
/// `out` (for `hyt write`; nothing is sent to a radio).
pub fn prepare_folder(
    config: &SavedRead,
    config_source: &str,
    target: &Path,
    out: &Path,
    rcdb: &RcdbRoot,
    options_path: Option<&Path>,
    opts: &CopyOptions,
) -> Result<CopyPlan> {
    let target_read = load_config(target, rcdb)?;
    let (ws, plan) = prepare(config, &target_read, target, rcdb, options_path, opts)?;
    let source = format!("copied from {config_source} for the radio read in {}", target.display());
    save_config(&ws.saved, ws.files(), out, &source, &ws.layout)?;
    Ok(plan)
}

/// What to copy where.
pub struct CopyJob<'a> {
    /// The configuration to copy (a read or edited folder).
    pub config: &'a SavedRead,
    /// How to name `config` in messages (e.g. its folder).
    pub config_source: &'a str,
    /// New folder for the backup of the target radio, read before anything is written.
    pub backup: &'a Path,
    /// New folder for the configuration made for the target radio (what is written).
    pub out: &'a Path,
    /// Recorded as the backup's source (e.g. the serial port name).
    pub port: &'a str,
    pub options_path: Option<&'a Path>,
}

/// Progress of [`copy_to_radio`].
#[derive(Debug)]
pub enum CopyEvent<'a> {
    /// A step of the backup read or of the write (as in [`crate::write_flow::write_config`]).
    Step(WriteEvent<'a>),
    /// The target's configuration is ready (saved in [`CopyJob::out`]); writing starts next.
    Prepared(&'a CopyPlan),
}

/// Outcome of a successful [`copy_to_radio`].
#[derive(Debug, Clone)]
pub struct CopyReport {
    pub plan: CopyPlan,
    pub write: WriteReport,
    /// The folder that was written ([`CopyJob::out`]).
    pub folder: PathBuf,
}

/// What to do with the radio that answered, decided once it has been read (see
/// [`copy_choosing`]).
#[derive(Debug, Clone)]
pub enum Choice {
    Copy(CopyOptions),
    /// Leave this radio as it is, for this reason.
    Skip(String),
}

/// Outcome of a successful [`copy_choosing`].
#[derive(Debug, Clone)]
pub enum Copied {
    Done(Box<CopyReport>),
    /// The radio was read into the backup and then left unchanged.
    Skipped { reason: String },
}

/// Copy `job.config` to the radio on `session` with `opts`. Changes the radio: callers must have
/// the owner's OK.
///
/// The radio is read once into `job.backup`; its configuration is built with [`prepare`] and saved
/// in `job.out`; then it is written and verified like [`crate::write_flow::write_config`]. If
/// building is refused (another model, a duplicate Radio ID, …) nothing is written.
pub fn copy_to_radio<T: Transport>(
    session: &mut Session<T>,
    rcdb: &RcdbRoot,
    job: &CopyJob<'_>,
    opts: &CopyOptions,
    settings: &WriteSettings,
    on_event: impl FnMut(CopyEvent<'_>),
) -> Result<CopyReport> {
    match copy_choosing(session, rcdb, job, settings, |_| Ok(Choice::Copy(opts.clone())), on_event)? {
        Copied::Done(report) => Ok(*report),
        Copied::Skipped { reason } => bail!("skipped: {reason}"),
    }
}

/// [`copy_to_radio`], with the options chosen by `choose` from the radio's backup once it has
/// been read (e.g. a Radio ID planned for its serial number). When `choose` fails or skips,
/// nothing is written.
pub fn copy_choosing<T: Transport>(
    session: &mut Session<T>,
    rcdb: &RcdbRoot,
    job: &CopyJob<'_>,
    settings: &WriteSettings,
    choose: impl FnOnce(&SavedRead) -> Result<Choice>,
    mut on_event: impl FnMut(CopyEvent<'_>),
) -> Result<Copied> {
    let mut log = crate::write_flow::WriteLog::new(&format!("copy {} to the radio on {}", job.config_source, job.port));
    let result = copy_choosing_logged(session, rcdb, job, settings, choose, |e| {
        match &e {
            CopyEvent::Step(s) => log.event(s),
            CopyEvent::Prepared(plan) => log.event(&WriteEvent::Prepared { plan }),
        }
        on_event(e)
    });
    log.finish(job.backup, &result);
    result
}

fn copy_choosing_logged<T: Transport>(
    session: &mut Session<T>,
    rcdb: &RcdbRoot,
    job: &CopyJob<'_>,
    settings: &WriteSettings,
    choose: impl FnOnce(&SavedRead) -> Result<Choice>,
    mut on_event: impl FnMut(CopyEvent<'_>),
) -> Result<Copied> {
    for (what, p) in [("backup", job.backup), ("output", job.out)] {
        if p.exists() {
            bail!("{what} folder {} already exists; choose a new one", p.display());
        }
    }
    let current = read_radio(session, rcdb, |entry, _, done, total| {
        on_event(CopyEvent::Step(WriteEvent::BackupFile { done, total, entry }))
    })?;
    save_read(&current, job.backup, job.port, rcdb)?;
    on_event(CopyEvent::Step(WriteEvent::BackupSaved { folder: job.backup }));

    let nothing_written = || format!("nothing was written; this radio's backup is in {}", job.backup.display());
    let target = SavedRead::from_result(&current, job.port);
    let opts = match choose(&target).with_context(nothing_written)? {
        Choice::Copy(opts) => opts,
        Choice::Skip(reason) => return Ok(Copied::Skipped { reason }),
    };
    let (ws, plan) =
        prepare(job.config, &target, job.backup, rcdb, job.options_path, &opts).with_context(nothing_written)?;
    let source = format!("copied from {} for radio {}", job.config_source, plan.before.serial.as_deref().unwrap_or("?"));
    save_config(&ws.saved, ws.files(), job.out, &source, &ws.layout).with_context(nothing_written)?;
    let files = ws.files().clone();
    drop(ws);
    on_event(CopyEvent::Prepared(&plan));

    let out_name = job.out.display().to_string();
    let mut step = |e: WriteEvent<'_>| on_event(CopyEvent::Step(e));
    let write = write_prepared(session, rcdb, &current, &files, &out_name, job.backup, settings, &mut step)?;
    Ok(Copied::Done(Box::new(CopyReport { plan, write, folder: job.out.to_path_buf() })))
}
