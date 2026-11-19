//! Fields CPS does not show for the open radio, in two kinds the GUI can show separately:
//!
//! - **Internal**: CPS never shows them to customers. Source, as CPS decides it (observed in CPS's
//!   code, `docs/research-cps-visibility.md`): the parameter database's `Param.visible` (replaced by a
//!   matching model-number rule) must be 0, 3 or 4 and `Param.delete_flag` 0 (`visibility.json`, see
//!   [`hytera_codeplug::param_visibility`]). Without that file, `CPSParams.xlsx`'s visibility column
//!   is used as a fallback (it disagrees with the database for 138 fields of the BP515). Fields on no
//!   CPS screen at all (not in `ui.json`) are internal too.
//! - **Licence**: they belong to a feature the radio is not licensed for. Observed in CPS's code:
//!   rule `ParamTemplate.config` LinkType 20125 (`AuthorizationRelationRule`) hides these fields and
//!   whole pages/categories when the licence flag in `Comm_Basic` block 902 is Off; only Roam, Basic
//!   Encryption and Two Tone hide anything.
//!
//! Both kinds are read-only in hyt. Not modelled: the option lists CPS narrows for the same licences
//! (e.g. RX Squelch Mode without signaling choices when Two Tone is Off), what the Pure Digital and
//! Pure Analog licences do when On, and the values CPS forces when it opens a configuration.

use std::collections::{BTreeMap, HashMap, HashSet};

use hytera_codeplug::decode::DecodedFile;
use hytera_codeplug::help::{BuildVisibility, HelpCatalog};
use hytera_codeplug::layout::Layout;
use hytera_codeplug::param_visibility::{ParamVisibility, shown_by_external_build};
use hytera_codeplug::rcdb::TableLink;
use hytera_codeplug::ui::UiTree;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HiddenKind {
    /// Needs a licence the radio does not have.
    Licence,
    /// CPS never shows it to customers.
    Internal,
}

/// Why CPS does not show a field.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hidden {
    pub kind: HiddenKind,
    pub reason: String,
}

/// A licence flag (`Comm_Basic` block 902) and what CPS hides while it is Off: single fields
/// (layout ids) and whole files (pages/categories). From `AuthorizationRelationRule` (observed).
struct Licence {
    flag: &'static str,
    fields: &'static [u32],
    files: &'static [&'static str],
}

const LICENCE_FILE: &str = "Comm_Basic";

const LICENCES: &[Licence] = &[
    Licence {
        flag: "Basic Encryption",
        fields: &[
            // Digital channel Encrypt, Encrypt Key (and the channel template's); menu item Encrypt.
            32, 37, 6533, 6540, 6464,
            // Page Digital Setting › Encrypt: global Encrypt, key lengths, key list.
            1688, 6178, 6179, 6180, 6185, 6132, 6133, 6134,
        ],
        files: &[],
    },
    Licence {
        flag: "Roam",
        // IP Multi Site Connect, Beacon Interval, menu item Roam, Roam On (category Roam).
        fields: &[6186, 850, 6456, 738],
        // Category Conventional › Roam.
        files: &["QEU_C_RoamList", "QEU_C_RoamConfig"],
    },
    Licence {
        flag: "Two Tone",
        // Signaling Side Tone (+volume, menu item); analog channel Signaling Type, Personality List,
        // RX/TX Signaling System, CTCSS or CDCSS Required For Data, Auto Reset Mode/Time.
        fields: &[6500, 6501, 6665, 6343, 6348, 6345, 6346, 6491, 102, 103],
        // Category Analog Service.
        files: &[
            "QEU_C_HDC1200_System",
            "QEU_C_HDC1200_Contact",
            "QEU_C_2Tone_System",
            "QEU_C_2Tone_Contact",
            "QEU_C_2Tone_Personality",
        ],
    },
];

/// `Comm_Basic` 902 "Custom Authorization" flags (6645..=6652) and their read-only copies in
/// `Comm_RadioInfo` 920 "Feature in Subscriber" (6636..=6643): CPS shows a copy when the flag is
/// shown (observed, rule 20125 at load).
const LICENCE_FLAGS: std::ops::RangeInclusive<u32> = 6645..=6652;
const LICENCE_COPIES: u32 = 6636;

/// The licence flags of the configuration: flag name → licensed. Missing flags are absent.
fn licences(files: &[DecodedFile]) -> HashMap<&'static str, bool> {
    let Some(file) = files.iter().find(|f| f.name == LICENCE_FILE) else { return HashMap::new() };
    let mut out = HashMap::new();
    for l in LICENCES {
        let value = file
            .blocks
            .iter()
            .filter_map(|b| b.records.first()?.fields.as_ref())
            .find_map(|f| f.0.get(l.flag).and_then(Value::as_i64));
        if let Some(v) = value {
            out.insert(l.flag, v != 0);
        }
    }
    out
}

/// Fields CPS shows in another form, so the "internal" rules must not hide them: the index and type
/// fields of CPS's virtual parameters (`ComplexRuleConfig.Config`) and the call IDs of its dial
/// rules. (Not hyt's reference fields in general: CPS hides some of those, e.g. Phone System.)
pub fn shown_otherwise(config: Option<&hytera_codeplug::validation::ValidationConfig>) -> HashSet<u32> {
    let mut out = HashSet::new();
    if let Some(c) = config {
        for v in &c.virtual_params {
            out.extend(v.index_param);
            out.extend(v.type_param);
        }
        for d in &c.dial_params {
            out.insert(d.param);
            out.extend(d.call_type_param);
        }
    }
    out
}

