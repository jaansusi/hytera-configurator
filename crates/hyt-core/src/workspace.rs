//! An opened configuration folder with pending edits, as used by the GUI.
//!
//! Every edit is applied to a working copy of the decoded files and immediately run through
//! [`apply_edits`] (the same validation and encoding as `hyt apply`); an edit that fails is not
//! kept. Channels can be added and removed and zone members added, removed and moved; other
//! records only change values (adding/removing them has not been checked against CPS).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use hytera_codeplug::decode::{DecodedBlock, DecodedFile, Fields, Record, decode};
use hytera_codeplug::layout::{FieldDef, FieldType, Layout};
use hytera_codeplug::link_rules::LinkRules;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{RcdbRoot, TableLink};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::edit::{Change, Edited, apply_edits, def_for, is_text};
use crate::view::{Labeler, is_secret};
use crate::{ReadManifest, SavedRead, decode_read, load_config, load_options_for_read, save_config};

/// Where one field value lives in the decoded files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldRef {
    pub file: u16,
    pub block: u16,
    /// 0-based position of the record in its block.
    pub record: usize,
    /// 0-based member position, for two-level records (zones).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<usize>,
    /// Field key as in [`hytera_codeplug::decode::keys`].
    pub key: String,
}

impl FieldRef {
    /// The field of `change` (positions are `[record]` or `[record.member]`, 1-based).
    pub fn of_change(change: &Change) -> Option<FieldRef> {
        let inner = change.position.strip_prefix('[')?.strip_suffix(']')?;
        let (record, member) = match inner.split_once('.') {
            Some((r, m)) => (r.parse::<usize>().ok()?, Some(m.parse::<usize>().ok()?.checked_sub(1)?)),
            None => (inner.parse::<usize>().ok()?, None),
        };
        Some(FieldRef {
            file: change.file_id,
            block: change.block_id,
            record: record.checked_sub(1)?,
            member,
            key: change.key.clone(),
        })
    }
}

/// The field map that `at` points into.
pub fn fields_at<'a>(files: &'a [DecodedFile], at: &FieldRef) -> Option<&'a Map<String, Value>> {
    let block = files.iter().find(|f| f.id == at.file)?.blocks.iter().find(|b| b.id == at.block)?;
    let record = block.records.get(at.record)?;
    match at.member {
        None => record.fields.as_ref().map(|f| &f.0),
        Some(m) => record.members.as_ref()?.get(m).map(|f| &f.0),
    }
}

fn fields_at_mut<'a>(files: &'a mut [DecodedFile], at: &FieldRef) -> Option<&'a mut Map<String, Value>> {
    let block = files.iter_mut().find(|f| f.id == at.file)?.blocks.iter_mut().find(|b| b.id == at.block)?;
    let record = block.records.get_mut(at.record)?;
    match at.member {
        None => record.fields.as_mut().map(|f| &mut f.0),
        Some(m) => record.members.as_mut()?.get_mut(m).map(|f| &mut f.0),
    }
}

/// CPS's Radio IP for `subnet` and `radio_id` (observed in CPS's code: `RadioIDRadioIPRule.JudgeRadioIP`
/// in `G2CPS.ComplexRule.G2LinkRule.dll`, link type 2024): the address `subnet.id₂.id₁.id₀` (ID bytes
/// most significant first) stored as a little-endian u32. `None` where CPS leaves Radio IP unchanged:
/// subnet outside 1–126 or ID outside 1–16776415.
pub fn radio_ip(subnet: i64, radio_id: i64) -> Option<i64> {
    if !(1..=126).contains(&subnet) || !(1..=16_776_415).contains(&radio_id) {
        return None;
    }
    Some(((radio_id & 0xFF) << 24) + ((radio_id & 0xFF00) << 8) + ((radio_id & 0xFF_0000) >> 8) + subnet)
}

/// The file holding Radio ID, Subnet and Radio IP.
const CONFIG_FILE: &str = "QEU_C_Config";

