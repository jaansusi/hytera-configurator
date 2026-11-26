//! Programming several radios from one configuration: a *batch*.
//!
//! A batch is a folder holding a frozen copy of the configuration, the numbering rules and a log
//! of every radio connected:
//!
//! ```text
//! <batch>/batch.json        rules + log (the batch's state)
//! <batch>/log.csv           the same log for a spreadsheet (rewritten after every radio)
//! <batch>/configuration/    the configuration copied to every radio (frozen when the batch starts)
//! <batch>/radios/001/backup   radio 1 as read before anything was written
//! <batch>/radios/001/written  the configuration made for radio 1 (what was written)
//! ```
//!
//! Each radio goes through [`crate::copy::copy_choosing`]: it is read once (the backup), then its
//! Radio ID and name are chosen for its serial number, then it is written and verified. Choosing:
//! 1. a row of the plan for its serial number (Radio ID and/or name);
//! 2. a radio programmed before in this batch (written or partly written) keeps what it got then;
//! 3. otherwise the rules: the next free Radio ID from the first one (skipping IDs this batch has
//!    given or planned, and the configuration's own) or the radio's own; the name from a pattern
//!    (`{n}` the radio's number in the batch, `{id}` its Radio ID, `{serial}`) or the radio's own.
//!
//! A radio already programmed and verified in this batch is skipped unless asked again; a radio
//! whose write failed part way is written again with the same Radio ID and name.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use hytera_codeplug::rcdb::RcdbRoot;
use hytera_proto::Session;
use hytera_proto::transport::Transport;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::copy::{Choice, CopyEvent, CopyJob, CopyOptions, CopyPlan, Copied, Identity, copy_choosing, identity_of};
use crate::workspace::{Workspace, single_field, value_at};
use crate::write_flow::{WriteEvent, WriteSettings};
use crate::{SavedRead, load_config, model_number, save_config};

pub const BATCH_FILE: &str = "batch.json";
pub const LOG_FILE: &str = "log.csv";
pub const CONFIG_DIR: &str = "configuration";
pub const RADIOS_DIR: &str = "radios";

/// How radios get their Radio ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum IdRule {
    /// Each radio keeps its own.
    Keep,
    /// The lowest free ID from `first` on.
    Sequence { first: u64 },
}

/// A radio's Radio ID and/or name given in advance, by serial number.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Planned {
    pub serial: String,
    #[serde(default)]
    pub radio_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// The batch's numbering rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    pub radio_id: IdRule,
    /// Name pattern (`{n}`, `{id}`, `{serial}`); `None` keeps each radio's own name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub plan: Vec<Planned>,
    /// Allow radios to get the configuration's own Radio ID or one already given in this batch.
    #[serde(default)]
    pub allow_duplicate_id: bool,
}

impl Rules {
    /// The rules in words, one line each.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = vec![match self.radio_id {
            IdRule::Keep => "Radio ID: each radio keeps its own".to_string(),
            IdRule::Sequence { first } => format!(
                "Radio ID: the lowest free one from {first} up (skipping IDs given or planned in this batch and the \
                 configuration's own)"
            ),
        }];
        lines.push(match &self.name {
            None => "Name: each radio keeps its own".into(),
            Some(p) => format!("Name: \"{p}\" ({{n}} = the radio's number in this batch, {{id}} = Radio ID, {{serial}})"),
        });
        if !self.plan.is_empty() {
            let n = self.plan.len();
            lines.push(format!("Plan: {n} radio{} get{} a planned Radio ID and/or name by serial number", if n == 1 { "" } else { "s" }, if n == 1 { "s" } else { "" }));
        }
        if self.allow_duplicate_id {
            lines.push("Duplicate Radio IDs: allowed".into());
        }
        lines
    }
}

/// Attempts by outcome, and radios (serial numbers) programmed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub verified: usize,
    pub skipped: usize,
    pub not_written: usize,
    pub failed: usize,
    /// Different radios whose last write in this batch was verified.
    pub radios_done: usize,
    /// Different radios whose last write in this batch failed part way.
    pub radios_failed: usize,
}

/// How programming one radio ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Written and read back as written.
    Verified,
    /// Already programmed in this batch; read only.
    Skipped,
    /// Stopped before writing (no radio, another model, refused Radio ID, …): the radio is unchanged.
    NotWritten,
    /// Writing started but did not finish or verify: the radio may hold a mix of old and new settings.
    Failed,
}

