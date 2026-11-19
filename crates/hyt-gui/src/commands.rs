//! Commands the web page calls (`window.__TAURI__.core.invoke`). Errors are returned as text.
//!
//! Talking to the radio runs on a blocking thread and reports progress as `progress` events; only
//! one radio operation runs at a time.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use hyt_core::tables::{self, ChangeView, ChannelOption, DetailField, RecordRef, TABLE_IDS, Table};
use hyt_core::workspace::{ChannelKind, FieldRef, Workspace};
use hyt_core::write_flow::{WriteEvent, WriteJob, WriteSettings, write_config};
use hyt_core::{load_read, read_radio as read_all, save_read};
use hytera_codeplug::rcdb::RcdbRoot;
use hytera_proto::Session;
use hytera_proto::sim::SharedSim;
use hytera_proto::transport::{ReplayTransport, Transport, available_ports};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

type CmdResult<T> = Result<T, String>;

fn text(e: anyhow::Error) -> String {
    format!("{e:#}")
}

pub struct AppState {
    ws: Mutex<Option<Workspace>>,
    busy: AtomicBool,
    rcdb_dir: PathBuf,
    options_path: Option<PathBuf>,
    startup_folder: Option<PathBuf>,
    dev: bool,
    /// A simulated radio (from `HYT_GUI_SIM`), offered as a port, and the folder it was made from.
    sim: Option<(SharedSim, String)>,
}

impl AppState {
    pub fn new(
        rcdb_dir: PathBuf,
        options_path: Option<PathBuf>,
        startup_folder: Option<PathBuf>,
        dev: bool,
        sim: Option<(SharedSim, String)>,
    ) -> Self {
        AppState { ws: Mutex::new(None), busy: AtomicBool::new(false), rcdb_dir, options_path, startup_folder, dev, sim }
    }

    fn rcdb(&self) -> RcdbRoot {
        RcdbRoot::new(&self.rcdb_dir)
    }

    fn open(&self, folder: &Path) -> anyhow::Result<Workspace> {
        Workspace::open(folder, &self.rcdb(), self.options_path.as_deref())
    }

    fn with_ws<T>(&self, f: impl FnOnce(&mut Workspace) -> anyhow::Result<T>) -> CmdResult<T> {
        let mut guard = self.ws.lock().map_err(|_| "internal error: state lock poisoned".to_string())?;
        let ws = guard.as_mut().ok_or("open a configuration first")?;
        f(ws).map_err(text)
    }
}

/// Holds the "radio busy" flag until dropped.
struct Busy(AppHandle);

impl Busy {
    fn acquire(app: &AppHandle) -> CmdResult<Busy> {
        if app.state::<AppState>().busy.swap(true, Ordering::SeqCst) {
            return Err("another radio operation is still running".into());
        }
        Ok(Busy(app.clone()))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.state::<AppState>().busy.store(false, Ordering::SeqCst);
    }
}

#[derive(Serialize)]
pub struct AppInfo {
    rcdb_dir: String,
    rcdb_found: bool,
    startup_folder: Option<String>,
    /// Offer replaying recordings instead of a serial port.
    dev: bool,
    /// Folder the simulated radio was made from, when there is one.
    sim: Option<String>,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        rcdb_dir: state.rcdb_dir.display().to_string(),
        rcdb_found: state.rcdb_dir.join("RcdbJsons").is_dir(),
        startup_folder: state.startup_folder.as_ref().map(|p| p.display().to_string()),
        dev: state.dev,
        sim: state.sim.as_ref().map(|(_, folder)| folder.clone()),
    }
}

#[derive(Serialize)]
pub struct Port {
    name: String,
    description: String,
}

#[tauri::command]
pub fn list_ports() -> CmdResult<Vec<Port>> {
    let ports = available_ports().map_err(|e| e.to_string())?;
    Ok(ports.into_iter().map(|(name, description)| Port { name, description }).collect())
}

#[derive(Serialize)]
pub struct TableInfo {
    id: String,
    title: String,
    rows: usize,
    capacity: usize,
}