/// Position of the one single-record field `key` of the file named `file`, if there is exactly one.
pub fn single_field(files: &[DecodedFile], file: &str, key: &str) -> Option<FieldRef> {
    let f = files.iter().find(|f| f.name == file)?;
    let mut found = f
        .blocks
        .iter()
        .filter(|b| b.records.len() == 1 && b.records[0].fields.as_ref().is_some_and(|r| r.0.contains_key(key)));
    let block = found.next()?;
    found.next().is_none().then(|| FieldRef { file: f.id, block: block.id, record: 0, member: None, key: key.into() })
}

/// Keep Radio IP derived from Radio ID and Subnet as CPS does ([`radio_ip`]) when either differs
/// from `base`; when both are as in `base`, Radio IP is too (an untouched configuration stays
/// exactly as read, even where its Radio IP does not follow the rule). CPS does not let Radio IP
/// be edited, so a direct edit of it does not survive.
fn derive_radio_ip(base: &[DecodedFile], working: &mut [DecodedFile]) {
    let get = |files: &[DecodedFile], key: &str| {
        single_field(files, CONFIG_FILE, key).and_then(|at| value_at(files, &at)?.as_i64())
    };
    let (id, subnet) = (get(working, "Radio ID"), get(working, "Subnet"));
    let ip = if (id, subnet) == (get(base, "Radio ID"), get(base, "Subnet")) {
        get(base, "Radio IP")
    } else {
        id.zip(subnet).and_then(|(id, subnet)| radio_ip(subnet, id))
    };
    let Some(ip) = ip else { return };
    if let Some(r) = single_field(working, CONFIG_FILE, "Radio IP").and_then(|at| fields_at_mut(working, &at).map(|r| (r, at))) {
        r.0.insert(r.1.key, Value::from(ip));
    }
}

/// The value at `at`.
pub fn value_at<'a>(files: &'a [DecodedFile], at: &FieldRef) -> Option<&'a Value> {
    fields_at(files, at)?.get(&at.key)
}

/// Whether the GUI may edit a field of this definition holding `value`: single numbers and clean
/// text, but never passwords, reserved bytes, lists, floats or raw byte strings.
pub fn is_editable(def: &FieldDef, key: &str, value: &Value) -> bool {
    if is_secret(key) || key.starts_with("Reserved") {
        return false;
    }
    match value {
        Value::String(_) => is_text(def),
        Value::Number(n) => n.is_i64() && !is_text(def) && def.kind != FieldType::Float,
        _ => false,
    }
}

/// Convert what the user typed into the raw JSON value for field `def`, with the checks of
/// `hyt apply` (option catalogue choices, ranges and step grid; text length and characters).
pub fn parse_input(def: &FieldDef, key: &str, options: Option<&Options>, text: &str) -> Result<Value> {
    if is_secret(key) {
        bail!("{key}: programming passwords cannot be shown or edited here");
    }
    if is_text(def) {
        let units = text.encode_utf16().count();
        if units > def.length {
            bail!("{key}: at most {} characters ({units} given)", def.length);
        }
        if text.chars().any(char::is_control) {
            bail!("{key}: control characters are not allowed");
        }
        if def.kind == FieldType::AsciiString && !text.chars().all(|c| (' '..='~').contains(&c)) {
            bail!("{key}: only plain ASCII characters are allowed");
        }
        return Ok(Value::String(text.to_string()));
    }
    let text = text.trim();
    let raw = match options.and_then(|o| o.parse_display(def.id, text)) {
        Some(raw) => raw,
        None => match text.parse::<i64>() {
            // A bare number is taken as raw only when the catalogue has no display rule for the
            // field; otherwise "12.5" vs raw 12 would be ambiguous.
            Ok(raw) if options.and_then(|o| o.param(def.id)).is_none_or(|p| p.choices.is_empty() && p.scales.is_empty()) => raw,
            _ => {
                let allowed = options.map_or("a raw number (no option catalogue loaded)".into(), |o| o.describe_allowed(def.id));
                bail!("{key}: {text:?} is not a valid value; allowed: {allowed}");
            }
        },
    };
    if let Some(o) = options
        && o.allows(def.id, raw) == Some(false)
    {
        bail!("{key}: {text:?} is not allowed; allowed: {}", o.describe_allowed(def.id));
    }
    Ok(Value::from(raw))
}

/// A channel list that the GUI can add to and remove from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Digital,
    Analog,
}