impl Outcome {
    pub fn text(self) -> &'static str {
        match self {
            Outcome::Verified => "verified",
            Outcome::Skipped => "skipped",
            Outcome::NotWritten => "not written",
            Outcome::Failed => "FAILED",
        }
    }
}

/// One radio connected to the batch (one attempt).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Attempt number; its folder is `radios/<number, 3 digits>`.
    pub number: usize,
    pub started_unix: u64,
    pub finished_unix: u64,
    pub port: String,
    pub serial: Option<String>,
    /// The radio's number in the batch (`{n}`), once one was chosen.
    pub n: Option<usize>,
    /// Before and after the copy, once known.
    pub before: Option<Identity>,
    pub after: Option<Identity>,
    pub radio_ip: Option<String>,
    pub outcome: Outcome,
    /// What happened, in words (the error for a radio not written or failed).
    pub message: String,
    /// The attempt's folder, relative to the batch folder (`/`-separated).
    pub folder: String,
}

impl Entry {
    /// Whether this attempt gave the radio its Radio ID and name (written, at least in part).
    fn assigned(&self) -> bool {
        matches!(self.outcome, Outcome::Verified | Outcome::Failed) && self.after.is_some()
    }
}

/// `batch.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchFile {
    pub tool: String,
    pub created_unix: u64,
    /// Where the configuration was copied from when the batch started.
    pub configuration_source: String,
    /// The radio the configuration came from.
    pub configuration: Identity,
    pub model: Option<String>,
    pub model_number: Option<String>,
    pub rcdb_version: String,
    pub rules: Rules,
    #[serde(default)]
    pub radios: Vec<Entry>,
}

/// What the next radio gets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Assignment {
    /// Its number in the batch (`{n}`).
    pub n: usize,
    /// New Radio ID; `None` keeps its own.
    pub radio_id: Option<String>,
    /// New name; `None` keeps its own.
    pub name: Option<String>,
    /// Why, in words.
    pub why: String,
}

