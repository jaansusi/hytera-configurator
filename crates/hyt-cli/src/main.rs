use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use hyt_core::edit::{ADDED, Change, REMOVED, apply_edits, changes, labelize};
use hyt_core::view::{Labeler, full_listing, summary};
use hyt_core::write_flow::{WriteEvent, WriteJob, WriteSettings, write_config};
use hyt_core::{decode_read, info_entries, load_layout, load_read, read_radio, save_edited, save_read};
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
    /// Copy the RCDB folders (and exported options.json / ui.json, if the source has them) into the
    /// per-user data folder. FROM is a CPS install folder or a copy of one, or a copied `hyt/rcdb`
    /// data folder.
    Import { from: PathBuf },
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
            let t = tpl::Template::read(&file)?;
            let reference = load_read(&like)?;
            let name = t.header.name.clone().unwrap_or_default();
            let source = format!("CPS template {} (\"{name}\")", file.display());
            let report = tpl::import(&t, &reference, &out, &source)?;
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
            let saved = load_read(&folder)?;
            let seqo = match seqo_from {
                Some(p) => Some(tpl::Template::read(&p)?),
                None => [rcdb.path().join("Templates"), Path::new(hytera_codeplug::rcdb::DEFAULT_CPS_DATA_DIR).join("Templates")]
                    .iter()
                    .find_map(|d| tpl::find_template(d, &saved.manifest.rcdb_version, saved.manifest.model_name.as_deref())),
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
            let (t, report) = tpl::export(&folder, &out, &links, opts)?;
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
                if has("options.json") || has("ui.json") {
                    any = true;
                    let yn = |b: bool| if b { "yes" } else { "no" };
                    println!("  {v}: options.json {}, ui.json {}", yn(has("options.json")), yn(has("ui.json")));
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
            // Exported catalogues: <from>/<version>/{options,ui}.json (a copied hyt/rcdb folder).
            for entry in std::fs::read_dir(&from).with_context(|| format!("reading {}", from.display()))?.flatten() {
                let dir = entry.path();
                for f in ["options.json", "ui.json"] {
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
            let manifest = save_read(&result, &out, &source_name)?;
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
            let saved = load_read(&folder)?;
            let rcdb = rcdb_root(rcdb);
            let layout = load_layout(&saved, &rcdb)?;
            let links = rcdb.table_links()?;
            let options = load_options(options, &saved.manifest.rcdb_version);
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
            save_edited(&saved, &files, &out, &format!("edited from {} with {}", folder.display(), edits.display()))?;
            println!("Saved {} changes into {}. Check with `hyt show`, then `hyt write --from`.", changes.len(), out.display());
        }
        Command::Diff { before, after, rcdb, options } => {
            let a = load_read(&before)?;
            let b = load_read(&after)?;
            if a.manifest.rcdb_version != b.manifest.rcdb_version {
                bail!("the folders have different RCDB versions ({} vs {})", a.manifest.rcdb_version, b.manifest.rcdb_version);
            }
            let layout = load_layout(&a, &rcdb_root(rcdb))?;
            let options = load_options(options, &a.manifest.rcdb_version);
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
            let saved = load_read(&folder)?;
            let layout = load_layout(&saved, &rcdb_root(rcdb))?;
            let mut decoded = decode_read(&saved, &layout)?;
            if labels {
                match load_options(options, &saved.manifest.rcdb_version) {
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
            let saved = load_read(&folder)?;
            let layout = load_layout(&saved, &rcdb_root(rcdb))?;
            let decoded = decode_read(&saved, &layout)?;
            let version = &saved.manifest.rcdb_version;
            let options = load_options(options, version);
            let labels = Labeler { layout: &layout, options: options.as_ref() };
            if all {
                print!("{}", full_listing(&decoded, &labels));
            } else {
                print!("{}", summary(&saved.manifest, &decoded, &labels));
            }
        }
        Command::Rcdb { action } => rcdb_command(action)?,
        Command::Tpl { action } => tpl_command(action)?,
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

/// Load the option catalogue from `explicit` or the default per-user location; note and continue
/// without labels if it is missing or for another RCDB version.
fn load_options(explicit: Option<PathBuf>, version: &str) -> Option<Options> {
    match hyt_core::load_options(explicit.as_deref(), version) {
        Ok(o) => Some(o),
        Err(note) => {
            eprintln!("note: {note}; showing raw values");
            None
        }
    }
}

fn write_command(port: &str, from: &Path, backup: &Path, rcdb: &RcdbRoot, allow_unverified: bool) -> Result<()> {
    let saved = load_read(from)?;
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
