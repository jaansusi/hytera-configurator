//! The guarded write sequence shared by `hyt write`, copying and the GUI: fresh backup → the
//! configuration made into this radio's files ([`crate::copy::prepare`], any RCDB layout) → write in
//! CPS's order → wait for the radio's restart → read back and verify.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hytera_codeplug::container::prepare_for_write;
use hytera_codeplug::rcdb::{FileEntry, RcdbRoot};
use hytera_proto::Session;
use hytera_proto::transport::Transport;

use crate::copy::{CopyOptions, CopyPlan};
use crate::{ReadManifest, ReadResult, SavedRead, plan_write, read_radio, save_read, verify_write, write_radio};

/// Tunables of [`write_config`].
#[derive(Debug, Clone)]
pub struct WriteSettings {
    /// Write even when writing has not been verified for the radio's model and RCDB version
    /// ([`hytera_models::ModelProfile::write_verified`]).
    pub allow_unverified: bool,
    /// Pause before (and between) read-back attempts after the write.
    pub restart_pause: Duration,
    /// Read-back attempts before giving up.
    pub restart_attempts: u32,
    /// Stamp this date (`YYYY-MM-DD`) as the programming date, as CPS does on every write.
    pub programming_date: Option<String>,
}

impl Default for WriteSettings {
    fn default() -> Self {
        // The BP515 stops answering for a few seconds after a write (observed 2026-11-18, most
        // likely a restart); 10 × 3 s covered it with a wide margin.
        WriteSettings { allow_unverified: false, restart_pause: Duration::from_secs(3), restart_attempts: 10, programming_date: None }
    }
}