/// An open batch folder.
#[derive(Debug, Clone)]
pub struct Batch {
    pub folder: PathBuf,
    pub file: BatchFile,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Local date and time of a Unix time, for people.
pub fn local_time(unix: u64) -> String {
    chrono::DateTime::from_timestamp(i64::try_from(unix).unwrap_or(0), 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

fn same_serial(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn id_number(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

/// Check that `pattern` only uses `{n}`, `{id}` and `{serial}`.
fn check_pattern(pattern: &str) -> Result<()> {
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}').with_context(|| format!("name pattern {pattern:?}: '{{' without '}}'"))?;
        let name = &rest[open + 1..open + close];
        if !matches!(name, "n" | "id" | "serial") {
            bail!("name pattern {pattern:?}: unknown {{{name}}}; use {{n}} (number in the batch), {{id}} (Radio ID) or {{serial}}");
        }
        rest = &rest[open + close + 1..];
    }
    Ok(())
}

/// The name for radio `n` from `pattern`.
pub fn render_name(pattern: &str, n: usize, radio_id: Option<&str>, serial: Option<&str>) -> Result<String> {
    if pattern.contains("{id}") && radio_id.is_none() {
        bail!("the name pattern uses {{id}}, but this radio keeps its own Radio ID");
    }
    Ok(pattern
        .replace("{n}", &n.to_string())
        .replace("{id}", radio_id.unwrap_or_default())
        .replace("{serial}", serial.unwrap_or("?")))
}

/// Read a plan from CSV text: a header row with `serial` (or `serial number`) and any of
/// `radio id` (`radio_id`, `id`) and `name` (`alias`, `radio alias`, `radio name`); one radio per row.
pub fn parse_plan(text: &str) -> Result<Vec<Planned>> {
    let mut r = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).from_reader(text.trim_start_matches('\u{FEFF}').as_bytes());
    let headers: Vec<String> = r.headers()?.iter().map(|h| h.to_ascii_lowercase().replace(['_', '-'], " ")).collect();
    let col = |names: &[&str]| headers.iter().position(|h| names.contains(&h.as_str()));
    let serial = col(&["serial", "serial number", "sn"]).context("the plan needs a \"serial\" column")?;
    let id = col(&["radio id", "id"]);
    let name = col(&["name", "alias", "radio alias", "radio name"]);
    if id.is_none() && name.is_none() {
        bail!("the plan needs a \"radio id\" or a \"name\" column besides \"serial\"");
    }
    let mut plan = Vec::new();
    for (i, row) in r.records().enumerate() {
        let row = row.with_context(|| format!("plan row {}", i + 2))?;
        let get = |c: Option<usize>| c.and_then(|c| row.get(c)).map(str::to_string).filter(|t| !t.is_empty());
        let Some(serial) = get(Some(serial)) else {
            if row.iter().all(str::is_empty) {
                continue;
            }
            bail!("plan row {}: no serial number", i + 2);
        };
        plan.push(Planned { serial, radio_id: get(id), name: get(name) });
    }
    Ok(plan)
}

fn check_plan(rules: &Rules, configuration_id: Option<&str>) -> Result<()> {
    for (i, p) in rules.plan.iter().enumerate() {
        if rules.plan[..i].iter().any(|q| same_serial(&q.serial, &p.serial)) {
            bail!("the plan names serial {} twice", p.serial);
        }
        if let Some(id) = &p.radio_id {
            let n = id_number(id).with_context(|| format!("plan for serial {}: Radio ID {id:?} is not a number", p.serial))?;
            if rules.allow_duplicate_id {
                continue;
            }
            if let Some(q) = rules.plan[..i].iter().find(|q| q.radio_id.as_deref().and_then(id_number) == Some(n)) {
                bail!("the plan gives Radio ID {id} to both {} and {}", q.serial, p.serial);
            }
            if configuration_id.and_then(id_number) == Some(n) {
                bail!("the plan gives serial {} Radio ID {id}, the configuration's own radio's ID", p.serial);
            }
        }
    }
    Ok(())
}

impl Batch {
    /// Start a batch in the new (or empty) folder `folder` for the configuration in `config`
    /// (read or edited folder, copied into the batch). The rules are checked against the
    /// configuration: pattern, plan, and that the first Radio ID and the names are accepted.
    pub fn create(folder: &Path, config: &Path, rules: Rules, rcdb: &RcdbRoot, options_path: Option<&Path>) -> Result<Batch> {
        if folder.exists() && folder.read_dir().with_context(|| format!("reading {}", folder.display()))?.next().is_some() {
            bail!("{} already exists and is not empty; choose a new folder for the batch", folder.display());
        }
        let saved = load_config(config, rcdb)?;
        let mut ws = Workspace::open(config, rcdb, options_path)?;
        let configuration = identity_of(&ws, ws.view(), &saved.manifest);
        if let Some(p) = &rules.name {
            check_pattern(p)?;
        }
        check_plan(&rules, configuration.radio_id.as_deref())?;
        // Try the first ID, the planned values and a name on the configuration itself, so mistakes
        // (out of range, too long) show now rather than at the first radio.
        let id_at = single_field(ws.view(), "QEU_C_Config", "Radio ID").context("this configuration has no Radio ID")?;
        let name_at = single_field(ws.view(), "Comm_Basic", "Radio Alias").context("this configuration has no Radio Alias")?;
        let mut ids: Vec<(String, String)> = rules.plan.iter().filter_map(|p| p.radio_id.clone().map(|id| (id, format!("plan for serial {}", p.serial)))).collect();
        if let IdRule::Sequence { first } = rules.radio_id {
            ids.push((first.to_string(), "first Radio ID".into()));
        }
        for (id, what) in &ids {
            ws.set_value(&id_at, id).with_context(|| format!("{what}: Radio ID {id}"))?;
        }
        let mut names: Vec<(String, String)> = rules.plan.iter().filter_map(|p| p.name.clone().map(|n| (n, format!("plan for serial {}", p.serial)))).collect();
        if let Some(p) = &rules.name {
            let id = match rules.radio_id {
                IdRule::Sequence { first } => Some(first.to_string()),
                IdRule::Keep if p.contains("{id}") => bail!("the name pattern uses {{id}}, but radios keep their own Radio ID"),
                IdRule::Keep => None,
            };
            // A long number and serial, to catch names that grow too long later in the batch.
            let name = render_name(p, 999, id.as_deref().map(|_| "16776415"), Some("R00N000000"))?;
            names.push((name, format!("name pattern {p:?}")));
        }
        for (name, what) in &names {
            ws.set_value(&name_at, name).with_context(|| format!("{what}: name {name:?}"))?;
        }
        let layout = crate::load_layout(&saved, rcdb)?;
        drop(ws);

        std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
        let source = format!("configuration of the batch, copied from {}", config.display());
        save_config(&saved, &saved.files, &folder.join(CONFIG_DIR), &source, &layout)?;
        let m = &saved.manifest;
        let batch = Batch {
            folder: folder.to_path_buf(),
            file: BatchFile {
                tool: format!("hyt {}", env!("CARGO_PKG_VERSION")),
                created_unix: now_unix(),
                configuration_source: config.display().to_string(),
                configuration,
                model: m.model_name.clone(),
                model_number: model_number(m).map(str::to_string),
                rcdb_version: m.rcdb_version.clone(),
                rules,
                radios: Vec::new(),
            },
        };
        batch.save()?;
        Ok(batch)
    }

    /// Open the batch in `folder`.
    pub fn open(folder: &Path) -> Result<Batch> {
        let path = folder.join(BATCH_FILE);
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {} (is this a batch folder?)", path.display()))?;
        let file = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Batch { folder: folder.to_path_buf(), file })
    }

    /// The batch's configuration (frozen copy).
    pub fn config_folder(&self) -> PathBuf {
        self.folder.join(CONFIG_DIR)
    }

    /// Save `batch.json` and `log.csv` (each written to a temporary file, then renamed).
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.file)?;
        replace_file(&self.folder.join(BATCH_FILE), json.as_bytes())?;
        replace_file(&self.folder.join(LOG_FILE), self.log_csv()?.as_bytes())
    }

    /// The log as CSV text.
    pub fn log_csv(&self) -> Result<String> {
        let mut w = csv::Writer::from_writer(Vec::new());
        w.write_record([
            "No.", "Time", "Port", "Serial", "Result", "Radio ID before", "Name before", "Radio ID after", "Name after",
            "Radio IP", "Message", "Folder",
        ])?;
        for e in &self.file.radios {
            let id = |i: &Option<Identity>, f: fn(&Identity) -> &Option<String>| i.as_ref().and_then(|i| f(i).clone()).unwrap_or_default();
            w.write_record([
                e.number.to_string(),
                local_time(e.started_unix),
                e.port.clone(),
                e.serial.clone().unwrap_or_default(),
                e.outcome.text().to_string(),
                id(&e.before, |i| &i.radio_id),
                id(&e.before, |i| &i.alias),
                id(&e.after, |i| &i.radio_id),
                id(&e.after, |i| &i.alias),
                e.radio_ip.clone().unwrap_or_default(),
                e.message.clone(),
                e.folder.clone(),
            ])?;
        }
        Ok(String::from_utf8(w.into_inner()?)?)
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        let mut serials: Vec<&str> = Vec::new();
        for e in &self.file.radios {
            match e.outcome {
                Outcome::Verified => c.verified += 1,
                Outcome::Skipped => c.skipped += 1,
                Outcome::NotWritten => c.not_written += 1,
                Outcome::Failed => c.failed += 1,
            }
            if let Some(s) = e.serial.as_deref()
                && e.assigned()
                && !serials.iter().any(|t| same_serial(t, s))
            {
                serials.push(s);
            }
        }
        for s in serials {
            match self.previous(s).map(|e| e.outcome) {
                Some(Outcome::Verified) => c.radios_done += 1,
                Some(Outcome::Failed) => c.radios_failed += 1,
                _ => {}
            }
        }
        c
    }

    /// The last attempt that gave the radio `serial` its Radio ID and name.
    fn previous(&self, serial: &str) -> Option<&Entry> {
        self.file.radios.iter().rev().find(|e| e.assigned() && e.serial.as_deref().is_some_and(|s| same_serial(s, serial)))
    }

    /// Radio IDs given (or planned) to radios other than `serial`, and the configuration's own.
    fn taken_ids(&self, serial: Option<&str>) -> BTreeSet<u64> {
        let other = |s: Option<&str>| match (s, serial) {
            (Some(a), Some(b)) => !same_serial(a, b),
            _ => true,
        };
        let mut ids: BTreeSet<u64> = self
            .file
            .radios
            .iter()
            .filter(|e| e.assigned() && other(e.serial.as_deref()))
            .filter_map(|e| e.after.as_ref()?.radio_id.as_deref().and_then(id_number))
            .collect();
        let rules = &self.file.rules;
        ids.extend(rules.plan.iter().filter(|p| other(Some(&p.serial))).filter_map(|p| p.radio_id.as_deref().and_then(id_number)));
        ids.extend(self.file.configuration.radio_id.as_deref().and_then(id_number));
        ids
    }

    /// What the radio with serial number `serial` gets (`None`: the next unknown radio, for
    /// showing what comes next). `own_id` is the radio's own Radio ID (checked against the batch
    /// when it is kept).
    pub fn assign(&self, serial: Option<&str>, own_id: Option<&str>) -> Result<Assignment> {
        let rules = &self.file.rules;
        let prev = serial.and_then(|s| self.previous(s));
        let planned = serial.and_then(|s| rules.plan.iter().find(|p| same_serial(&p.serial, s)));
        let n = match prev.and_then(|e| e.n) {
            Some(n) => n,
            None => self.file.radios.iter().filter(|e| e.assigned()).filter_map(|e| e.n).max().unwrap_or(0) + 1,
        };
        let mut why = Vec::new();
        let radio_id = if let Some(id) = planned.and_then(|p| p.radio_id.clone()) {
            why.push(format!("Radio ID from the plan for {}", planned.map_or("", |p| p.serial.as_str())));
            Some(id)
        } else if let Some(prev) = prev {
            why.push(format!("as when this radio was programmed in attempt {}", prev.number));
            match rules.radio_id {
                IdRule::Keep => None,
                IdRule::Sequence { .. } => prev.after.as_ref().and_then(|a| a.radio_id.clone()),
            }
        } else {
            match rules.radio_id {
                IdRule::Keep => None,
                IdRule::Sequence { first } => {
                    let taken = self.taken_ids(serial);
                    let id = (first.max(1)..).find(|i| !taken.contains(i)).unwrap_or(first);
                    why.push("next free Radio ID".into());
                    Some(id.to_string())
                }
            }
        };
        let name = if let Some(name) = planned.and_then(|p| p.name.clone()) {
            why.push("name from the plan".into());
            Some(name)
        } else {
            match &rules.name {
                Some(p) => Some(render_name(p, n, radio_id.as_deref(), serial)?),
                None => None,
            }
        };
        if !rules.allow_duplicate_id {
            let check = radio_id.as_deref().or(own_id);
            if let Some(id) = check.and_then(id_number)
                && serial.is_some()
                && let Some(e) = self.file.radios.iter().rev().find(|e| {
                    e.assigned()
                        && e.serial.as_deref().zip(serial).is_none_or(|(a, b)| !same_serial(a, b))
                        && e.after.as_ref().and_then(|a| a.radio_id.as_deref()).and_then(id_number) == Some(id)
                })
            {
                bail!(
                    "Radio ID {id} was already given to radio {} in attempt {} of this batch; two radios with one ID \
                     break private calls and caller display",
                    e.serial.as_deref().unwrap_or("?"),
                    e.number
                );
            }
        }
        if why.is_empty() {
            why.push("keeps its own Radio ID and name".into());
        }
        Ok(Assignment { n, radio_id, name, why: why.join("; ") })
    }

    /// Program the radio on `session`: read it into a new attempt folder, choose its Radio ID and
    /// name, write and verify (see the module docs). Every attempt is logged, also when nothing
    /// could be written; the error of a failed attempt is in the returned entry's message. Errors
    /// returned are only those of the batch itself (saving its log). Changes the radio: callers
    /// must have the owner's OK. `again`: also program a radio already verified in this batch.
    #[allow(clippy::too_many_arguments)]
    pub fn program<T: Transport>(
        &mut self,
        session: &mut Session<T>,
        rcdb: &RcdbRoot,
        port: &str,
        options_path: Option<&Path>,
        settings: &WriteSettings,
        again: bool,
        mut on_event: impl FnMut(CopyEvent<'_>),
    ) -> Result<Entry> {
        let config_folder = self.config_folder();
        let config = load_config(&config_folder, rcdb)?;
        let mut number = self.file.radios.iter().map(|e| e.number).max().unwrap_or(0) + 1;
        while self.folder.join(RADIOS_DIR).join(format!("{number:03}")).exists() {
            number += 1;
        }
        let rel = format!("{RADIOS_DIR}/{number:03}");
        let dir = self.folder.join(RADIOS_DIR).join(format!("{number:03}"));
        let (backup, out) = (dir.join("backup"), dir.join("written"));
        let started = now_unix();
        let config_source = config_folder.display().to_string();
        let job = CopyJob { config: &config, config_source: &config_source, backup: &backup, out: &out, port, options_path };

        let mut serial = None;
        let mut chosen: Option<Assignment> = None;
        let mut before = None;
        let mut plan: Option<CopyPlan> = None;
        let mut writing = false;
        let this = &*self;
        let choose = |target: &SavedRead| -> Result<Choice> {
            serial = target.manifest.serial_number.clone();
            let s = serial.as_deref();
            if !again
                && let Some(prev) = s.and_then(|s| this.previous(s)).filter(|e| e.outcome == Outcome::Verified)
            {
                before = own_identity(target, rcdb);
                return Ok(Choice::Skip(format!(
                    "already programmed in this batch (attempt {}, {}); nothing was written",
                    prev.number,
                    local_time(prev.finished_unix)
                )));
            }
            let own = own_radio_id(target, rcdb);
            let a = this.assign(s, own.as_deref())?;
            let opts = CopyOptions {
                radio_id: a.radio_id.clone(),
                alias: a.name.clone(),
                allow_duplicate_id: this.file.rules.allow_duplicate_id,
                keep_config_identity: false,
            };
            chosen = Some(a);
            Ok(Choice::Copy(opts))
        };
        let result = copy_choosing(session, rcdb, &job, settings, choose, |e| {
            match &e {
                CopyEvent::Prepared(p) => plan = Some((*p).clone()),
                CopyEvent::Step(WriteEvent::Writing { .. }) => writing = true,
                _ => {}
            }
            on_event(e);
        });

        let n = chosen.as_ref().map(|a| a.n);
        let mut entry = Entry {
            number,
            started_unix: started,
            finished_unix: now_unix(),
            port: port.to_string(),
            serial,
            n,
            before: plan.as_ref().map(|p| p.before.clone()).or(before),
            after: plan.as_ref().map(|p| p.after.clone()),
            radio_ip: plan.as_ref().and_then(|p| p.radio_ip.clone()),
            outcome: Outcome::NotWritten,
            message: String::new(),
            folder: rel,
        };
        match result {
            Ok(Copied::Done(report)) => {
                entry.outcome = Outcome::Verified;
                entry.message = format!(
                    "written and verified ({} files, {} differed); {}",
                    report.write.written,
                    report.write.differing,
                    chosen.map(|a| a.why).unwrap_or_default()
                );
            }
            Ok(Copied::Skipped { reason }) => {
                entry.outcome = Outcome::Skipped;
                entry.message = reason;
            }
            Err(e) => {
                entry.outcome = if writing { Outcome::Failed } else { Outcome::NotWritten };
                if !writing {
                    entry.after = None;
                    entry.radio_ip = None;
                    entry.n = None;
                }
                entry.message = format!("{e:#}");
            }
        }
        self.file.radios.push(entry.clone());
        self.save()?;
        Ok(entry)
    }
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

/// The raw Radio ID of a read radio, as text.
fn own_radio_id(target: &SavedRead, rcdb: &RcdbRoot) -> Option<String> {
    let layout = crate::load_layout(target, rcdb).ok()?;
    let files = crate::decode_read(target, &layout).ok()?;
    let at = single_field(&files, "QEU_C_Config", "Radio ID")?;
    value_at(&files, &at).and_then(Value::as_i64).map(|v| v.to_string())
}

/// Serial, Radio ID and (raw) name of a read radio, for the log of a skipped radio.
fn own_identity(target: &SavedRead, rcdb: &RcdbRoot) -> Option<Identity> {
    let layout = crate::load_layout(target, rcdb).ok()?;
    let files = crate::decode_read(target, &layout).ok()?;
    let at = single_field(&files, "Comm_Basic", "Radio Alias")?;
    let alias = value_at(&files, &at).and_then(Value::as_str).map(str::to_string);
    Some(Identity { serial: target.manifest.serial_number.clone(), radio_id: own_radio_id(target, rcdb), alias })
}