/// What the header and navigation show.
#[derive(Serialize)]
pub struct Overview {
    folder: String,
    source: String,
    model: Option<String>,
    serial: Option<String>,
    firmware: Option<String>,
    rcdb_version: String,
    /// Why values are shown raw (no option catalogue), if they are.
    options_note: Option<String>,
    /// CPS's screen structure (ui.json) is loaded, so the settings view is available.
    has_settings: bool,
    changes: usize,
    tables: Vec<TableInfo>,
}

fn overview(ws: &Workspace) -> Overview {
    let m = ws.manifest();
    Overview {
        folder: ws.folder.display().to_string(),
        source: m.source.clone(),
        model: m.model_name.clone(),
        serial: m.serial_number.clone(),
        firmware: m.firmware_version.clone(),
        rcdb_version: m.rcdb_version.clone(),
        options_note: ws.options_note.clone(),
        has_settings: ws.ui.is_some(),
        // As listed in the Changes view (a zone member's type and number count once).
        changes: tables::change_list(ws).len(),
        tables: TABLE_IDS
            .iter()
            .filter_map(|id| tables::table(ws, id))
            .map(|t| TableInfo { id: t.id, title: t.title, rows: t.rows.len(), capacity: t.capacity })
            .collect(),
    }
}

#[tauri::command]
pub fn open_folder(state: State<'_, AppState>, path: String) -> CmdResult<Overview> {
    let ws = state.open(Path::new(&path)).map_err(text)?;
    let o = overview(&ws);
    *state.ws.lock().map_err(|_| "internal error: state lock poisoned")? = Some(ws);
    Ok(o)
}

#[tauri::command]
pub fn get_overview(state: State<'_, AppState>) -> CmdResult<Option<Overview>> {
    let guard = state.ws.lock().map_err(|_| "internal error: state lock poisoned")?;
    Ok(guard.as_ref().map(overview))
}

#[tauri::command]
pub fn get_table(state: State<'_, AppState>, id: String) -> CmdResult<Table> {
    state.with_ws(|ws| tables::table(ws, &id).with_context(|| format!("this configuration has no {id} table")))
}

#[tauri::command]
pub fn get_detail(state: State<'_, AppState>, record: RecordRef) -> CmdResult<Vec<DetailField>> {
    state.with_ws(|ws| Ok(tables::record_detail(ws, &record)))
}

#[tauri::command]
pub fn set_value(state: State<'_, AppState>, field: FieldRef, text: String) -> CmdResult<Overview> {
    state.with_ws(|ws| {
        ws.set_value(&field, &text)?;
        Ok(overview(ws))
    })
}

#[tauri::command]
pub fn revert_value(state: State<'_, AppState>, field: FieldRef) -> CmdResult<Overview> {
    state.with_ws(|ws| {
        ws.revert(&field)?;
        Ok(overview(ws))
    })
}

#[tauri::command]
pub fn discard_changes(state: State<'_, AppState>) -> CmdResult<Overview> {
    state.with_ws(|ws| {
        ws.discard();
        Ok(overview(ws))
    })
}

/// Add a channel (from CPS's template); returns the overview and the new row's position.
#[tauri::command]
pub fn add_channel(state: State<'_, AppState>, kind: ChannelKind) -> CmdResult<(Overview, usize)> {
    state.with_ws(|ws| {
        let position = ws.add_channel(kind)?;
        Ok((overview(ws), position))
    })
}

#[tauri::command]
pub fn remove_channel(state: State<'_, AppState>, kind: ChannelKind, position: usize) -> CmdResult<Overview> {
    state.with_ws(|ws| {
        ws.remove_channel(kind, position)?;
        Ok(overview(ws))
    })
}

/// The settings topics (see `hyt_core::settings`).
#[tauri::command]
pub fn get_settings_index(state: State<'_, AppState>) -> CmdResult<Vec<hyt_core::settings::TopicInfo>> {
    state.with_ws(|ws| Ok(hyt_core::settings::index(ws)))
}

#[tauri::command]
pub fn get_settings_topic(state: State<'_, AppState>, id: String) -> CmdResult<hyt_core::settings::Topic> {
    state.with_ws(|ws| hyt_core::settings::topic(ws, &id).with_context(|| format!("no settings topic {id:?}")))
}

#[tauri::command]
pub fn search_settings(state: State<'_, AppState>, query: String) -> CmdResult<Vec<hyt_core::settings::Topic>> {
    state.with_ws(|ws| Ok(hyt_core::settings::search(ws, &query)))
}