/// Progress of [`write_config`], in order.
#[derive(Debug)]
pub enum WriteEvent<'a> {
    /// Reading the radio's current configuration into the backup folder.
    BackupFile { done: usize, total: usize, entry: &'a FileEntry },
    BackupSaved { folder: &'a Path },
    /// The configuration has been made into this radio's files (what is kept, what is converted).
    Prepared { plan: &'a CopyPlan },
    /// Writing starts.
    Writing { files: usize, differing: usize },
    Wrote { done: usize, total: usize, entry: &'a FileEntry },
    WaitingForRestart,
    NotReadyYet { attempt: u32 },
    ReadBackFile { done: usize, total: usize, entry: &'a FileEntry },
    Verified { files: usize },
}

/// Outcome of a successful [`write_config`].
#[derive(Debug, Clone)]
pub struct WriteReport {
    pub written: usize,
    /// Files whose content differed from what the radio held before the write.
    pub differing: usize,
    pub backup: PathBuf,
    /// How the configuration was made into this radio's files ([`write_config`] only).
    pub prepared: Option<CopyPlan>,
}

/// Whether the radio that answered (`current`) is the one `manifest` is for: same model name,
/// serial number and RCDB version.
pub fn is_same_radio(current: &ReadResult, manifest: &ReadManifest) -> bool {
    current.info.model_name() == manifest.model_name
        && current.info.serial_number() == manifest.serial_number
        && current.rcdb_version == manifest.rcdb_version
}

/// What to write and where the backup goes.
pub struct WriteJob<'a> {
    /// The configuration to write (from any radio of the same model and band, any RCDB version).
    pub saved: &'a SavedRead,
    /// How to name `saved` in messages (e.g. its folder).
    pub source: &'a str,
    /// New folder for the fresh backup taken right before writing.
    pub backup: &'a Path,
    /// Recorded as the backup's source (e.g. the serial port name).
    pub port: &'a str,
}

/// Write `job.saved` to the radio on `session`. Changes the radio's configuration: callers must
/// have the owner's OK.
///
/// Before writing, the radio is read in full into the new folder `job.backup`. The configuration
/// is then made into this radio's files with its own layout ([`crate::copy::prepare`]): on the radio
/// it came from, everything is the configuration's; on another radio of the same model and band, the
/// radio keeps its identity, own data, Radio ID and alias. Writing is refused if writing has not
/// been verified for the radio's model and RCDB version and `settings.allow_unverified` is off.
/// After writing, the radio is read back (with retries while it restarts) and every written file
/// is compared.
pub fn write_config<T: Transport>(
    session: &mut Session<T>,
    rcdb: &RcdbRoot,
    job: &WriteJob<'_>,
    settings: &WriteSettings,
    mut on_event: impl FnMut(WriteEvent<'_>),
) -> Result<WriteReport> {
    let mut log = WriteLog::new(&format!("write {} to the radio on {}", job.source, job.port));
    let result = write_config_logged(session, rcdb, job, settings, |e| {
        log.event(&e);
        on_event(e)
    });
    log.finish(job.backup, &result);
    result
}

fn write_config_logged<T: Transport>(
    session: &mut Session<T>,
    rcdb: &RcdbRoot,
    job: &WriteJob<'_>,
    settings: &WriteSettings,
    mut on_event: impl FnMut(WriteEvent<'_>),
) -> Result<WriteReport> {
    let WriteJob { saved, source, backup, port } = *job;
    if backup.exists() {
        bail!("backup folder {} already exists; choose a new one", backup.display());
    }
    let current = read_radio(session, rcdb, |entry, _, done, total| {
        on_event(WriteEvent::BackupFile { done, total, entry })
    })?;
    save_read(&current, backup, port, rcdb)?;
    on_event(WriteEvent::BackupSaved { folder: backup });

    let nothing_written = || format!("nothing was written; this radio's backup is in {}", backup.display());
    let target = SavedRead::from_result(&current, port);
    let same = saved.manifest.serial_number.is_some() && saved.manifest.serial_number == target.manifest.serial_number;
    // On another radio it keeps its own Radio ID, even when that equals the configuration's radio's
    // (the plan says so; giving new IDs is what copying and batches are for).
    let opts = CopyOptions { keep_config_identity: same, allow_duplicate_id: !same, ..CopyOptions::default() };
    let (ws, plan) = crate::copy::prepare(saved, &target, backup, rcdb, None, &opts).with_context(nothing_written)?;
    on_event(WriteEvent::Prepared { plan: &plan });
    let files = ws.files().clone();
    drop(ws);
    let mut report = write_prepared(session, rcdb, &current, &files, source, backup, settings, &mut on_event)?;
    report.prepared = Some(plan);
    Ok(report)
}

/// Write `files` (already made for the radio read as `current`, e.g. by
/// [`crate::copy::prepare`]) and verify them; the radio's backup is in `backup`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_prepared<T: Transport>(
    session: &mut Session<T>,
    rcdb: &RcdbRoot,
    current: &ReadResult,
    files: &BTreeMap<u16, Vec<u8>>,
    source: &str,
    backup: &Path,
    settings: &WriteSettings,
    on_event: &mut impl FnMut(WriteEvent<'_>),
) -> Result<WriteReport> {
    let profile = current.profile;
    if !profile.write_verified(&current.rcdb_version) && !settings.allow_unverified {
        bail!(
            "writing has not been verified for {} with RCDB {} yet; allow unverified writes to write anyway \
             (nothing was written; backup saved in {})",
            profile.display_name,
            current.rcdb_version,
            backup.display()
        );
    }
    tracing::info!("writing {source}");

    let list = rcdb.file_list(&current.rcdb_version)?;
    let links = rcdb.table_links()?;
    let mut files = files.clone();
    if let Some(date) = &settings.programming_date {
        let layout = rcdb.layout(&current.rcdb_version)?;
        let target = SavedRead::from_result(current, "");
        crate::stamp_programming_date(&target, &mut files, &layout, date)?;
    }
    let plan = plan_write(&files, &list, &links, profile)?;
    let differing = plan
        .iter()
        .filter(|(e, bytes)| {
            let now = current.files.iter().find(|(c, _)| c.id == e.id).map(|(_, b)| b);
            now.and_then(|b| prepare_for_write(b)).as_ref() != Some(bytes)
        })
        .count();
    on_event(WriteEvent::Writing { files: plan.len(), differing });
    let total = plan.len();
    let mut done = 0;
    let written = write_radio(session, &plan, profile.chunk_size, |entry| {
        done += 1;
        on_event(WriteEvent::Wrote { done, total, entry });
    });
    if let Err(e) = written {
        let advice = if done == 0 {
            format!(
                "writing failed before any file was complete, so the radio most likely still has its previous \
                 settings; switch it off and on and read it to check (backup in {})",
                backup.display()
            )
        } else {
            format!(
                "writing stopped after {done} of {total} files: the radio may now hold a mix of old and new \
                 settings. Write again, or write the backup in {} to go back",
                backup.display()
            )
        };
        return Err(e.context(advice));
    }

    on_event(WriteEvent::WaitingForRestart);
    let after = read_back_after_restart(session, rcdb, settings, on_event).with_context(|| {
        format!(
            "all {total} files were sent, but the radio could not be read back to verify them; read it again to \
             check (backup in {})",
            backup.display()
        )
    })?;
    let bad = verify_write(&plan, &after.files);
    if !bad.is_empty() {
        let names: Vec<String> = bad.iter().map(|e| format!("{} {}", e.id, e.name)).collect();
        bail!(
            "{} files did not read back as written ({}); the backup is in {}",
            bad.len(),
            names.join(", "),
            backup.display()
        );
    }
    on_event(WriteEvent::Verified { files: plan.len() });
    Ok(WriteReport { written: plan.len(), differing, backup: backup.to_path_buf(), prepared: None })
}

/// What happened during a write, saved as [`WRITE_LOG`] in its backup folder (also when it fails,
/// so the error survives a closed dialog).
pub(crate) struct WriteLog {
    started: std::time::Instant,
    lines: Vec<String>,
}

/// File name of the log of a write, in its backup folder.
pub const WRITE_LOG: &str = "write-log.txt";

impl WriteLog {
    pub(crate) fn new(what: &str) -> Self {
        let when = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        WriteLog { started: std::time::Instant::now(), lines: vec![format!("{when} {what} (hyt {})", env!("CARGO_PKG_VERSION"))] }
    }

    pub(crate) fn line(&mut self, text: String) {
        self.lines.push(format!("{:>8.1}s {text}", self.started.elapsed().as_secs_f32()));
    }

    pub(crate) fn event(&mut self, e: &WriteEvent<'_>) {
        let text = match e {
            WriteEvent::BackupFile { .. } | WriteEvent::ReadBackFile { .. } => return,
            WriteEvent::Prepared { plan } => {
                let mut t = format!(
                    "prepared for serial {} (configuration from serial {}), {} settings differ",
                    plan.before.serial.as_deref().unwrap_or("?"),
                    plan.source.serial.as_deref().unwrap_or("?"),
                    plan.changed_settings
                );
                for k in &plan.kept {
                    t.push_str(&format!("\n           kept: {k}"));
                }
                for l in plan.conversion.lines() {
                    t.push_str(&format!("\n           {l}"));
                }
                t
            }
            WriteEvent::Wrote { done, total, entry } => format!("wrote {done}/{total}: file {} {}", entry.id, entry.name),
            other => format!("{other:?}"),
        };
        self.line(text);
    }

    pub(crate) fn finish<R>(mut self, backup: &Path, result: &Result<R>) {
        match result {
            Ok(_) => self.line("finished".into()),
            Err(e) => self.line(format!("FAILED: {e:#}")),
        }
        if backup.is_dir() {
            let path = backup.join(WRITE_LOG);
            if let Err(e) = std::fs::write(&path, self.lines.join("\n") + "\n") {
                tracing::warn!("could not save {}: {e}", path.display());
            }
        }
    }
}

/// The radio acknowledges requests during its restart but does not answer them; retry the
/// (read-only) read until it does.
fn read_back_after_restart<T: Transport>(
    session: &mut Session<T>,
    rcdb: &RcdbRoot,
    settings: &WriteSettings,
    on_event: &mut impl FnMut(WriteEvent<'_>),
) -> Result<ReadResult> {
    std::thread::sleep(settings.restart_pause);
    let attempts = settings.restart_attempts.max(1);
    for attempt in 1..=attempts {
        let result = read_radio(session, rcdb, |entry, _, done, total| {
            on_event(WriteEvent::ReadBackFile { done, total, entry })
        });
        match result {
            Ok(result) => return Ok(result),
            Err(e) if attempt < attempts => {
                tracing::info!("radio not ready yet (attempt {attempt}): {e:#}");
                on_event(WriteEvent::NotReadyYet { attempt });
                std::thread::sleep(settings.restart_pause);
            }
            Err(e) => return Err(e.context("the radio did not answer after the write")),
        }
    }
    unreachable!("the loop returns on the last attempt")
}