impl ChannelKind {
    /// File holding the list.
    pub fn file(self) -> &'static str {
        match self {
            ChannelKind::Digital => "QEU_C_DChannel",
            ChannelKind::Analog => "QEU_C_AChannel",
        }
    }

    /// Zone member `Channel Type` value for this list.
    pub fn member_type(self) -> i64 {
        match self {
            ChannelKind::Digital => 0,
            ChannelKind::Analog => 1,
        }
    }

    pub fn from_member_type(t: i64) -> Option<Self> {
        match t {
            0 => Some(ChannelKind::Digital),
            1 => Some(ChannelKind::Analog),
            _ => None,
        }
    }
}

const ZONE_FILE: &str = "QEU_C_Zone";

/// (`Channel Type`, `Channel List`) of a zone member.
fn member(m: &Fields) -> Option<(i64, i64)> {
    Some((m.0.get("Channel Type")?.as_i64()?, m.0.get("Channel List")?.as_i64()?))
}

/// Pending edits saved by [`Workspace::checkpoint`].
#[derive(Clone)]
pub struct Checkpoint {
    working: Vec<DecodedFile>,
    view: Vec<DecodedFile>,
    pending: Option<Edited>,
}

/// An opened folder and its pending edits.
///
/// Two forms of the configuration are kept. The *edit form* ([`Workspace::edit_form`]) is what
/// [`apply_edits`] takes (and `hyt apply` reads from JSON): records keep the `index` of the record
/// they came from (added ones have none) and zone members use the channel numbering of the opened
/// folder, with added channels numbered after it. The *view* ([`Workspace::view`]) is the result,
/// decoded, as it will be written. Record and zone member positions are the same in both, so
/// positions in the view also address the edit form.
pub struct Workspace {
    pub folder: PathBuf,
    pub saved: SavedRead,
    pub layout: Layout,
    pub options: Option<Options>,
    /// Why no option catalogue is loaded, when it is not.
    pub options_note: Option<String>,
    /// Why the catalogue is not narrowed to the radio's model number (generic ranges), when it is not.
    pub model_note: Option<String>,
    /// CPS's screen structure (`ui.json` next to the option catalogue), for the settings view.
    pub ui: Option<hytera_codeplug::ui::UiTree>,
    /// Fields that refer to other records (from CPS's `DataValidationConfig.config`).
    pub refs: crate::refs::References,
    /// Why references use hyt's built-in table instead of CPS's configuration, when they do.
    pub refs_note: Option<String>,
    /// CPS's link rules for this configuration (`links.json` next to the option catalogue).
    pub rules: crate::link_rules::Links,
    /// Why no link rules are applied, when they are not.
    pub rules_note: Option<String>,
    /// CPS's help texts for fields (`CPSParams.xlsx` in the RCDB root), when found.
    pub help: Option<hytera_codeplug::help::HelpCatalog>,
    /// Fields CPS does not show for this radio, by layout field id ([`crate::visibility`]).
    pub hidden: std::collections::HashMap<u32, crate::visibility::Hidden>,
    links: Vec<TableLink>,
    base: Vec<DecodedFile>,
    working: Vec<DecodedFile>,
    view: Vec<DecodedFile>,
    /// Result of applying the edit form, when it differs from the folder.
    pending: Option<Edited>,
}

impl Workspace {
    /// Open a folder saved by `hyt read` (or by [`Workspace::save_as`]), loading the layout from
    /// `rcdb` and the option catalogue from `options_path` or its default location.
    pub fn open(folder: &Path, rcdb: &RcdbRoot, options_path: Option<&Path>) -> Result<Self> {
        Self::from_saved(load_config(folder, rcdb)?, folder, rcdb, options_path)
    }