/// What CPS's data says about one field.
pub struct Sources<'a> {
    /// `Param.visible` / `delete_flag` (`visibility.json`), when exported for this RCDB version.
    pub flags: Option<&'a ParamVisibility>,
    /// Model-number visibility for the radio (replaces `Param.visible`).
    pub model_visible: Option<&'a BTreeMap<u32, bool>>,
    /// `CPSParams.xlsx` (fallback when `flags` has no row).
    pub help: Option<&'a HelpCatalog>,
    /// CPS's screen tree.
    pub ui: Option<&'a UiTree>,
}

/// Why the internal rules hide field `id`, if they do.
fn internal_reason(id: u32, s: &Sources) -> Option<&'static str> {
    if let Some(f) = s.flags.and_then(|f| f.get(id)) {
        if f.deleted {
            return Some("CPS marks this setting as removed (internal)");
        }
        return match s.model_visible.and_then(|m| m.get(&id)) {
            Some(true) => None,
            Some(false) => Some("CPS does not show this setting for this model (internal)"),
            None if shown_by_external_build(f.visible) => None,
            None if f.visible == 2 => Some("Only Hytera's internal CPS shows this setting"),
            None => Some("CPS never shows this setting (internal)"),
        };
    }
    match s.model_visible.and_then(|m| m.get(&id)) {
        Some(true) => return None,
        Some(false) => return Some("CPS does not show this setting for this model (internal)"),
        None => {}
    }
    match s.help.and_then(|h| h.visibility(id)) {
        Some(BuildVisibility::Neither) => Some("CPS never shows this setting (internal)"),
        Some(BuildVisibility::InternalOnly) => Some("Only Hytera's internal CPS shows this setting"),
        _ => None,
    }
}

/// Classify every field of the configuration's files. Fields CPS shows are absent from the map.
/// `links` are CPS's list pairs: the name file of a two-level list (e.g. zone names) is shown as
/// the lists' names, not as fields on a screen, so "not on any screen" does not apply to it.
/// Fields in `otherwise` ([`shown_otherwise`]) and the files of hyt's list editors (`raw_lists`)
/// are never classified as internal: CPS shows e.g. a preset channel's zone and channel or a
/// one-touch call's contact in another form, and the list editors show raw values as they are.
/// The licence rules apply to them all.
pub fn classify(
    layout: &Layout,
    files: &[DecodedFile],
    sources: &Sources,
    links: &[TableLink],
    otherwise: &HashSet<u32>,
    raw_lists: &[&str],
) -> HashMap<u32, Hidden> {
    let licensed = licences(files);
    let on_screen = sources.ui.map(UiTree::param_ids);
    let unlicensed: Vec<&Licence> = LICENCES.iter().filter(|l| licensed.get(l.flag) == Some(&false)).collect();
    let licence_hidden = |reason_flag: &str| Hidden {
        kind: HiddenKind::Licence,
        reason: format!("Needs the {reason_flag} licence, which this radio does not have"),
    };
    let mut out = HashMap::new();
    for f in files {
        let list_names = links.iter().any(|l| l.one_dim == f.id);
        let raw_list = raw_lists.contains(&f.name.as_str());
        let file_licence = unlicensed.iter().find(|l| l.files.contains(&f.name.as_str()));
        for block in layout.block_ids(f.id) {
            for def in layout.fields(f.id, block) {
                if out.contains_key(&def.id) {
                    continue;
                }
                let exempt = raw_list || otherwise.contains(&def.id);
                let internal = |reason: &str| Hidden { kind: HiddenKind::Internal, reason: reason.into() };
                let hidden = (!exempt)
                    .then(|| internal_reason(def.id, sources))
                    .flatten()
                    .map(internal)
                    .or_else(|| {
                        file_licence
                            .or_else(|| unlicensed.iter().find(|l| l.fields.contains(&def.id)))
                            .map(|l| licence_hidden(l.flag))
                    })
                    .or_else(|| {
                        on_screen
                            .as_ref()
                            .filter(|s| !exempt && !list_names && !s.contains(&def.id))
                            .map(|_| internal("Not on any CPS screen (internal)"))
                    });
                if let Some(h) = hidden {
                    out.insert(def.id, h);
                }
            }
        }
    }
    // "Feature in Subscriber" shows the copy of each licence flag CPS shows.
    for (i, flag) in LICENCE_FLAGS.enumerate() {
        if !out.contains_key(&flag) {
            out.remove(&(LICENCE_COPIES + i as u32));
        }
    }
    out
}

/// Why CPS hides the whole list in `file` (all of a licence's files), if it does.
pub fn hidden_file(files: &[DecodedFile], file: &str) -> Option<Hidden> {
    let licensed = licences(files);
    LICENCES.iter().find(|l| l.files.contains(&file) && licensed.get(l.flag) == Some(&false)).map(|l| Hidden {
        kind: HiddenKind::Licence,
        reason: format!("Needs the {} licence, which this radio does not have", l.flag),
    })
}
