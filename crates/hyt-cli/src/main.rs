use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use hyt_core::batch::{Assignment, Batch, IdRule, Outcome, Rules, local_time, parse_plan};
use hyt_core::copy::{CopyEvent, CopyJob, CopyOptions, CopyPlan, copy_to_radio, prepare_folder};
use hyt_core::edit::{ADDED, Change, REMOVED, apply_edits, changes, labelize};
use hyt_core::view::{Labeler, full_listing, summary};
use hyt_core::write_flow::{WriteEvent, WriteJob, WriteSettings, write_config};
use hyt_core::{decode_read, info_entries, load_config, load_layout, read_radio, save_config, save_read};
use hytera_codeplug::decode::DecodedFile;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::RcdbRoot;
use hytera_models::profile_for;
use hytera_proto::Session;
use hytera_proto::transport::{ReplayTransport, Transport, available_ports};

/// Read Hytera G2 radio settings without Hytera CPS.
#[derive(Parser)]
#[command(name = "hyt", version)]
struct Cli {
    /// More log output (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List serial ports.
    Ports,
    /// Identify the connected radio.
    Info {
        #[command(flatten)]
        source: Source,
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Read all settings files from the radio into a folder.
    Read {
        #[command(flatten)]
        source: Source,
        /// Output folder (must be new or empty).
        #[arg(short, long)]
        out: PathBuf,
        /// Folder with Hytera's RCDB data (a CPS install or a copy of its RCDB folders).
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
    },
    /// Decode a folder saved by `hyt read` into named settings (JSON).
    Decode {
        /// Folder saved by `hyt read`.
        folder: PathBuf,
        /// Write the JSON to this file instead of standard output.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Use CPS's display text for values ("High", "444.725 MHz") where it converts back exactly.
        #[arg(long)]
        labels: bool,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
        /// Option catalogue (default: the per-user data folder).
        #[arg(long, env = "HYT_OPTIONS")]
        options: Option<PathBuf>,
    },
    /// Apply an edited `hyt decode` JSON (all files or only some) to a saved read, saving the
    /// result as a new folder for `hyt write`. Values only; records cannot be added or removed yet.
    Apply {
        /// Folder saved by `hyt read` (the starting point).
        folder: PathBuf,
        /// Edited JSON from `hyt decode` (raw numbers or display text both work).
        edits: PathBuf,
        /// New folder for the result.
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
        #[arg(long, env = "HYT_OPTIONS")]
        options: Option<PathBuf>,
    },
    /// Show which settings differ between two saved folders.
    Diff {
        /// The "before" folder.
        before: PathBuf,
        /// The "after" folder.
        after: PathBuf,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
        #[arg(long, env = "HYT_OPTIONS")]
        options: Option<PathBuf>,
    },
    /// Show the main settings of a folder saved by `hyt read` (channels, zones, contacts).
    Show {
        /// Folder saved by `hyt read`.
        folder: PathBuf,
        /// List every field of every file instead of the overview.
        #[arg(long)]
        all: bool,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
        /// Option catalogue from tools/export-rcdb-options.ps1 (default: the per-user data folder).
        #[arg(long, env = "HYT_OPTIONS")]
        options: Option<PathBuf>,
    },
    /// Hytera licences: check a licence package (.zip) offline like CPS's Authorization does, or
    /// show which licences a radio has installed. Never sends a licence to the radio.
    Licence {
        #[command(subcommand)]
        action: LicenceAction,
    },
    /// Tables as spreadsheets (CSV): every field of every record with CPS's labels and units.
    Csv {
        #[command(subcommand)]
        action: CsvAction,
    },
    /// Hytera's layout data (RCDB): where it is loaded from, and copying it from a CPS install so
    /// hyt works without CPS (e.g. on macOS or Linux, from a copy of a Windows CPS install).
    Rcdb {
        #[command(subcommand)]
        action: RcdbAction,
    },
    /// CPS template files (.tpl): show one, import one for a radio, export a folder for CPS.
    Tpl {
        #[command(subcommand)]
        action: TplAction,
    },
    /// Program several radios with one configuration, one after another: a batch folder holds a
    /// frozen copy of the configuration, how radios get their Radio ID and name, and a log of
    /// every radio. Start with `hyt batch new`, then `hyt batch program`.
    Batch {
        #[command(subcommand)]
        action: BatchAction,
    },
    /// Put a configuration on another radio of the same model (programming several radios from
    /// one configuration). Reads the radio once as its backup, keeps its identity, its
    /// never-written files and radio data, and its own Radio ID and name unless --radio-id /
    /// --alias are given (Radio IP follows Radio ID like in CPS), then writes and verifies. With
    /// --like instead of --port only the folder for `hyt write` is made; nothing is sent.
    Copy {
        /// Configuration to copy (a folder saved by `hyt read`, `hyt apply` or the GUI).
        #[arg(long)]
        from: PathBuf,
        /// Serial port of the radio to program (changes that radio).
        #[arg(short, long, required_unless_present = "like", conflicts_with = "like")]
        port: Option<String>,
        /// Instead of a radio: a folder read from the target radio (`hyt read`); only --out is made.
        #[arg(long)]
        like: Option<PathBuf>,
        /// New folder for the radio's backup, read before anything is written (with --port).
        #[arg(long, required_unless_present = "like", conflicts_with = "like")]
        backup: Option<PathBuf>,
        /// New folder for the configuration made for this radio (what is written).
        #[arg(short, long)]
        out: PathBuf,
        /// Give the radio this Radio ID (default: keep its own).
        #[arg(long)]
        radio_id: Option<String>,
        /// Give the radio this name, Radio Alias (default: keep its own).
        #[arg(long)]
        alias: Option<String>,
        /// Allow the radio to end up with the same Radio ID as the radio the configuration came from.
        #[arg(long)]
        allow_duplicate_id: bool,
        /// Write even though writing has not been verified for this model yet.
        #[arg(long)]
        allow_unverified: bool,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
        #[arg(long, env = "HYT_OPTIONS")]
        options: Option<PathBuf>,
    },
    /// Write a folder saved by `hyt read` back to the radio. Takes a fresh backup first, refuses a
    /// different radio, and reads back afterwards to verify.
    Write {
        /// Serial port of the programming cable.
        #[arg(short, long)]
        port: String,
        /// Folder saved by `hyt read` whose files are written.
        #[arg(long)]
        from: PathBuf,
        /// New folder for the fresh backup taken right before writing.
        #[arg(long)]
        backup: PathBuf,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
        /// Write even though writing has not been verified for this model yet.
        #[arg(long)]
        allow_unverified: bool,
    },
}

#[derive(Subcommand)]
enum RcdbAction {
    /// Show which RCDB data and option catalogues hyt uses.
    Status,
    /// Copy the RCDB folders (and exported options.json / ui.json / models.json, if the source has them) into the
    /// per-user data folder. FROM is a CPS install folder or a copy of one, or a copied `hyt/rcdb`
    /// data folder.
    Import { from: PathBuf },
}

#[derive(Subcommand)]
enum CsvAction {
    /// List the tables of a folder (ids for export/import).
    Tables {
        folder: PathBuf,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
    },
    /// Write table TABLE (e.g. digital, analog, zones, contacts, list:QEU_C_ScanConfig) as CSV.
    Export {
        folder: PathBuf,
        table: String,
        /// CSV file to write (default: standard output).
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
    },
    /// Apply an edited CSV to table TABLE of FOLDER and save the result as a new folder. Rows are
    /// matched by "No."; extra rows add channels (channel tables only); nothing is removed.
    Import {
        folder: PathBuf,
        table: String,
        csv: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
    },
}

fn csv_command(action: CsvAction) -> Result<()> {
    use hyt_core::workspace::Workspace;
    match action {
        CsvAction::Tables { folder, rcdb } => {
            let ws = Workspace::open(&folder, &rcdb_root(rcdb), None)?;
            for id in hyt_core::tables::TABLE_IDS {
                if let Some(t) = hyt_core::tables::table(&ws, id) {
                    println!("{:<28} {} ({} of {})", t.id, t.title, t.rows.len(), t.capacity);
                }
            }
            for l in hyt_core::lists::index(&ws) {
                println!("{:<28} {} ({} of {})", l.id, l.title, l.rows, l.capacity);
            }
        }
        CsvAction::Export { folder, table, out, rcdb } => {
            let ws = Workspace::open(&folder, &rcdb_root(rcdb), None)?;
            let text = hyt_core::sheet::export(&ws, &table)?;
            match out {
                Some(path) => {
                    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
                    println!("Wrote {} as {}", table, path.display());
                }
                None => print!("{text}"),
            }
        }
        CsvAction::Import { folder, table, csv, out, rcdb } => {
            let mut ws = Workspace::open(&folder, &rcdb_root(rcdb), None)?;
            let text = std::fs::read_to_string(&csv).with_context(|| format!("reading {}", csv.display()))?;
            let report = hyt_core::sheet::import(&mut ws, &table, &text)?;
            for e in &report.errors {
                eprintln!("  refused: {e}");
            }
            if !ws.has_changes() {
                println!("No changes; nothing saved.");
                return Ok(());
            }
            for c in hyt_core::tables::change_list(&ws) {
                println!("  {}: {} {} -> {}", c.place, c.setting, c.old, c.new);
            }
            ws.save_as(&out)?;
            println!(
                "Saved {} changed values and {} new records into {} ({} refused). Check with `hyt diff`, then `hyt write --from`.",
                report.changed,
                report.added,
                out.display(),
                report.errors.len()
            );
        }
    }
    Ok(())
}

#[derive(Subcommand)]
enum BatchAction {
    /// Start a batch: copy the configuration into a new batch folder and set how radios get their
    /// Radio ID and name. The rules are checked now. Nothing is sent to a radio.
    New {
        /// New folder for the batch.
        folder: PathBuf,
        /// The configuration to put on every radio (a folder saved by `hyt read`, `hyt apply` or
        /// the GUI).
        #[arg(long)]
        from: PathBuf,
        /// Give each radio the lowest free Radio ID from this one up. Default: radios keep their own.
        #[arg(long)]
        first_id: Option<u64>,
        /// Name radios by this pattern: {n} = the radio's number in the batch, {id} = its Radio ID,
        /// {serial} = its serial number (e.g. "Unit {n}"). Default: radios keep their own name.
        #[arg(long)]
        name: Option<String>,
        /// CSV of planned Radio IDs and/or names by serial number (header: serial, radio id, name).
        #[arg(long)]
        plan: Option<PathBuf>,
        /// Allow a radio to get the configuration's own Radio ID or one already given in the batch.
        #[arg(long)]
        allow_duplicate_id: bool,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
        #[arg(long, env = "HYT_OPTIONS")]
        options: Option<PathBuf>,
    },
    /// Program radios on one port, one after another: for each, wait for Enter, read it (its
    /// backup), give it its Radio ID and name, write and verify, and log it. A radio already done
    /// in this batch is skipped. Changes every radio programmed.
    Program {
        /// The batch folder (`hyt batch new`).
        folder: PathBuf,
        /// Serial port of the programming cable.
        #[arg(short, long)]
        port: String,
        /// Program the radio connected now and stop, without asking.
        #[arg(long)]
        once: bool,
        /// Also program radios already verified in this batch (they keep their Radio ID and name).
        #[arg(long)]
        again: bool,
        /// Write even though writing has not been verified for this model yet.
        #[arg(long)]
        allow_unverified: bool,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
        #[arg(long, env = "HYT_OPTIONS")]
        options: Option<PathBuf>,
    },
    /// Show a batch: configuration, rules, what the next radio gets, and the log.
    Status {
        folder: PathBuf,
        /// Print the batch file (JSON) instead.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum LicenceAction {
    /// Check a licence package from Hytera (.zip with readme .xml, .md5 and .fcr) the way CPS's
    /// Authorization button does: files, checksums, serial number and CPS V1.5.24's model rules.
    /// Offline; nothing is sent to the radio.
    Check {
        /// The licence package (.zip).
        package: PathBuf,
        /// A folder read from the radio the licence is for (`hyt read`): checks its serial
        /// number and model like CPS.
        #[arg(long)]
        radio: Option<PathBuf>,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show the licences a radio reports (identify item 0x0200) and the licence flags in its
    /// settings, from a saved read or (with --port) by identifying the radio (no programming mode).
    Status {
        /// A folder saved by `hyt read`.
        #[arg(required_unless_present_any = ["port", "replay"])]
        folder: Option<PathBuf>,
        /// Identify the radio on this serial port instead.
        #[arg(short, long, conflicts_with = "folder")]
        port: Option<String>,
        /// Replay a recorded CPS session instead.
        #[arg(long, conflicts_with_all = ["folder", "port"])]
        replay: Option<PathBuf>,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
    },
}

fn print_installed(l: &hyt_core::licence::InstalledLicences) {
    use hyt_core::licence::licence_name;
    if l.unrecognised {
        println!("Installed licences (item 0x0200): answer not understood, raw {}", l.raw_hex);
    } else if l.codes.is_empty() {
        println!("Installed licences (item 0x0200): none (all zero)");
    } else {
        println!("Installed licences (item 0x0200):");
        for c in &l.codes {
            println!("  {c} {}", licence_name(c).unwrap_or("(unknown code)"));
        }
    }
}

fn licence_command(action: LicenceAction) -> Result<()> {
    use hyt_core::licence::{Level, RadioFacts, check_package_file, installed_from_manifest, installed_licences};
    match action {
        LicenceAction::Check { package, radio, json } => {
            let facts = match &radio {
                Some(dir) => Some(RadioFacts::from_manifest(&load_config(dir, &rcdb_root(None))?.manifest)?),
                None => None,
            };
            let report = check_package_file(&package, facts.as_ref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(());
            }
            println!("Licence package {}", package.display());
            for e in &report.entries {
                println!("  {:<40} {:>8} bytes", e.name, e.size);
            }
            for p in &report.packages {
                println!("Package for serial {:?}: file {:?}", p.objdevice, p.file);
                for f in &p.features {
                    println!(
                        "  {} {:<22} value {:?} -> {}",
                        f.code,
                        f.name.unwrap_or("(unknown code)"),
                        f.value,
                        if f.licensed { "licensed" } else { "not licensed" }
                    );
                }
            }
            println!("Checks (CPS's order):");
            for c in &report.checks {
                let tag = match c.level {
                    Level::Ok => "ok     ",
                    Level::Note => "note   ",
                    Level::Refused => "REFUSED",
                };
                println!("  {tag} {}", c.text);
            }
            match &report.radio {
                None => println!("Result: give --radio <read folder> to check against the radio."),
                Some(r) if report.accepted() => println!(
                    "Result: CPS V1.5.24 would send this licence to {} {} (the radio then checks it itself).",
                    r.model_name, r.serial
                ),
                Some(r) => println!("Result: CPS V1.5.24 would refuse this package for {} {}.", r.model_name, r.serial),
            }
        }
        LicenceAction::Status { folder, port, replay, rcdb } => match folder {
            Some(dir) => {
                let saved = load_config(&dir, &rcdb_root(rcdb.clone()))?;
                match installed_from_manifest(&saved.manifest) {
                    Some(l) => print_installed(&l),
                    None => println!("Installed licences: item 0x0200 not in this read's manifest"),
                }
                let layout = load_layout(&saved, &rcdb_root(rcdb))?;
                let decoded = decode_read(&saved, &layout)?;
                for (file, block, what) in [(18u16, 902u16, "Comm_Basic 902 (used by CPS)"), (253, 920, "Comm_RadioInfo 920")] {
                    let Some(rec) = decoded
                        .iter()
                        .find(|f| f.id == file)
                        .and_then(|f| f.blocks.iter().find(|b| b.id == block))
                        .and_then(|b| b.records.first())
                        .and_then(|r| r.fields.as_ref())
                    else {
                        continue;
                    };
                    let flags: Vec<String> = hyt_core::licence::LICENCE_CODES
                        .iter()
                        .filter_map(|(_, name)| rec.0.get(*name).map(|v| format!("{name}={v}")))
                        .collect();
                    println!("Licence flags in {what}: {}", flags.join(", "));
                }
            }
            None => {
                let (transport, _) = Source { port, replay }.open()?;
                let mut session = Session::new(transport);
                let info = session.identify().context("identifying the radio")?;
                println!(
                    "{} serial {}",
                    info.model_name().unwrap_or_default(),
                    info.serial_number().unwrap_or_default()
                );
                match info.items.get(&hyt_core::licence::LICENCE_INFO_ITEM) {
                    Some(i) if i.status == 0 => print_installed(&installed_licences(&i.value)),
                    Some(i) => println!("Installed licences: item 0x0200 not supported (status {})", i.status),
                    None => println!("Installed licences: item 0x0200 not answered"),
                }
            }
        },
    }
    Ok(())
}

#[derive(Subcommand)]
enum TplAction {
    /// Show a template's header and the radio files it holds.
    Info { file: PathBuf },
    /// Save a template's radio files as a new folder for `hyt write`, taking the radio identity
    /// from a read of the target radio. Refuses a template for another model or RCDB version;
    /// files the template lacks are kept from the read.
    Import {
        file: PathBuf,
        /// A folder read from the radio the template is meant for (`hyt read`).
        #[arg(long)]
        like: PathBuf,
        /// New folder for the result.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Save a folder (read, edited or imported) as a template file that CPS can open.
    Export {
        folder: PathBuf,
        /// New template file (.tpl).
        #[arg(short, long)]
        out: PathBuf,
        /// Template name shown in CPS (default: the imported template's name, else the folder name).
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// A CPS template of the same RCDB version to take CPS's file descriptions from (default:
        /// one from the CPS install's Templates folder).
        #[arg(long)]
        seqo_from: Option<PathBuf>,
        #[arg(long, env = "HYT_RCDB")]
        rcdb: Option<PathBuf>,
    },
}

fn tpl_command(action: TplAction) -> Result<()> {
    use hyt_core::tpl;
    match action {
        TplAction::Info { file } => {
            let t = tpl::Template::read(&file)?;
            let h = &t.header;
            let show = |label: &str, v: &Option<String>| println!("{label:<14} {}", v.as_deref().unwrap_or("-"));
            show("Name", &h.name);
            show("Description", &h.description);
            show("Product", &h.product);
            show("Model number", &h.model_number);
            show("RCDB version", &h.rcdb_version);
            show("Serial number", &h.serial_number);
            show("Created", &h.created_time);
            show("Guid", &h.guid);
            let files = t.files()?;
            println!("{} radio files:", files.len());
            for (f, e) in files.iter().zip(&t.entries) {
                let extra: Vec<&str> = e.files.iter().map(|(n, _)| n.as_str()).filter(|n| *n != e.name).collect();
                let with = if extra.is_empty() { String::new() } else { format!("  (+ {})", extra.join(", ")) };
                println!("  {:>3} {:<28} {:>6} bytes{with}", f.id, f.name, f.bytes.len());
            }
            for w in t.warnings() {
                println!("warning: {w}");
            }
        }
        TplAction::Import { file, like, out } => {
            let rcdb = rcdb_root(None);
            let reference = load_config(&like, &rcdb)?;
            let layout = load_layout(&reference, &rcdb)?;
            let (t, report) = tpl::import_file(&file, &reference, &out, &layout)?;
            let name = t.header.name.clone().unwrap_or_default();
            for w in &report.warnings {
                println!("warning: {w}");
            }
            for (id, n) in &report.kept_from_reference {
                println!("not in the template, kept from {}: {id} {n}", like.display());
            }
            for (id, n) in &report.ignored {
                println!("not a file of this radio, ignored: {id} {n}");
            }
            println!(
                "Imported {} files from template \"{name}\" into {} for radio {} ({}). Check with `hyt diff {} {}`.",
                report.from_template,
                out.display(),
                reference.manifest.serial_number.as_deref().unwrap_or("?"),
                reference.manifest.model_name.as_deref().unwrap_or("?"),
                like.display(),
                out.display()
            );
        }
        TplAction::Export { folder, out, name, description, seqo_from, rcdb } => {
            let rcdb = rcdb_root(rcdb);
            let links = rcdb.table_links()?;
            let saved = load_config(&folder, &rcdb)?;
            let seqo = match seqo_from {
                Some(p) => Some(tpl::Template::read(&p)?),
                None => tpl::cps_template_for(&rcdb, &saved.manifest),
            };
            let default_name = folder.file_name().map_or_else(|| "hyt".to_string(), |n| n.to_string_lossy().into_owned());
            let opts = tpl::ExportOptions {
                name,
                default_name,
                description,
                sidecar: None,
                seqo_from: seqo.as_ref(),
                when: tpl::now(),
            };
            let (t, report) = tpl::export(&folder, &out, &rcdb, &links, opts)?;
            for w in &report.warnings {
                println!("warning: {w}");
            }
            println!(
                "Exported {} files as template \"{}\" to {}. In CPS, import it into the template library.",
                report.files,
                t.header.name.as_deref().unwrap_or(""),
                out.display()
            );
        }
    }
    Ok(())
}

/// The RCDB data folder: as given (or `HYT_RCDB`), else the CPS install, else the per-user copy.
fn rcdb_root(given: Option<PathBuf>) -> RcdbRoot {
    RcdbRoot::new(given.unwrap_or_else(hytera_codeplug::rcdb::default_root))
}

fn rcdb_command(action: RcdbAction) -> Result<()> {
    use hytera_codeplug::options::data_dir;
    use hytera_codeplug::rcdb::{default_root, import, user_copy_dir, versions};
    let data = data_dir().context("no per-user data folder (LOCALAPPDATA / XDG_DATA_HOME / HOME)")?;
    match action {
        RcdbAction::Status => {
            let root = std::env::var_os("HYT_RCDB").map_or_else(default_root, PathBuf::from);
            let found = versions(&root);
            println!("RCDB data:   {}{}", root.display(), if found.is_empty() { "  (not found)" } else { "" });
            if let (Some(first), Some(last)) = (found.first(), found.last()) {
                println!("  layouts for {} RCDB versions ({first} … {last})", found.len());
            }
            if let Some(copy) = user_copy_dir() {
                println!("User copy:   {}{}", copy.display(), if copy.join("RcdbJsons").is_dir() { "" } else { "  (none; see `hyt rcdb import`)" });
            }
            println!("Catalogues:  {}", data.display());
            let mut any = false;
            for v in &found {
                let has = |f: &str| data.join(v).join(f).is_file();
                if has("options.json") || has("ui.json") || has("models.json") {
                    any = true;
                    let yn = |b: bool| if b { "yes" } else { "no" };
                    println!(
                        "  {v}: options.json {}, ui.json {}, models.json {}",
                        yn(has("options.json")),
                        yn(has("ui.json")),
                        yn(has("models.json"))
                    );
                }
            }
            if !any {
                println!("  none exported yet (run tools/export-rcdb-options.ps1 on Windows with CPS)");
            }
        }
        RcdbAction::Import { from } => {
            let mut copied = 0;
            let cps = if from.join("RcdbJsons").is_dir() { Some(from.clone()) } else { Some(from.join("cps")).filter(|p| p.join("RcdbJsons").is_dir()) };
            if let Some(src) = cps {
                let dst = user_copy_dir().context("no per-user data folder")?;
                let n = import(&src, &dst)?;
                println!("Copied {n} RCDB files from {} to {}", src.display(), dst.display());
                copied += n;
            }
            // Exported catalogues: <from>/<version>/{options,ui,models}.json (a copied hyt/rcdb folder).
            for entry in std::fs::read_dir(&from).with_context(|| format!("reading {}", from.display()))?.flatten() {
                let dir = entry.path();
                for f in ["options.json", "ui.json", "models.json", "visibility.json", "links.json"] {
                    if dir.join(f).is_file() && dir != data.join(entry.file_name()) {
                        std::fs::create_dir_all(data.join(entry.file_name()))?;
                        std::fs::copy(dir.join(f), data.join(entry.file_name()).join(f))?;
                        println!("Copied {} {f}", entry.file_name().to_string_lossy());
                        copied += 1;
                    }
                }
            }
            if copied == 0 {
                bail!("{} is neither a CPS install (RcdbJsons) nor a copied hyt data folder", from.display());
            }
        }
    }
    Ok(())
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct Source {
    /// Serial port of the programming cable, e.g. COM3 or /dev/ttyUSB0.
    #[arg(short, long)]
    port: Option<String>,
    /// Replay a recorded CPS session (captures/*.jsonl) instead of talking to a radio.
    #[arg(long)]
    replay: Option<PathBuf>,
}

impl Source {
    fn open(&self) -> Result<(Box<dyn Transport>, String)> {
        match (&self.port, &self.replay) {
            (Some(port), _) => Ok((
                Box::new(hyt_core::open_port(port)?),
                port.clone(),
            )),
            (_, Some(path)) => Ok((
                Box::new(
                    ReplayTransport::from_jsonl_file(path)
                        .with_context(|| format!("loading {}", path.display()))?,
                ),
                format!("replay:{}", path.display()),
            )),
            _ => bail!("give --port or --replay"),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt().with_max_level(level).with_writer(std::io::stderr).init();

    match cli.command {
        Command::Ports => {
            let ports = available_ports()?;
            if ports.is_empty() {
                println!("No serial ports found.");
            }
            for (name, desc) in ports {
                println!("{name:<16} {desc}");
            }
        }
        Command::Info { source, json } => {
            let (transport, _) = source.open()?;
            let mut session = Session::new(transport);
            let info = session.identify().context("identifying the radio")?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info_entries(&info))?);
            } else {
                println!("Profile: {}", profile_for(&info).display_name);
                for e in info_entries(&info) {
                    let value = match (e.status, e.text) {
                        (0, _) if e.item == format!("0x{:04X}", hyt_core::licence::LICENCE_INFO_ITEM) => {
                            let l = info
                                .items
                                .get(&hyt_core::licence::LICENCE_INFO_ITEM)
                                .map(|i| hyt_core::licence::installed_licences(&i.value));
                            match l {
                                Some(l) if l.unrecognised => format!("hex {}", l.raw_hex),
                                Some(l) if l.codes.is_empty() => "none".to_string(),
                                Some(l) => l
                                    .codes
                                    .iter()
                                    .map(|c| hyt_core::licence::licence_name(c).unwrap_or(c).to_string())
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                None => String::new(),
                            }
                        }
                        (0, Some(t)) => t,
                        (0, None) => format!("hex {}", e.hex),
                        (s, _) => format!("(not supported, status {s})"),
                    };
                    println!("  {} {:<30} {value}", e.item, e.label);
                }
            }
        }
        Command::Read { source, out, rcdb } => {
            let (transport, source_name) = source.open()?;
            let mut session = Session::new(transport);
            let rcdb = rcdb_root(rcdb);
            let result = read_radio(&mut session, &rcdb, |entry, bytes, _, _| {
                println!("  read file {:>3} {:<28} {:>6} bytes", entry.id, entry.name, bytes.len());
            })?;
            let manifest = save_read(&result, &out, &source_name, &rcdb)?;
            let unknown_crc = manifest.files.iter().filter(|f| f.crc == "unknown").count();
            println!(
                "Read {} files from {} (serial {}, RCDB {}) into {}. CRC rule unknown for {unknown_crc} files.",
                manifest.files.len(),
                manifest.model_name.as_deref().unwrap_or("unknown model"),
                manifest.serial_number.as_deref().unwrap_or("?"),
                manifest.rcdb_version,
                out.display()
            );
        }
        Command::Apply { folder, edits, out, rcdb, options } => {
            let rcdb = rcdb_root(rcdb);
            let saved = load_config(&folder, &rcdb)?;
            let layout = load_layout(&saved, &rcdb)?;
            let links = rcdb.table_links()?;
            let options = load_options_for_read(options, &saved.manifest);
            let text = std::fs::read_to_string(&edits).with_context(|| format!("reading {}", edits.display()))?;
            let edited: Vec<DecodedFile> =
                serde_json::from_str(&text).with_context(|| format!("parsing {}", edits.display()))?;
            let (files, changes) = apply_edits(&saved, &layout, options.as_ref(), &links, edited)?;
            if changes.is_empty() {
                println!("No changes; nothing saved.");
                return Ok(());
            }
            let labels = Labeler { layout: &layout, options: options.as_ref() };
            print_changes(&changes, &labels);
            let source = format!("edited from {} with {}", folder.display(), edits.display());
            save_config(&saved, &files, &out, &source, &layout)?;
            println!("Saved {} changes into {}. Check with `hyt show`, then `hyt write --from`.", changes.len(), out.display());
        }
        Command::Diff { before, after, rcdb, options } => {
            let rcdb = rcdb_root(rcdb);
            let a = load_config(&before, &rcdb)?;
            let b = load_config(&after, &rcdb)?;
            if a.manifest.rcdb_version != b.manifest.rcdb_version {
                bail!(
                    "the folders use different RCDB layouts ({} vs {}); compare each with `hyt show`",
                    a.manifest.rcdb_version,
                    b.manifest.rcdb_version
                );
            }
            let layout = load_layout(&a, &rcdb)?;
            let options = load_options_for_read(options, &a.manifest);
            let (da, db) = (decode_read(&a, &layout)?, decode_read(&b, &layout)?);
            let mut all = Vec::new();
            for fa in &da {
                match db.iter().find(|f| f.id == fa.id) {
                    Some(fb) => match changes(fa, fb) {
                        Ok(c) => all.extend(c),
                        Err(e) => println!("  {e}"),
                    },
                    None => println!("  file {} ({}) only in {}", fa.id, fa.name, before.display()),
                }
            }
            let labels = Labeler { layout: &layout, options: options.as_ref() };
            print_changes(&all, &labels);
            println!("{} settings differ.", all.len());
        }
        Command::Decode { folder, out, labels, rcdb, options } => {
            let rcdb = rcdb_root(rcdb);
            let saved = load_config(&folder, &rcdb)?;
            let layout = load_layout(&saved, &rcdb)?;
            let mut decoded = decode_read(&saved, &layout)?;
            if labels {
                match load_options_for_read(options, &saved.manifest) {
                    Some(o) => labelize(&mut decoded, &layout, &o),
                    None => bail!("--labels needs the option catalogue (run tools/export-rcdb-options.ps1)"),
                }
            }
            let json = serde_json::to_string_pretty(&decoded)?;
            match out {
                Some(path) => {
                    std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
                    println!("Decoded {} files into {}", decoded.len(), path.display());
                }
                None => println!("{json}"),
            }
        }
        Command::Show { folder, all, rcdb, options } => {
            let rcdb = rcdb_root(rcdb);
            let saved = load_config(&folder, &rcdb)?;
            let layout = load_layout(&saved, &rcdb)?;
            let decoded = decode_read(&saved, &layout)?;
            let options = load_options_for_read(options, &saved.manifest);
            let labels = Labeler { layout: &layout, options: options.as_ref() };
            if all {
                print!("{}", full_listing(&decoded, &labels));
            } else {
                print!("{}", summary(&saved.manifest, &decoded, &labels));
            }
        }
        Command::Rcdb { action } => rcdb_command(action)?,
        Command::Tpl { action } => tpl_command(action)?,
        Command::Csv { action } => csv_command(action)?,
        Command::Licence { action } => licence_command(action)?,
        Command::Batch { action } => batch_command(action)?,
        Command::Copy { from, port, like, backup, out, radio_id, alias, allow_duplicate_id, allow_unverified, rcdb, options } => {
            let opts = CopyOptions { radio_id, alias, allow_duplicate_id, keep_config_identity: false };
            let target = match (&port, &like, &backup) {
                (Some(port), None, Some(backup)) => CopyTarget::Radio { port, backup },
                (None, Some(like), None) => CopyTarget::Read(like),
                _ => bail!("give --port and --backup, or --like"),
            };
            copy_command(&from, target, &out, &opts, &rcdb_root(rcdb), options.as_deref(), allow_unverified)?;
        }
        Command::Write { port, from, backup, rcdb, allow_unverified } => {
            write_command(&port, &from, &backup, &rcdb_root(rcdb), allow_unverified)?;
        }
    }
    Ok(())
}

fn print_changes(changes: &[Change], labels: &Labeler) {
    for c in changes {
        let object = |v: &serde_json::Value| {
            let parts: Vec<String> = v
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, _)| !k.starts_with("Reserved"))
                .take(5)
                .map(|(k, v)| format!("{k} = {}", labels.text(c.file_id, c.block_id, k, v)))
                .collect();
            format!("{{{}{}}}", parts.join(", "), if v.as_object().is_some_and(|o| o.len() > 5) { ", …" } else { "" })
        };
        let show = |v: &serde_json::Value| match v {
            serde_json::Value::Null => "-".to_string(),
            serde_json::Value::Object(_) => object(v),
            serde_json::Value::Array(items) if c.key == ADDED || c.key == REMOVED => {
                format!("{} members", items.len())
            }
            _ => labels.text(c.file_id, c.block_id, &c.key, v),
        };
        println!(
            "  file {} {} {} {}: {} -> {}",
            c.file_id,
            c.file_name,
            c.position,
            c.key,
            show(&c.old),
            show(&c.new)
        );
    }
}

/// Load the option catalogue from `explicit` or the default per-user location, narrowed to the
/// saved radio's model number (CPS's model-number rules). Notes and continues without labels if the
/// catalogue is missing or for another RCDB version, and with the generic ranges if the rules are.
fn load_options_for_read(explicit: Option<PathBuf>, manifest: &hyt_core::ReadManifest) -> Option<Options> {
    match hyt_core::load_options_for_read(explicit.as_deref(), manifest) {
        Ok((o, note)) => {
            if let Some(note) = note {
                eprintln!("note: {note}");
            }
            Some(o)
        }
        Err(note) => {
            eprintln!("note: {note}; showing raw values");
            None
        }
    }
}

fn write_command(port: &str, from: &Path, backup: &Path, rcdb: &RcdbRoot, allow_unverified: bool) -> Result<()> {
    let saved = load_config(from, rcdb)?;
    if backup.exists() {
        bail!("backup folder {} already exists; choose a new one", backup.display());
    }
    let transport = hyt_core::open_port(port)?;
    let mut session = Session::new(transport);
    println!("Taking a fresh backup into {} ...", backup.display());
    let source = from.display().to_string();
    let job = WriteJob { saved: &saved, source: &source, backup, port };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let settings = WriteSettings { allow_unverified, programming_date: Some(today), ..WriteSettings::default() };
    write_config(&mut session, rcdb, &job, &settings, |event| match event {
        WriteEvent::Prepared { plan } if plan.other_radio() || plan.conversion.converted() => print_copy_plan(plan),
        WriteEvent::Writing { files, differing } => {
            println!("Writing {files} files ({differing} differ from the radio's current contents) ...")
        }
        WriteEvent::Wrote { entry, .. } => println!("  wrote file {:>3} {}", entry.id, entry.name),
        WriteEvent::WaitingForRestart => println!("Waiting for the radio to restart, then reading back to verify ..."),
        WriteEvent::Verified { files } => println!("Verified: all {files} written files read back as written."),
        _ => {}
    })?;
    Ok(())
}

enum CopyTarget<'a> {
    Radio { port: &'a str, backup: &'a Path },
    Read(&'a Path),
}

fn print_copy_plan(plan: &CopyPlan) {
    let or = |v: &Option<String>| v.clone().unwrap_or_else(|| "?".into());
    let change = |a: &Option<String>, b: &Option<String>| {
        if a == b { format!("{} (kept)", or(a)) } else { format!("{} -> {}", or(a), or(b)) }
    };
    println!("Radio serial {}:", or(&plan.before.serial));
    println!("  Radio ID     {}", change(&plan.before.radio_id, &plan.after.radio_id));
    println!("  Radio name   {}", change(&plan.before.alias, &plan.after.alias));
    if let Some(ip) = &plan.radio_ip {
        println!("  Radio IP     {ip}");
    }
    println!(
        "  configuration from radio {} (Radio ID {}, name {})",
        or(&plan.source.serial),
        or(&plan.source.radio_id),
        or(&plan.source.alias)
    );
    println!("  {} settings differ from what this radio has", plan.changed_settings);
    for k in &plan.kept {
        println!("  kept from this radio: {k}");
    }
    if plan.shares_id_with_source() {
        println!("  note: this radio has the same Radio ID as the radio the configuration came from; give one another ID");
    }
    for line in plan.conversion.lines() {
        println!("  {line}");
    }
}

fn copy_command(
    from: &Path,
    target: CopyTarget<'_>,
    out: &Path,
    opts: &CopyOptions,
    rcdb: &RcdbRoot,
    options: Option<&Path>,
    allow_unverified: bool,
) -> Result<()> {
    let config = load_config(from, rcdb)?;
    let source = from.display().to_string();
    let (port, backup) = match target {
        CopyTarget::Read(like) => {
            let plan = prepare_folder(&config, &source, like, out, rcdb, options, opts)?;
            print_copy_plan(&plan);
            println!("Saved {}. Nothing was written; check it with `hyt diff`, then `hyt write --from` it.", out.display());
            return Ok(());
        }
        CopyTarget::Radio { port, backup } => (port, backup),
    };
    let transport = hyt_core::open_port(port)?;
    let mut session = Session::new(transport);
    println!("Reading the radio into the backup {} ...", backup.display());
    let job = CopyJob { config: &config, config_source: &source, backup, out, port, options_path: options };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let settings = WriteSettings { allow_unverified, programming_date: Some(today), ..WriteSettings::default() };
    let report = copy_to_radio(&mut session, rcdb, &job, opts, &settings, |event| match event {
        CopyEvent::Prepared(plan) => {
            print_copy_plan(plan);
            println!("Saved the configuration for this radio in {}.", out.display());
        }
        CopyEvent::Step(WriteEvent::Writing { files, differing }) => {
            println!("Writing {files} files ({differing} differ from the radio's current contents) ...")
        }
        CopyEvent::Step(WriteEvent::Wrote { entry, .. }) => println!("  wrote file {:>3} {}", entry.id, entry.name),
        CopyEvent::Step(WriteEvent::WaitingForRestart) => {
            println!("Waiting for the radio to restart, then reading back to verify ...")
        }
        CopyEvent::Step(WriteEvent::Verified { files }) => {
            println!("Verified: all {files} written files read back as written.")
        }
        _ => {}
    })?;
    println!("Done. Backup of this radio before the copy: {}", report.write.backup.display());
    Ok(())
}

fn print_batch_status(batch: &Batch) {
    let f = &batch.file;
    let c = &f.configuration;
    println!("Batch {}", batch.folder.display());
    println!(
        "Configuration: {} (from {}), radio {} ID {} \"{}\"; {} RCDB {}",
        batch.config_folder().display(),
        f.configuration_source,
        c.serial.as_deref().unwrap_or("?"),
        c.radio_id.as_deref().unwrap_or("?"),
        c.alias.as_deref().unwrap_or(""),
        f.model.as_deref().unwrap_or("?"),
        f.rcdb_version
    );
    for line in f.rules.describe() {
        println!("  {line}");
    }
    let n = batch.counts();
    println!(
        "Radios done: {} ({} attempts: {} verified, {} skipped, {} not written, {} failed){}",
        n.radios_done,
        f.radios.len(),
        n.verified,
        n.skipped,
        n.not_written,
        n.failed,
        if n.radios_failed > 0 { format!("; {} radios need writing again", n.radios_failed) } else { String::new() }
    );
    if !f.radios.is_empty() {
        println!(
            "{:>4}  {:<19}  {:<12} {:<11} {:<10} {:<16} {:<15}  message",
            "no.", "time", "serial", "result", "Radio ID", "name", "Radio IP"
        );
        for e in &f.radios {
            let a = e.after.as_ref().or(e.before.as_ref());
            println!(
                "{:>4}  {:<19}  {:<12} {:<11} {:<10} {:<16} {:<15}  {}",
                e.number,
                local_time(e.started_unix),
                e.serial.as_deref().unwrap_or("-"),
                e.outcome.text(),
                a.and_then(|a| a.radio_id.as_deref()).unwrap_or("-"),
                a.and_then(|a| a.alias.as_deref()).unwrap_or("-"),
                e.radio_ip.as_deref().unwrap_or("-"),
                e.message
            );
        }
    }
    match batch.assign(None, None) {
        Ok(a) => println!("{}", next_radio_text(&a)),
        Err(e) => println!("Next radio: {e:#}"),
    }
}

fn next_radio_text(a: &Assignment) -> String {
    format!(
        "Next new radio gets Radio ID {} and name {} (radios in the plan get their planned values).",
        a.radio_id.as_deref().unwrap_or("(its own)"),
        a.name.as_deref().map_or("(its own)".to_string(), |n| format!("\"{n}\""))
    )
}

fn batch_command(action: BatchAction) -> Result<()> {
    match action {
        BatchAction::New { folder, from, first_id, name, plan, allow_duplicate_id, rcdb, options } => {
            let plan = match plan {
                Some(p) => parse_plan(&std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?)
                    .with_context(|| format!("plan {}", p.display()))?,
                None => Vec::new(),
            };
            let radio_id = first_id.map_or(IdRule::Keep, |first| IdRule::Sequence { first });
            let rules = Rules { radio_id, name, plan, allow_duplicate_id };
            let batch = Batch::create(&folder, &from, rules, &rcdb_root(rcdb), options.as_deref())?;
            print_batch_status(&batch);
            println!("Program radios with `hyt batch program {} --port <port>`.", folder.display());
        }
        BatchAction::Status { folder, json } => {
            let batch = Batch::open(&folder)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&batch.file)?);
            } else {
                print_batch_status(&batch);
            }
        }
        BatchAction::Program { folder, port, once, again, allow_unverified, rcdb, options } => {
            let rcdb = rcdb_root(rcdb);
            let mut batch = Batch::open(&folder)?;
            print_batch_status(&batch);
            let stdin = std::io::stdin();
            loop {
                if !once {
                    println!();
                    println!("Connect the next radio (switched on) to {port} and press Enter; type q and Enter to stop.");
                    let mut line = String::new();
                    if stdin.read_line(&mut line)? == 0 || line.trim().eq_ignore_ascii_case("q") {
                        break;
                    }
                }
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                let settings = WriteSettings { allow_unverified, programming_date: Some(today), ..WriteSettings::default() };
                let transport = match hyt_core::open_port(&port) {
                    Ok(t) => t,
                    Err(e) if !once => {
                        println!("{e:#}");
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                println!("Reading the radio ...");
                let mut session = Session::new(transport);
                let entry = batch.program(&mut session, &rcdb, &port, options.as_deref(), &settings, again, |event| match event {
                    CopyEvent::Prepared(plan) => println!(
                        "Radio {}: Radio ID {} -> {}, name \"{}\" -> \"{}\"; writing ...",
                        plan.before.serial.as_deref().unwrap_or("?"),
                        plan.before.radio_id.as_deref().unwrap_or("?"),
                        plan.after.radio_id.as_deref().unwrap_or("?"),
                        plan.before.alias.as_deref().unwrap_or(""),
                        plan.after.alias.as_deref().unwrap_or("")
                    ),
                    CopyEvent::Step(WriteEvent::WaitingForRestart) => {
                        println!("Waiting for the radio to restart, then reading back to verify ...")
                    }
                    _ => {}
                })?;
                println!(
                    "#{} {} {}: {}",
                    entry.number,
                    entry.serial.as_deref().unwrap_or("(no radio)"),
                    entry.outcome.text(),
                    entry.message
                );
                if once {
                    if matches!(entry.outcome, Outcome::Failed | Outcome::NotWritten) {
                        bail!("the radio was not programmed (see above; logged in the batch)");
                    }
                    break;
                }
                match batch.assign(None, None) {
                    Ok(a) => println!("{}", next_radio_text(&a)),
                    Err(e) => println!("Next radio: {e:#}"),
                }
            }
            let n = batch.counts();
            println!("{} radios done in this batch; log in {}", n.radios_done, folder.join(hyt_core::batch::LOG_FILE).display());
        }
    }
    Ok(())
}