    /// [`Workspace::open`] for a configuration already in memory; `folder` is where it is said
    /// to come from (used in messages and as the source of [`Workspace::save_as`]).
    pub fn from_saved(saved: SavedRead, folder: &Path, rcdb: &RcdbRoot, options_path: Option<&Path>) -> Result<Self> {
        // Exported CPS data (screens, link rules, visibility, model rules) may be another RCDB
        // version's than the radio's layout ([`crate::data_version`]).
        let version = crate::data_version(&saved.manifest.rcdb_version);
        let layout = crate::load_layout(&saved, rcdb)?;
        let links = rcdb.table_links().context("loading TableLink.xml")?;
        let (options, options_note, model_note) = match load_options_for_read(options_path, &saved.manifest) {
            Ok((o, model_note)) => (Some(o), None, model_note),
            Err(note) => (None, Some(note), None),
        };
        let base = decode_read(&saved, &layout)?;
        let ui_path = match options_path {
            Some(p) => Some(p.with_file_name("ui.json")),
            None => hytera_codeplug::ui::UiTree::default_path(&version),
        };
        let ui = ui_path.and_then(|p| hytera_codeplug::ui::UiTree::load(&p).ok()).filter(|u| u.rcdb_version == version);
        let (config, refs_note) = match rcdb.validation_config() {
            Ok(c) => (Some(c), None),
            Err(e) => (None, Some(format!("CPS's reference configuration not found ({e}); using hyt's built-in references"))),
        };
        let refs = crate::refs::References::new(config.as_ref(), &layout, &links, &base);
        let rules_path = match options_path {
            Some(p) => Some(p.with_file_name("links.json")),
            None => LinkRules::default_path(&version),
        };
        let (rules, rules_note) = match rules_path.map(|p| LinkRules::load(&p)) {
            Some(Ok(r)) if r.rcdb_version == version => (crate::link_rules::Links::new(&r, &layout, &base), None),
            Some(Ok(r)) => (Default::default(), Some(format!("link rules are for RCDB {}, not {version}", r.rcdb_version))),
            Some(Err(e)) => (Default::default(), Some(format!("no link rules ({e}); run tools/export-rcdb-links.ps1"))),
            None => (Default::default(), Some("no per-user data folder for the link rules".into())),
        };
        // Help is optional: without it fields simply have no help text.
        let help = rcdb.help_catalog().ok();
        let otherwise = crate::visibility::shown_otherwise(config.as_ref());
        let raw_lists: Vec<&str> = crate::lists::list_files().collect();
        let flags_path = match options_path {
            Some(p) => Some(p.with_file_name("visibility.json")),
            None => hytera_codeplug::param_visibility::ParamVisibility::default_path(&version),
        };
        let flags = flags_path
            .and_then(|p| hytera_codeplug::param_visibility::ParamVisibility::load(&p).ok())
            .filter(|f| f.rcdb_version == version);
        let model = hytera_codeplug::models::ModelRules::default_path(&version)
            .and_then(|p| hytera_codeplug::models::ModelRules::load(&p).ok())
            .filter(|r| r.rcdb_version == version)
            .zip(crate::model_number(&saved.manifest))
            .map(|(r, m)| r.for_model(m));
        let sources = crate::visibility::Sources {
            flags: flags.as_ref(),
            model_visible: model.as_ref().map(|m| &m.visible),
            help: help.as_ref(),
            ui: ui.as_ref(),
        };
        let hidden = crate::visibility::classify(&layout, &base, &sources, &links, &otherwise, &raw_lists);
        Ok(Workspace {
            folder: folder.to_path_buf(),
            saved,
            layout,
            options,
            options_note,
            model_note,
            ui,
            refs,
            refs_note,
            rules,
            rules_note,
            help,
            hidden,
            links,
            working: base.clone(),
            view: base.clone(),
            base,
            pending: None,
        })
    }

