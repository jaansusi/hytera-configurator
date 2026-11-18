//! The guarded write sequence shared by `hyt write` and the GUI: fresh backup → same-radio check →
//! write the prepared files in CPS's order → wait for the radio's restart → read back and verify.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};
use hytera_codeplug::container::prepare_for_write;
use hytera_codeplug::rcdb::{FileEntry, RcdbRoot};
use hytera_proto::Session;
use hytera_proto::transport::Transport;

use crate::{ReadManifest, ReadResult, SavedRead, plan_write, read_radio, save_read, verify_write, write_radio};

/// Tunables of [`write_config`].
#[derive(Debug, Clone)]
pub struct WriteSettings {
    /// Write even when the model's profile does not have `write_supported`.
    pub allow_unverified: bool,
    /// Pause before (and between) read-back attempts after the write.
    pub restart_pause: Duration,
    /// Read-back attempts before giving up.
    pub restart_attempts: u32,
}

impl Default for WriteSettings {
    fn default() -> Self {
        // The BP515 stops answering for a few seconds after a write (observed 2026-11-18, most
        // likely a restart); 10 × 3 s covered it with a wide margin.
        WriteSettings { allow_unverified: false, restart_pause: Duration::from_secs(3), restart_attempts: 10 }
    }
}

/// Progress of [`write_config`], in order.
#[derive(Debug)]
pub enum WriteEvent<'a> {
    /// Reading the radio's current configuration into the backup folder.
    BackupFile { done: usize, total: usize, entry: &'a FileEntry },
    BackupSaved { folder: &'a Path },
    /// The radio is the one the configuration came from; writing starts.
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
}

/// Whether the radio that answered (`current`) is the one `manifest` was read from: same model
/// name, serial number and RCDB version.
pub fn is_same_radio(current: &ReadResult, manifest: &ReadManifest) -> bool {
    current.info.model_name() == manifest.model_name
        && current.info.serial_number() == manifest.serial_number
        && current.rcdb_version == manifest.rcdb_version
}

/// What to write and where the backup goes.
pub struct WriteJob<'a> {
    /// The configuration to write (a read or edited folder).
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
/// Before writing, the radio is read in full into the new folder `job.backup`; writing is refused
/// if the radio is a different one, or if the model's profile is not `write_supported` and
/// `settings.allow_unverified` is off. After writing, the radio is read back (with retries while
/// it restarts) and every written file is compared.
pub fn write_config<T: Transport>(
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
    save_read(&current, backup, port)?;
    on_event(WriteEvent::BackupSaved { folder: backup });

    let want = &saved.manifest;
    if !is_same_radio(&current, want) {
        bail!(
            "refusing to write: the radio is {:?} serial {:?} RCDB {}, but {source} came from {:?} serial {:?} RCDB {}",
            current.info.model_name(),
            current.info.serial_number(),
            current.rcdb_version,
            want.model_name,
            want.serial_number,
            want.rcdb_version
        );
    }
    let profile = current.profile;
    if !profile.write_supported && !settings.allow_unverified {
        bail!(
            "writing has not been verified for {} yet; allow unverified writes to write anyway (backup saved in {})",
            profile.display_name,
            backup.display()
        );
    }

    let list = rcdb.file_list(&current.rcdb_version)?;
    let links = rcdb.table_links()?;
    let plan = plan_write(&saved.files, &list, &links, profile)?;
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
    write_radio(session, &plan, profile.chunk_size, |entry| {
        done += 1;
        on_event(WriteEvent::Wrote { done, total, entry });
    })?;

    on_event(WriteEvent::WaitingForRestart);
    let after = read_back_after_restart(session, rcdb, settings, &mut on_event)?;
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
    Ok(WriteReport { written: plan.len(), differing, backup: backup.to_path_buf() })
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