/// Channels that can be put into a zone.
#[tauri::command]
pub fn get_channel_options(state: State<'_, AppState>) -> CmdResult<Vec<ChannelOption>> {
    state.with_ws(|ws| Ok(tables::channel_options(ws)))
}

#[tauri::command]
pub fn add_zone_member(state: State<'_, AppState>, zone: usize, kind: ChannelKind, channel: usize) -> CmdResult<Overview> {
    state.with_ws(|ws| {
        ws.add_zone_member(zone, kind, channel)?;
        Ok(overview(ws))
    })
}

#[tauri::command]
pub fn remove_zone_member(state: State<'_, AppState>, zone: usize, member: usize) -> CmdResult<Overview> {
    state.with_ws(|ws| {
        ws.remove_zone_member(zone, member)?;
        Ok(overview(ws))
    })
}

#[tauri::command]
pub fn move_zone_member(state: State<'_, AppState>, zone: usize, member: usize, up: bool) -> CmdResult<Overview> {
    state.with_ws(|ws| {
        ws.move_zone_member(zone, member, up)?;
        Ok(overview(ws))
    })
}

#[tauri::command]
pub fn get_changes(state: State<'_, AppState>) -> CmdResult<Vec<ChangeView>> {
    state.with_ws(|ws| Ok(tables::change_list(ws)))
}

#[tauri::command]
pub fn save_as(state: State<'_, AppState>, path: String) -> CmdResult<Overview> {
    state.with_ws(|ws| {
        ws.save_as(Path::new(&path))?;
        Ok(overview(ws))
    })
}

/// A new folder name for `kind` (`read`, `backup` or `edited`) next to the open folder, or in
/// `Documents/hyt` when nothing is open.
#[tauri::command]
pub fn suggest_folder(app: AppHandle, state: State<'_, AppState>, kind: String) -> CmdResult<String> {
    let open = state.ws.lock().ok().and_then(|g| g.as_ref().and_then(|ws| ws.folder.parent().map(Path::to_path_buf)));
    let base = match open {
        Some(dir) => dir,
        None => app.path().document_dir().map_err(|e| e.to_string())?.join("hyt"),
    };
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H%M%S");
    let name = match kind.as_str() {
        "read" => format!("read_{stamp}"),
        "backup" => format!("backup-before-write_{stamp}"),
        "edited" => format!("edited_{stamp}"),
        other => return Err(format!("unknown folder kind {other:?}")),
    };
    Ok(base.join(name).display().to_string())
}

#[derive(Clone, Serialize)]
struct Progress {
    op: &'static str,
    stage: String,
    done: usize,
    total: usize,
}

fn emit(app: &AppHandle, op: &'static str, stage: impl Into<String>, done: usize, total: usize) {
    let _ = app.emit("progress", Progress { op, stage: stage.into(), done, total });
}

/// Where to read from or write to: a serial port, (dev mode only) a recorded CPS session, or the
/// simulated radio (when `HYT_GUI_SIM` is set).
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    Port(String),
    Replay(String),
    Sim,
}

fn open_transport(state: &AppState, source: &ReadSource) -> anyhow::Result<(Box<dyn Transport>, String)> {
    Ok(match source {
        ReadSource::Port(port) => (Box::new(hyt_core::open_port(port)?), port.clone()),
        ReadSource::Sim => match &state.sim {
            Some((sim, folder)) => (Box::new(sim.clone()), format!("simulated radio ({folder})")),
            None => anyhow::bail!("no simulated radio (set HYT_GUI_SIM to a saved folder)"),
        },
        ReadSource::Replay(path) if state.dev => (
            Box::new(ReplayTransport::from_jsonl_file(Path::new(path)).with_context(|| format!("loading {path}"))?),
            format!("replay:{path}"),
        ),
        ReadSource::Replay(_) => anyhow::bail!("replaying recordings needs HYT_GUI_DEV=1"),
    })
}

fn ensure_new_folder(path: &Path) -> CmdResult<()> {
    let non_empty = path.read_dir().is_ok_and(|mut d| d.next().is_some());
    if path.is_file() || non_empty {
        return Err(format!("{} already exists and is not empty; choose a new folder", path.display()));
    }
    Ok(())
}