    pub fn labeler(&self) -> Labeler<'_> {
        Labeler { layout: &self.layout, options: self.options.as_ref() }
    }

    pub fn manifest(&self) -> &ReadManifest {
        &self.saved.manifest
    }

    /// Decoded files as saved in the folder.
    pub fn base(&self) -> &[DecodedFile] {
        &self.base
    }

    /// The configuration including pending edits, decoded (what will be saved or written).
    pub fn view(&self) -> &[DecodedFile] {
        &self.view
    }

    /// Pending edits in [`apply_edits`]'s form (see the type's documentation).
    pub fn edit_form(&self) -> &[DecodedFile] {
        &self.working
    }

    /// Pending changes (as `hyt apply` reports them).
    pub fn changes(&self) -> &[Change] {
        self.pending.as_ref().map_or(&[], |(_, c)| c.as_slice())
    }

    pub fn has_changes(&self) -> bool {
        !self.changes().is_empty()
    }

    /// All files including pending edits, ready to save or write.
    pub fn files(&self) -> &BTreeMap<u16, Vec<u8>> {
        self.pending.as_ref().map_or(&self.saved.files, |(f, _)| f)
    }

    /// CPS's two-level/one-level list file pairs (`TableLink.xml`).
    pub fn links(&self) -> &[TableLink] {
        &self.links
    }

    pub fn def(&self, at: &FieldRef) -> Option<&FieldDef> {
        def_for(&self.layout, at.file, at.block, &at.key)
    }

    /// The index in the opened folder of the record at view position `record`, or `None` for a
    /// record added since.
    pub fn original_index(&self, file: u16, block: u16, record: usize) -> Option<usize> {
        let b = self.working.iter().find(|f| f.id == file)?.blocks.iter().find(|b| b.id == block)?;
        b.records.get(record)?.index
    }

    /// Value of the field at `at` (a view position) in the opened folder; `None` for added
    /// records.
    pub fn original_value(&self, at: &FieldRef) -> Option<&Value> {
        let index = self.original_index(at.file, at.block, at.record)?;
        value_at(&self.base, &FieldRef { record: index, ..at.clone() })
    }

    /// Reference choices for the field at `at` (what can be chosen), if it refers to other records.
    pub fn reference_choices(&self, at: &FieldRef) -> Option<Vec<(i64, String)>> {
        let record = fields_at(&self.view, &FieldRef { member: None, ..at.clone() })?;
        self.refs.choices(&self.view, at, record)
    }

    /// Names of every value the reference field at `at` can name (for showing its value).
    pub fn reference_names(&self, at: &FieldRef) -> Option<Vec<(i64, String)>> {
        let record = fields_at(&self.view, &FieldRef { member: None, ..at.clone() })?;
        self.refs.names(&self.view, at, record)
    }

    /// Why CPS would not let the field at `at` be changed now (a link rule greys it out or
    /// hides it), if so.
    pub fn restriction(&self, at: &FieldRef) -> Option<crate::link_rules::Restriction> {
        let name = |f: &FieldRef, raw: i64| crate::refs::label(&self.reference_names(f)?, raw);
        self.rules.restriction(&self.view, self.options.as_ref(), &name, at)
    }

    /// Set the field at `at` (a view position, not a zone member) from user input: display text
    /// such as `"444.725"` or `"High"`, a referenced record's name, or a raw number where the
    /// catalogue knows nothing. On error nothing changes.
    pub fn set_value(&mut self, at: &FieldRef, text: &str) -> Result<()> {
        if at.member.is_some() {
            bail!("zone members are changed with the member actions (add, remove, move)");
        }
        let def = self.def(at).ok_or_else(|| anyhow!("file {} block {}: unknown field {:?}", at.file, at.block, at.key))?;
        let current = value_at(&self.view, at).ok_or_else(|| anyhow!("no field {:?} at that position", at.key))?;
        if !is_editable(def, &at.key, current) {
            bail!("{} cannot be edited here", at.key);
        }
        if let Some(r) = self.restriction(at) {
            bail!("{} cannot be changed: {}", at.key, r.reason);
        }
        if let Some(h) = self.hidden.get(&def.id) {
            bail!("{} cannot be changed here: {}", at.key, h.reason);
        }
        let value = match (self.reference_choices(at), self.reference_names(at)) {
            (Some(choices), Some(names)) => match crate::refs::parse(&choices, text) {
                Some(raw) => Value::from(raw),
                None if crate::refs::parse(&names, text).is_some() => {
                    bail!("{}: {:?} cannot be chosen: {}", at.key, text.trim(), self.refs.why_not(at))
                }
                None => parse_input(def, &at.key, self.options.as_ref(), text)?,
            },
            _ => parse_input(def, &at.key, self.options.as_ref(), text)?,
        };
        // Values CPS sets along with this one (link rules), where the catalogue allows them.
        let forced: Vec<(FieldRef, Value)> = match value.as_i64() {
            Some(raw) if value_at(&self.working, at) != Some(&value) => self
                .rules
                .forced(&self.working, at, raw)
                .into_iter()
                .filter(|(f, v)| {
                    let Some(d) = self.def(f).filter(|_| f != at) else { return false };
                    let current = value_at(&self.working, f);
                    current.is_some_and(|c| is_editable(d, &f.key, c) && c.as_i64() != Some(*v))
                        && self.options.as_ref().and_then(|o| o.allows(d.id, *v)) != Some(false)
                })
                .map(|(f, v)| (f, Value::from(v)))
                .collect(),
            _ => Vec::new(),
        };
        self.update(|w| {
            for (f, v) in std::iter::once((at, &value)).chain(forced.iter().map(|(f, v)| (f, v))) {
                let map = fields_at_mut(w, f).ok_or_else(|| anyhow!("no record at that position"))?;
                map.insert(f.key.clone(), v.clone());
            }
            Ok(())
        })
    }

    /// Undo the pending change of one field of a record that exists in the opened folder.
    pub fn revert(&mut self, at: &FieldRef) -> Result<()> {
        let original = self
            .original_value(at)
            .ok_or_else(|| anyhow!("this record was added; remove it instead of reverting"))?
            .clone();
        self.set_raw(at, original)
    }

    /// Set a raw value without the checks of [`Workspace::set_value`] (still validated by
    /// [`apply_edits`]).
    pub(crate) fn set_raw(&mut self, at: &FieldRef, value: Value) -> Result<()> {
        self.update(|w| {
            let map = fields_at_mut(w, at).ok_or_else(|| anyhow!("no record at that position"))?;
            map.insert(at.key.clone(), value);
            Ok(())
        })
    }

    /// The pending edits as they are now, to go back to with [`Workspace::restore`] (for actions
    /// made of several edits that must all succeed, and for previews).
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint { working: self.working.clone(), view: self.view.clone(), pending: self.pending.clone() }
    }

    /// Go back to the pending edits of `checkpoint` (taken from this workspace).
    pub fn restore(&mut self, checkpoint: Checkpoint) {
        self.working = checkpoint.working;
        self.view = checkpoint.view;
        self.pending = checkpoint.pending;
    }

    /// Drop all pending edits.
    pub fn discard(&mut self) {
        self.working = self.base.clone();
        self.view = self.base.clone();
        self.pending = None;
    }

    /// Change a copy of the edit form with `f`, apply it ([`apply_edits`]) and keep it only if
    /// that succeeds.
    fn update(&mut self, f: impl FnOnce(&mut Vec<DecodedFile>) -> Result<()>) -> Result<()> {
        let mut working = self.working.clone();
        f(&mut working)?;
        derive_radio_ip(&self.base, &mut working);
        if working == self.working {
            return Ok(());
        }
        let edits: Vec<DecodedFile> =
            working.iter().zip(&self.base).filter(|(w, b)| w != b).map(|(w, _)| w.clone()).collect();
        let pending = if edits.is_empty() {
            None
        } else {
            let (files, changes) = apply_edits(&self.saved, &self.layout, self.options.as_ref(), &self.links, edits)?;
            (!changes.is_empty()).then_some((files, changes))
        };
        let view = match &pending {
            None => self.base.clone(),
            Some((files, _)) => self
                .base
                .iter()
                .map(|b| match files.get(&b.id) {
                    Some(bytes) if self.saved.files.get(&b.id) != Some(bytes) => decode(bytes, &b.name, &self.layout),
                    _ => Ok(b.clone()),
                })
                .collect::<hytera_codeplug::Result<Vec<_>>>()?,
        };
        self.working = working;
        self.view = view;
        self.pending = pending;
        Ok(())
    }

    fn channel_block(files: &[DecodedFile], kind: ChannelKind) -> Result<&DecodedBlock> {
        files
            .iter()
            .find(|f| f.name == kind.file())
            .and_then(|f| f.blocks.first())
            .with_context(|| format!("this configuration has no {}", kind.file()))
    }

    /// Number of channels of `kind` in the view.
    pub fn channel_count(&self, kind: ChannelKind) -> usize {
        Self::channel_block(&self.view, kind).map_or(0, |b| b.records.len())
    }

    /// Edit-form numbers (the opened folder's numbering, added channels after it) of the channels
    /// at each view position.
    fn edit_numbers(&self, kind: ChannelKind) -> Result<Vec<i64>> {
        let base_count = Self::channel_block(&self.base, kind)?.records.len() as i64;
        let mut added = 0;
        Ok(Self::channel_block(&self.working, kind)?
            .records
            .iter()
            .map(|r| match r.index {
                Some(i) => i as i64 + 1,
                None => {
                    added += 1;
                    base_count + added
                }
            })
            .collect())
    }

    /// Append a channel, starting from CPS's template for that list (as CPS does). Returns its
    /// view position.
    pub fn add_channel(&mut self, kind: ChannelKind) -> Result<usize> {
        let block = Self::channel_block(&self.working, kind)?;
        if block.records.len() >= usize::from(block.capacity) {
            bail!("the radio holds at most {} {kind:?} channels", block.capacity);
        }
        let position = block.records.len();
        self.update(|w| {
            let file = w.iter_mut().find(|f| f.name == kind.file()).context("no channel list")?;
            file.blocks[0].records.push(Record { index: None, fields: Some(Fields(Map::new())), members: None });
            Ok(())
        })?;
        Ok(position)
    }

    /// Append a copy of the channel at view position `source`: a new channel (added like CPS adds
    /// one) whose every field is set to the source channel's value. Returns the new position.
    /// How CPS copies channels has not been recorded; the result is the same as adding a channel
    /// and typing in all of the source's values.
    pub fn duplicate_channel(&mut self, kind: ChannelKind, source: usize) -> Result<usize> {
        let block = Self::channel_block(&self.view, kind)?;
        let fields = block
            .records
            .get(source)
            .and_then(|r| r.fields.clone())
            .with_context(|| format!("there is no {kind:?} channel {}", source + 1))?;
        if block.records.len() >= usize::from(block.capacity) {
            bail!("the radio holds at most {} {kind:?} channels", block.capacity);
        }
        let position = block.records.len();
        self.update(|w| {
            let file = w.iter_mut().find(|f| f.name == kind.file()).context("no channel list")?;
            file.blocks[0].records.push(Record { index: None, fields: Some(fields), members: None });
            Ok(())
        })?;
        Ok(position)
    }

    /// Reset every field of the channel at view position `position` to CPS's channel template
    /// (`QEU_C_Template_DChannel` / `…AChannel`, the record CPS copies when it adds a channel), so
    /// the channel is as if newly added but keeps its number (and so every reference to it).
    pub fn reset_channel(&mut self, kind: ChannelKind, position: usize) -> Result<()> {
        let template_name = kind.file().replacen("QEU_C_", "QEU_C_Template_", 1);
        let template = self
            .view
            .iter()
            .find(|f| f.name == template_name)
            .and_then(|f| f.blocks.first())
            .and_then(|b| b.records.first())
            .and_then(|r| r.fields.clone())
            .with_context(|| format!("this configuration has no {template_name}"))?;
        self.update(|w| {
            let file = w.iter_mut().find(|f| f.name == kind.file()).context("no channel list")?;
            let fields = file.blocks[0]
                .records
                .get_mut(position)
                .and_then(|r| r.fields.as_mut())
                .with_context(|| format!("there is no {kind:?} channel {}", position + 1))?;
            for (key, value) in template.0 {
                if let Some(slot) = fields.0.get_mut(&key) {
                    *slot = value;
                }
            }
            Ok(())
        })
    }

    /// Remove the channel at view position `position`. Zone members pointing to it are removed and
    /// later channels renumbered, as CPS does; other references to an affected channel make this
    /// fail (see [`apply_edits`]).
    pub fn remove_channel(&mut self, kind: ChannelKind, position: usize) -> Result<()> {
        let numbers = self.edit_numbers(kind)?;
        let removed = *numbers.get(position).with_context(|| format!("there is no {kind:?} channel {}", position + 1))?;
        let base_count = Self::channel_block(&self.base, kind)?.records.len() as i64;
        let was_added = removed > base_count;
        let t = kind.member_type();
        self.update(|w| {
            let file = w.iter_mut().find(|f| f.name == kind.file()).context("no channel list")?;
            file.blocks[0].records.remove(position);
            let zones = w.iter_mut().filter(|f| f.name == ZONE_FILE).flat_map(|f| f.blocks.iter_mut());
            for members in zones.flat_map(|b| b.records.iter_mut()).filter_map(|r| r.members.as_mut()) {
                members.retain(|m| member(m) != Some((t, removed)));
                if was_added {
                    // Channels added after this one move up in the edit numbering.
                    for m in members.iter_mut() {
                        if let Some((mt, n)) = member(m)
                            && mt == t
                            && n > removed
                        {
                            m.0.insert("Channel List".into(), Value::from(n - 1));
                        }
                    }
                }
            }
            Ok(())
        })
    }

    fn zone_members_mut(w: &mut [DecodedFile], zone: usize) -> Result<&mut Vec<Fields>> {
        w.iter_mut()
            .find(|f| f.name == ZONE_FILE)
            .and_then(|f| f.blocks.first_mut())
            .and_then(|b| b.records.get_mut(zone))
            .and_then(|r| r.members.as_mut())
            .with_context(|| format!("there is no zone {}", zone + 1))
    }

    /// Members of zone `zone` (view position) as (member type, 1-based channel number in the view).
    pub fn zone_members(&self, zone: usize) -> Vec<(i64, i64)> {
        self.view
            .iter()
            .find(|f| f.name == ZONE_FILE)
            .and_then(|f| f.blocks.first())
            .and_then(|b| b.records.get(zone))
            .and_then(|r| r.members.as_ref())
            .map_or_else(Vec::new, |m| m.iter().filter_map(member).collect())
    }

    /// Append channel number `channel` (1-based, as in the view) of `kind` to zone `zone`.
    pub fn add_zone_member(&mut self, zone: usize, kind: ChannelKind, channel: usize) -> Result<()> {
        let numbers = self.edit_numbers(kind)?;
        let number = *channel
            .checked_sub(1)
            .and_then(|i| numbers.get(i))
            .with_context(|| format!("there is no {kind:?} channel {channel}"))?;
        if self.zone_members(zone).contains(&(kind.member_type(), channel as i64)) {
            bail!("that channel is already in zone {}", zone + 1);
        }
        self.update(|w| {
            let members = Self::zone_members_mut(w, zone)?;
            let mut m = Map::new();
            m.insert("Channel List".into(), Value::from(number));
            m.insert("Channel Type".into(), Value::from(kind.member_type()));
            members.push(Fields(m));
            Ok(())
        })
    }

    /// Remove member `member` (0-based) of zone `zone`.
    pub fn remove_zone_member(&mut self, zone: usize, member: usize) -> Result<()> {
        self.update(|w| {
            let members = Self::zone_members_mut(w, zone)?;
            if member >= members.len() {
                bail!("zone {} has no member {}", zone + 1, member + 1);
            }
            members.remove(member);
            Ok(())
        })
    }

    /// Move member `member` of zone `zone` one place up (`up`) or down.
    pub fn move_zone_member(&mut self, zone: usize, member: usize, up: bool) -> Result<()> {
        self.update(|w| {
            let members = Self::zone_members_mut(w, zone)?;
            let other = if up { member.checked_sub(1) } else { Some(member + 1) };
            let Some(other) = other.filter(|&o| o < members.len() && member < members.len()) else {
                bail!("cannot move that member further");
            };
            // Swap only the channel, keeping each slot's other bytes, so moving back restores
            // the original exactly.
            for key in ["Channel List", "Channel Type"] {
                let (a, b) = (members[member].0.get(key).cloned(), members[other].0.get(key).cloned());
                if let (Some(a), Some(b)) = (a, b) {
                    members[member].0.insert(key.into(), b);
                    members[other].0.insert(key.into(), a);
                }
            }
            Ok(())
        })
    }

    /// Save the configuration including pending edits as the new configuration folder `out`
    /// ([`crate::save_config`]), then continue editing from that folder.
    pub fn save_as(&mut self, out: &Path) -> Result<ReadManifest> {
        let source = format!("edited in hyt-gui from {}", self.folder.display());
        let manifest = save_config(&self.saved, self.files(), out, &source, &self.layout)?;
        let radio = Some(Box::new(self.saved.radio_read().clone()));
        let saved = SavedRead { manifest: manifest.clone(), files: self.files().clone(), radio };
        let base = decode_read(&saved, &self.layout)?;
        self.folder = out.to_path_buf();
        self.saved = saved;
        self.working = base.clone();
        self.view = base.clone();
        self.base = base;
        self.pending = None;
        Ok(manifest)
    }
}
