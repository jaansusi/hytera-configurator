use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use hyt_core::edit::{ADDED, Change, REMOVED, apply_edits, changes, labelize};
use hyt_core::view::{Labeler, full_listing, summary};
use hyt_core::write_flow::{WriteEvent, WriteJob, WriteSettings, write_config};
use hyt_core::{decode_read, info_entries, load_layout, load_read, read_radio, save_edited, save_read};
use hytera_codeplug::decode::DecodedFile;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_models::profile_for;
use hytera_proto::Session;
use hytera_proto::transport::{ReplayTransport, SerialTransport, Transport, available_ports};

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
        #[arg(long, env = "HYT_RCDB", default_value = DEFAULT_CPS_DATA_DIR)]
        rcdb: PathBuf,
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
        #[arg(long, env = "HYT_RCDB", default_value = DEFAULT_CPS_DATA_DIR)]
        rcdb: PathBuf,
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
        #[arg(long, env = "HYT_RCDB", default_value = DEFAULT_CPS_DATA_DIR)]
        rcdb: PathBuf,
        #[arg(long, env = "HYT_OPTIONS")]
        options: Option<PathBuf>,
    },
    /// Show which settings differ between two saved folders.
    Diff {
        /// The "before" folder.
        before: PathBuf,
        /// The "after" folder.
        after: PathBuf,
        #[arg(long, env = "HYT_RCDB", default_value = DEFAULT_CPS_DATA_DIR)]
        rcdb: PathBuf,
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
        #[arg(long, env = "HYT_RCDB", default_value = DEFAULT_CPS_DATA_DIR)]
        rcdb: PathBuf,
        /// Option catalogue from tools/export-rcdb-options.ps1 (default: the per-user data folder).
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
        #[arg(long, env = "HYT_RCDB", default_value = DEFAULT_CPS_DATA_DIR)]
        rcdb: PathBuf,
        /// Write even though writing has not been verified for this model yet.
        #[arg(long)]
        allow_unverified: bool,
    },
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
                Box::new(SerialTransport::open(port).with_context(|| format!("opening {port}"))?),
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
            let rcdb = RcdbRoot::new(rcdb);
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
            let rcdb = RcdbRoot::new(rcdb);
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
            let layout = load_layout(&a, &RcdbRoot::new(rcdb))?;
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
            let layout = load_layout(&saved, &RcdbRoot::new(rcdb))?;
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
            let layout = load_layout(&saved, &RcdbRoot::new(rcdb))?;
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
        Command::Write { port, from, backup, rcdb, allow_unverified } => {
            write_command(&port, &from, &backup, &RcdbRoot::new(rcdb), allow_unverified)?;
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
    let transport = SerialTransport::open(port).with_context(|| format!("opening {port}"))?;
    let mut session = Session::new(transport);
    println!("Taking a fresh backup into {} ...", backup.display());
    let source = from.display().to_string();
    let job = WriteJob { saved: &saved, source: &source, backup, port };
    let settings = WriteSettings { allow_unverified, ..WriteSettings::default() };
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