/// Read every file from the radio into the new folder `out` and open it.
#[tauri::command]
pub async fn read_radio(app: AppHandle, source: ReadSource, out: String) -> CmdResult<Overview> {
    let out = PathBuf::from(out);
    ensure_new_folder(&out)?;
    let busy = Busy::acquire(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = busy;
        let state = app.state::<AppState>();
        let run = || -> anyhow::Result<Overview> {
            emit(&app, "read", "Connecting", 0, 0);
            let (transport, label) = open_transport(&state, &source)?;
            let mut session = Session::new(transport);
            let result = read_all(&mut session, &state.rcdb(), |entry, _, done, total| {
                emit(&app, "read", format!("Reading {}", entry.name), done, total)
            })?;
            save_read(&result, &out, &label)?;
            let ws = state.open(&out)?;
            let o = overview(&ws);
            *state.ws.lock().map_err(|_| anyhow::anyhow!("internal error: state lock poisoned"))? = Some(ws);
            Ok(o)
        };
        run().map_err(text)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct WriteOutcome {
    written: usize,
    differing: usize,
    backup: String,
    folder: String,
}

/// Write the open configuration to the radio on `port` (the same sequence as `hyt write`). If
/// there are unsaved changes they are first saved into the new folder `save_to`; a fresh backup
/// of the radio goes into the new folder `backup`. Changes the radio: the page asks the user to
/// confirm first.
#[tauri::command]
pub async fn write_radio(
    app: AppHandle,
    port: ReadSource,
    backup: String,
    save_to: Option<String>,
) -> CmdResult<WriteOutcome> {
    let backup = PathBuf::from(backup);
    if backup.exists() {
        return Err(format!("{} already exists; choose a new backup folder", backup.display()));
    }
    let busy = Busy::acquire(&app)?;
    let folder = {
        let state = app.state::<AppState>();
        state.with_ws(|ws| {
            if ws.has_changes() {
                let to = save_to.as_deref().context("the changes must be saved before writing; give a folder")?;
                ws.save_as(Path::new(to))?;
            }
            Ok(ws.folder.clone())
        })?
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = busy;
        let state = app.state::<AppState>();
        let run = || -> anyhow::Result<WriteOutcome> {
            let saved = load_read(&folder)?;
            emit(&app, "write", "Connecting", 0, 0);
            let (transport, label) = open_transport(&state, &port)?;
            let mut session = Session::new(transport);
            let source = folder.display().to_string();
            let job = WriteJob { saved: &saved, source: &source, backup: &backup, port: &label };
            let settings = WriteSettings {
                programming_date: Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
                ..WriteSettings::default()
            };
            let report = write_config(&mut session, &state.rcdb(), &job, &settings, |event| match event {
                WriteEvent::BackupFile { done, total, entry } => {
                    emit(&app, "write", format!("Backup: reading {}", entry.name), done, total)
                }
                WriteEvent::BackupSaved { .. } => emit(&app, "write", "Backup saved; checking it is the same radio", 0, 0),
                WriteEvent::Writing { files, differing } => emit(
                    &app,
                    "write",
                    format!("Writing {files} files ({differing} differ from the radio)"),
                    0,
                    files,
                ),
                WriteEvent::Wrote { done, total, entry } => emit(&app, "write", format!("Wrote {}", entry.name), done, total),
                WriteEvent::WaitingForRestart => emit(&app, "write", "Waiting for the radio to restart", 0, 0),
                WriteEvent::NotReadyYet { attempt } => {
                    emit(&app, "write", format!("Radio not ready yet (attempt {attempt})"), 0, 0)
                }
                WriteEvent::ReadBackFile { done, total, entry } => {
                    emit(&app, "write", format!("Verifying: reading {}", entry.name), done, total)
                }
                WriteEvent::Verified { files } => emit(&app, "write", format!("Verified all {files} files"), files, files),
            })?;
            Ok(WriteOutcome {
                written: report.written,
                differing: report.differing,
                backup: report.backup.display().to_string(),
                folder: source,
            })
        };
        run().map_err(text)
    })
    .await
    .map_err(|e| e.to_string())?
}
