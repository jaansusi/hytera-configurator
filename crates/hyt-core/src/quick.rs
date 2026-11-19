//! Quick setup: a short list of plain questions ("Do you want key beeps?") for people who do not
//! want to go through every setting. Each answer sets a few settings to common values; the result
//! is ordinary pending edits (validated like any edit, listed under Changes, saved or written as
//! usual). Nothing here talks to the radio.
//!
//! Questions are data ([`QUESTIONS`]): settings are named by file and field name, values by CPS's
//! option text. For the open radio, a question is offered only when CPS's settings view shows every
//! setting it changes (model-number rules) and every value is allowed; answers whose values are not
//! allowed are dropped. The preset values are hyt's suggestions, not CPS's.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, anyhow, bail};
use hytera_codeplug::help::ParamHelp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tables::{Cell, ChangeView, Column, Editor, cell, change_list, column, help_for};
use crate::workspace::{ChannelKind, FieldRef, Workspace, is_editable, parse_input, value_at};

/// Where a preset value goes.
#[derive(Debug, Clone, Copy)]
enum Target {
    /// A single-record setting: file name and field name.
    Setting(&'static str, &'static str),
    /// The same field of every digital and every analog channel (field names of the two lists).
    Channels { digital: &'static str, analog: &'static str },
}

#[derive(Clone, Copy)]
struct Preset(Target, &'static str);

struct AnswerSpec {
    id: &'static str,
    label: &'static str,
    detail: &'static str,
    /// Applied in this order (switches before the settings they enable).
    presets: &'static [Preset],
}

struct ValueSpec {
    file: &'static str,
    key: &'static str,
    label: &'static str,
    hint: &'static str,
}

enum Kind {
    Choice(&'static [AnswerSpec]),
    Values(&'static [ValueSpec]),
    Channel,
}

struct QuestionSpec {
    id: &'static str,
    title: &'static str,
    intro: &'static str,
    note: Option<&'static str>,
    /// The answer preselected when starting fresh (choice questions).
    recommended: Option<&'static str>,
    kind: Kind,
}

const UI: &str = "Comm_UI";
const SERVICE: &str = "Comm_Service_Setting";
const CONFIG: &str = "QEU_C_Config";
const BASIC: &str = "Comm_Basic";

use Target::{Channels, Setting};

const TONES_ON: [Preset; 5] = [
    Preset(Setting(UI, "Talk Permit Tone"), "On"),
    Preset(Setting(UI, "Call End Tone"), "On"),
    Preset(Setting(UI, "Low Battery Tone"), "On"),
    Preset(Setting(UI, "Keypad Tone"), "On"),
    Preset(Setting(UI, "Power Up or Down Tone"), "On"),
];

/// The questions, in the order asked.
const QUESTIONS: &[QuestionSpec] = &[
    QuestionSpec {
        id: "identity",
        recommended: None,
        title: "Your radio",
        intro: "Give the radio a name and its own number.",
        note: None,
        kind: Kind::Values(&[
            ValueSpec {
                file: BASIC,
                key: "Radio Alias",
                label: "Radio name",
                hint: "A name to tell your radios apart.",
            },
            ValueSpec {
                file: CONFIG,
                key: "Radio ID",
                label: "Radio ID",
                hint: "This radio's number on digital channels. Every radio must have a different one. \
                       Licensed radio amateurs use the ID registered for them; in a private group, number the radios 1, 2, 3, …",
            },
        ]),
    },
    QuestionSpec {
        id: "clock",
        recommended: None,
        title: "Language and clock",
        intro: "Which language should the radio use, and how should it show the time?",
        note: None,
        kind: Kind::Values(&[
            ValueSpec { file: BASIC, key: "Language", label: "Language", hint: "" },
            ValueSpec { file: BASIC, key: "Time Zone", label: "Time zone", hint: "" },
            ValueSpec { file: BASIC, key: "Date Format", label: "Date format", hint: "" },
            ValueSpec { file: BASIC, key: "Time Format", label: "Time format", hint: "" },
        ]),
    },
    QuestionSpec {
        id: "sounds",
        recommended: Some("all"),
        title: "Sounds",
        intro: "Which beeps and alert tones should the radio make?",
        note: None,
        kind: Kind::Choice(&[
            AnswerSpec {
                id: "all",
                label: "All tones",
                detail: "Key beeps, a beep when you may start talking, call end, power on/off and low battery tones.",
                presets: &[
                    Preset(Setting(UI, "Radio Silent"), "Silent Off"),
                    TONES_ON[0],
                    TONES_ON[1],
                    TONES_ON[2],
                    TONES_ON[3],
                    TONES_ON[4],
                ],
            },
            AnswerSpec {
                id: "quiet",
                label: "Useful tones only",
                detail: "Keeps the talk permit, call end and low battery tones, but no beep for every key press or at power on/off.",
                presets: &[
                    Preset(Setting(UI, "Radio Silent"), "Silent Off"),
                    TONES_ON[0],
                    TONES_ON[1],
                    TONES_ON[2],
                    Preset(Setting(UI, "Keypad Tone"), "Off"),
                    Preset(Setting(UI, "Power Up or Down Tone"), "Off"),
                ],
            },
            AnswerSpec {
                id: "silent",
                label: "Silent",
                detail: "The radio makes no alert tones at all.",
                presets: &[Preset(Setting(UI, "Radio Silent"), "Silent On")],
            },
        ]),
    },
    QuestionSpec {
        id: "screen",
        recommended: Some("15"),
        title: "Screen light",
        intro: "How long should the screen stay lit after you last used the radio?",
        note: Some("A shorter time saves battery."),
        kind: Kind::Choice(&[
            AnswerSpec { id: "5", label: "5 seconds", detail: "", presets: &[Preset(Setting(UI, "NB Screen Mode"), "5s")] },
            AnswerSpec { id: "15", label: "15 seconds", detail: "", presets: &[Preset(Setting(UI, "NB Screen Mode"), "15s")] },
            AnswerSpec { id: "30", label: "30 seconds", detail: "", presets: &[Preset(Setting(UI, "NB Screen Mode"), "30s")] },
            AnswerSpec { id: "always", label: "Always on", detail: "Uses the most battery.", presets: &[Preset(Setting(UI, "NB Screen Mode"), "Always on")] },
        ]),
    },
    QuestionSpec {
        id: "keylock",
        recommended: Some("off"),
        title: "Key lock",
        intro: "Should the keys lock by themselves when the radio is not used, so they are not pressed by accident in a pocket or bag?",
        note: None,
        kind: Kind::Choice(&[
            AnswerSpec { id: "off", label: "No", detail: "", presets: &[Preset(Setting(UI, "Keypad Auto Lock"), "Off")] },
            AnswerSpec {
                id: "on",
                label: "Yes, after 10 seconds",
                detail: "",
                presets: &[Preset(Setting(UI, "Keypad Auto Lock"), "On"), Preset(Setting(UI, "Keypad Auto Lock Delay Tim"), "10")],
            },
        ]),
    },
    QuestionSpec {
        id: "vox",
        recommended: Some("off"),
        title: "Hands-free talking (VOX)",
        intro: "Should the radio start transmitting by itself when you speak, without pressing the PTT key?",
        note: Some("Loud surroundings can start transmissions by accident. The sensitivity is in Settings (search for VOX)."),
        kind: Kind::Choice(&[
            AnswerSpec { id: "off", label: "No, I press PTT to talk", detail: "", presets: &[Preset(Setting(SERVICE, "VOX"), "Off")] },
            AnswerSpec {
                id: "external",
                label: "Yes, with a headset or external microphone",
                detail: "Only the external microphone starts a transmission.",
                presets: &[Preset(Setting(SERVICE, "VOX"), "On"), Preset(Setting(SERVICE, "VOX MIC Selection"), "Only External")],
            },
            AnswerSpec {
                id: "internal",
                label: "Yes, with the radio's own microphone",
                detail: "",
                presets: &[Preset(Setting(SERVICE, "VOX"), "On"), Preset(Setting(SERVICE, "VOX MIC Selection"), "Only Internal")],
            },
            // For radios where CPS does not offer the microphone choice (e.g. the BP515).
            AnswerSpec {
                id: "on",
                label: "Yes",
                detail: "Talk into the microphone and the radio transmits.",
                presets: &[Preset(Setting(SERVICE, "VOX"), "On")],
            },
        ]),
    },
    QuestionSpec {
        id: "battery",
        recommended: Some("on"),
        title: "Battery saving",
        intro: "Should the radio sleep part of the time while nobody is talking, to make the battery last longer?",
        note: Some(
            "Use the same battery saving setting on every radio you talk to: CPS warns that different settings can make calls between them fail.",
        ),
        kind: Kind::Choice(&[
            AnswerSpec {
                id: "on",
                label: "Yes (1-2, common)",
                detail: "Each transmission starts with a short wake-up signal (240 ms) so that sleeping radios hear it.",
                presets: &[Preset(Setting(CONFIG, "Power Save"), "Enable"), Preset(Setting(CONFIG, "Power Save Mode"), "1-2")],
            },
            AnswerSpec {
                id: "most",
                label: "Yes, as much as possible (1-4)",
                detail: "Longest battery life; transmissions start with a longer wake-up signal (480 ms).",
                presets: &[Preset(Setting(CONFIG, "Power Save"), "Enable"), Preset(Setting(CONFIG, "Power Save Mode"), "1-4")],
            },
            AnswerSpec {
                id: "off",
                label: "No",
                detail: "Always fully awake; uses more battery.",
                presets: &[Preset(Setting(CONFIG, "Power Save"), "Disable")],
            },
        ]),
    },
    QuestionSpec {
        id: "power",
        recommended: Some("low"),
        title: "Transmit power",
        intro: "Which transmit power should all channels use?",
        note: Some("Changes every digital and analog channel. Single channels can still be changed in the channel tables."),
        kind: Kind::Choice(&[
            AnswerSpec {
                id: "high",
                label: "High",
                detail: "Reaches further.",
                presets: &[Preset(Channels { digital: "Power Level", analog: "Power Level" }, "High")],
            },
            AnswerSpec {
                id: "low",
                label: "Low",
                detail: "Saves battery; enough when the other radios or the repeater are close.",
                presets: &[Preset(Channels { digital: "Power Level", analog: "Power Level" }, "Low")],
            },
        ]),
    },
    QuestionSpec {
        id: "timeout",
        recommended: Some("60"),
        title: "Transmit time limit",
        intro: "How long may one transmission last before the radio stops it? This keeps a stuck PTT key from blocking the channel.",
        note: Some("Changes every digital and analog channel."),
        kind: Kind::Choice(&[
            AnswerSpec { id: "30", label: "30 seconds", detail: "", presets: &[Preset(TIMEOUT, "30")] },
            AnswerSpec { id: "60", label: "60 seconds (common)", detail: "", presets: &[Preset(TIMEOUT, "60")] },
            AnswerSpec { id: "180", label: "3 minutes", detail: "", presets: &[Preset(TIMEOUT, "180")] },
        ]),
    },
    QuestionSpec {
        id: "channel",
        recommended: None,
        title: "Channels",
        intro: "Do you want to add a channel for a frequency you use, or replace all channels with a starter set? Skip this if your channels are already set up.",
        note: Some(
            "The other settings of new channels come from CPS's channel template; they can be changed in the channel tables afterwards.",
        ),
        kind: Kind::Channel,
    },
];

const TIMEOUT: Target = Channels { digital: "TX Timeout Time", analog: "TX Time Out Time" };

/// One channel of the starter set.
struct Starter {
    kind: ChannelKind,
    name: &'static str,
    /// MHz; transmit and receive (talking directly, no repeater).
    frequency: &'static str,
}

/// The starter channels that "replace all channels" writes, in zone order (hyt's choice, owner's
/// request 2026-11-19: some digital channels and a few PMR446 analog ones). Laid out like the
/// original PMR446 band plan: analog in 446.0–446.1 MHz (PMR 1–8), digital DMR in 446.1–446.2 MHz.
/// Every starter channel uses low power; digital ones colour code 1, slot 1; analog ones 12.5 kHz
/// and no tone. Everything else comes from CPS's channel template.
const STARTER: &[Starter] = &[
    Starter { kind: ChannelKind::Digital, name: "DMR 1", frequency: "446.10625" },
    Starter { kind: ChannelKind::Digital, name: "DMR 2", frequency: "446.11875" },
    Starter { kind: ChannelKind::Digital, name: "DMR 3", frequency: "446.13125" },
    Starter { kind: ChannelKind::Digital, name: "DMR 4", frequency: "446.14375" },
    Starter { kind: ChannelKind::Analog, name: "PMR 1", frequency: "446.00625" },
    Starter { kind: ChannelKind::Analog, name: "PMR 2", frequency: "446.01875" },
    Starter { kind: ChannelKind::Analog, name: "PMR 3", frequency: "446.03125" },
    Starter { kind: ChannelKind::Analog, name: "PMR 4", frequency: "446.04375" },
    Starter { kind: ChannelKind::Analog, name: "PMR 5", frequency: "446.05625" },
    Starter { kind: ChannelKind::Analog, name: "PMR 6", frequency: "446.06875" },
    Starter { kind: ChannelKind::Analog, name: "PMR 7", frequency: "446.08125" },
    Starter { kind: ChannelKind::Analog, name: "PMR 8", frequency: "446.09375" },
];

const STARTER_POWER: &str = "Low";
const STARTER_COLOR_CODE: &str = "1";
const STARTER_SLOT: &str = "Slot 1";
const STARTER_SPACING: &str = "12.5";

/// Field values of a starter channel (display text), in the order they are set.
fn starter_values(s: &Starter) -> Vec<(&'static str, &'static str)> {
    let mut v = vec![
        ("Channel Alias", s.name),
        ("RX Frequency", s.frequency),
        ("TX Frequency", s.frequency),
        ("Power Level", STARTER_POWER),
    ];
    match s.kind {
        ChannelKind::Digital => v.extend([("Color Code", STARTER_COLOR_CODE), ("Slot Operation", STARTER_SLOT)]),
        ChannelKind::Analog => v.push(("Channel Spacing", STARTER_SPACING)),
    }
    v
}

// ---------- what the GUI gets ----------

/// One question for the open radio.
#[derive(Debug, Clone, Serialize)]
pub struct Question {
    pub id: String,
    pub title: String,
    pub intro: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub kind: QuestionKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionKind {
    /// Pick one answer (or leave everything as it is).
    Choice {
        answers: Vec<Answer>,
        /// The answer the configuration matches now, if any.
        #[serde(skip_serializing_if = "Option::is_none")]
        current: Option<String>,
        /// The answer preselected when starting fresh, if offered.
        #[serde(skip_serializing_if = "Option::is_none")]
        recommended: Option<String>,
    },
    /// Type or choose a few values.
    Values { fields: Vec<ValueField> },
    /// Add a channel.
    Channel(ChannelForm),
}

#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// What the answer sets, for "show what this changes".
    pub settings: Vec<Preview>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    /// CPS's name of the setting (with "on every channel" for channel fields).
    pub setting: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<ParamHelp>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValueField {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hint: String,
    /// Editor, unit, CPS's name (`title`) and help.
    pub column: Column,
    /// The current value.
    pub cell: Cell,
}

/// What the add-a-channel form offers.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelForm {
    /// Channel kinds that can still be added.
    pub kinds: Vec<ChannelKind>,
    /// Allowed frequencies (from CPS's catalogue and the radio's model).
    pub frequency_hint: String,
    pub name_max_len: usize,
    pub color_code_hint: String,
    pub slots: Vec<String>,
    pub spacings: Vec<String>,
    /// CTCSS tones (Hz).
    pub tones: Vec<String>,
    /// The first zone's name, when there is one to add the channel to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// The starter set that can replace all channels (empty when it does not fit this radio).
    pub starter: Vec<StarterView>,
    /// Channels the configuration has now.
    pub existing_digital: usize,
    pub existing_analog: usize,
    /// Zones besides the first (their members keep pointing at channel numbers).
    pub other_zones: usize,
}

/// One starter channel, for the preview.
#[derive(Debug, Clone, Serialize)]
pub struct StarterView {
    pub kind: ChannelKind,
    pub name: String,
    /// MHz.
    pub frequency: String,
    /// e.g. "colour code 1, Slot 1, Low power".
    pub detail: String,
}

/// The answers to apply. Questions without a reply are left as they are.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Reply {
    pub question: String,
    /// For choice questions.
    #[serde(default)]
    pub answer: Option<String>,
    /// For value questions: field id → text.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    /// For the channel question.
    #[serde(default)]
    pub channel: Option<NewChannel>,
    /// For the channel question: replace all channels with the starter set (before `channel`).
    #[serde(default)]
    pub replace_channels: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewChannel {
    pub kind: ChannelKind,
    pub name: String,
    /// RX frequency in MHz.
    pub rx: String,
    /// TX frequency in MHz; empty for the same as RX (talking directly, no repeater).
    #[serde(default)]
    pub tx: String,
    /// Digital only.
    #[serde(default)]
    pub color_code: Option<String>,
    #[serde(default)]
    pub slot: Option<String>,
    /// Analog only.
    #[serde(default)]
    pub spacing: Option<String>,
    /// CTCSS tone sent when transmitting (analog), Hz; empty or absent for none.
    #[serde(default)]
    pub tone: Option<String>,
    /// Also only open the speaker for signals with that tone.
    #[serde(default)]
    pub tone_rx: bool,
    /// Add it to the first zone.
    #[serde(default)]
    pub add_to_zone: bool,
}

/// What applying the answers changed (or would change).
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    /// New entries of the change list.
    pub changes: Vec<ChangeView>,
    /// Things worth knowing (e.g. channels left alone).
    pub notes: Vec<String>,
}

// ---------- building the questions ----------

/// Settings the settings view shows (label, read-only reason), by (file, block, key).
type Shown = HashMap<(u16, u16, String), (String, Option<&'static str>)>;

/// The single-record field `key` of file `file`, when there is exactly one.
fn locate(ws: &Workspace, file: &str, key: &str) -> Option<FieldRef> {
    let f = ws.view().iter().find(|f| f.name == file)?;
    let mut found = f.blocks.iter().filter(|b| {
        crate::edit::def_for(&ws.layout, f.id, b.id, key).is_some_and(|d| d.block_type == 0)
            && b.records.len() == 1
            && b.records[0].fields.as_ref().is_some_and(|r| r.0.contains_key(key))
    });
    let block = found.next()?;
    if found.next().is_some() {
        return None;
    }
    Some(FieldRef { file: f.id, block: block.id, record: 0, member: None, key: key.into() })
}

/// A setting the wizard may change: shown by CPS for this radio and not read-only. Without CPS's
/// screen structure every located field is allowed (as in the record detail view).
fn usable(ws: &Workspace, shown: &Shown, file: &str, key: &str) -> Option<(FieldRef, String)> {
    let at = locate(ws, file, key)?;
    let def = ws.def(&at)?;
    // Fields CPS hides (licence-limited or internal) are not offered, like CPS.
    if !is_editable(def, key, value_at(ws.view(), &at)?) || ws.hidden.contains_key(&def.id) {
        return None;
    }
    if ws.ui.is_none() {
        let label = ws.options.as_ref().and_then(|o| o.param(def.id)).map_or(key.to_string(), |p| p.name.clone());
        return Some((at, label));
    }
    match shown.get(&(at.file, at.block, at.key.clone())) {
        Some((label, None)) => Some((at.clone(), label.clone())),
        _ => None,
    }
}

/// The raw value `text` means for the field at `at`.
fn parse(ws: &Workspace, at: &FieldRef, text: &str) -> Option<Value> {
    parse_input(ws.def(at)?, &at.key, ws.options.as_ref(), text).ok()
}

/// Channel field positions of every channel (view), with the list's file.
fn channel_fields(ws: &Workspace, digital: &str, analog: &str) -> Vec<FieldRef> {
    [(ChannelKind::Digital, digital), (ChannelKind::Analog, analog)]
        .into_iter()
        .filter_map(|(kind, key)| {
            let f = ws.view().iter().find(|f| f.name == kind.file())?;
            let b = f.blocks.first()?;
            Some((0..b.records.len()).map(|record| FieldRef { file: f.id, block: b.id, record, member: None, key: key.into() }))
        })
        .flatten()
        .collect()
}

/// A channel field that can be set now (not greyed out by a link rule, editable).
fn channel_settable(ws: &Workspace, at: &FieldRef) -> bool {
    ws.def(at).zip(value_at(ws.view(), at)).is_some_and(|(d, v)| is_editable(d, &at.key, v) && !ws.hidden.contains_key(&d.id))
        && ws.restriction(at).is_none()
}

/// The preset's value is allowed (for channels: on the first channel of each list, as the lists
/// share one catalogue entry per field).
fn preset_allowed(ws: &Workspace, shown: &Shown, p: &Preset) -> bool {
    match p.0 {
        Setting(file, key) => usable(ws, shown, file, key).is_some_and(|(at, _)| parse(ws, &at, p.1).is_some()),
        Channels { digital, analog } => {
            let fields = channel_fields(ws, digital, analog);
            !fields.is_empty()
                && fields.iter().all(|at| ws.def(at).is_some_and(|d| !ws.hidden.contains_key(&d.id) && parse(ws, at, p.1).is_some()))
        }
    }
}

/// The configuration has the preset's value now.
fn preset_current(ws: &Workspace, p: &Preset) -> bool {
    match p.0 {
        Setting(file, key) => locate(ws, file, key).is_some_and(|at| parse(ws, &at, p.1).as_ref() == value_at(ws.view(), &at)),
        Channels { digital, analog } => {
            let fields: Vec<FieldRef> = channel_fields(ws, digital, analog).into_iter().filter(|at| channel_settable(ws, at)).collect();
            !fields.is_empty() && fields.iter().all(|at| parse(ws, at, p.1).as_ref() == value_at(ws.view(), at))
        }
    }
}

fn preview(ws: &Workspace, shown: &Shown, p: &Preset) -> Preview {
    let (at, label, suffix) = match p.0 {
        Setting(file, key) => {
            let (at, label) = usable(ws, shown, file, key).unzip();
            (at, label.unwrap_or_else(|| key.to_string()), "")
        }
        Channels { digital, analog } => (channel_fields(ws, digital, analog).into_iter().next(), digital.to_string(), " (every channel)"),
    };
    let def = at.as_ref().and_then(|a| ws.def(a));
    let unit = def.and_then(|d| ws.options.as_ref()?.param(d.id)).map_or("", |p| p.unit.as_str());
    let value = if unit.is_empty() || p.1.ends_with(unit) || p.1.parse::<f64>().is_err() { p.1.to_string() } else { format!("{} {unit}", p.1) };
    Preview { setting: format!("{label}{suffix}"), value, help: def.and_then(|d| help_for(ws, d)) }
}

/// The questions that apply to the open radio.
pub fn questions(ws: &Workspace) -> Vec<Question> {
    let shown = crate::settings::shown(ws);
    QUESTIONS
        .iter()
        .filter_map(|q| {
            let kind = match &q.kind {
                Kind::Choice(answers) => {
                    let offered: Vec<&AnswerSpec> =
                        answers.iter().filter(|a| a.presets.iter().all(|p| preset_allowed(ws, &shown, p))).collect();
                    if offered.len() < 2 {
                        return None;
                    }
                    let current = offered.iter().find(|a| a.presets.iter().all(|p| preset_current(ws, p))).map(|a| a.id.to_string());
                    let recommended = q.recommended.filter(|r| offered.iter().any(|a| a.id == *r)).map(str::to_string);
                    QuestionKind::Choice {
                        recommended,
                        answers: offered
                            .iter()
                            .map(|a| Answer {
                                id: a.id.into(),
                                label: a.label.into(),
                                detail: a.detail.into(),
                                settings: a.presets.iter().map(|p| preview(ws, &shown, p)).collect(),
                            })
                            .collect(),
                        current,
                    }
                }
                Kind::Values(specs) => {
                    let fields: Vec<ValueField> = specs
                        .iter()
                        .filter_map(|s| {
                            let (at, label) = usable(ws, &shown, s.file, s.key)?;
                            let column = column(ws, at.file, at.block, &at.key, &label);
                            if column.editor == Editor::ReadOnly {
                                return None;
                            }
                            let cell = cell(ws, at, true);
                            Some(ValueField { id: s.key.into(), label: s.label.into(), hint: s.hint.into(), column, cell })
                        })
                        .collect();
                    if fields.is_empty() {
                        return None;
                    }
                    QuestionKind::Values { fields }
                }
                Kind::Channel => QuestionKind::Channel(channel_form(ws)?),
            };
            Some(Question {
                id: q.id.into(),
                title: q.title.into(),
                intro: q.intro.into(),
                note: q.note.map(str::to_string),
                kind,
            })
        })
        .collect()
}

fn channel_form(ws: &Workspace) -> Option<ChannelForm> {
    let room = |kind: ChannelKind| {
        let b = ws.view().iter().find(|f| f.name == kind.file())?.blocks.first()?;
        (b.records.len() < usize::from(b.capacity)).then_some(kind)
    };
    let kinds: Vec<ChannelKind> = [ChannelKind::Digital, ChannelKind::Analog].into_iter().filter_map(room).collect();
    if kinds.is_empty() && !starter_fits(ws) {
        return None;
    }
    let column_of = |kind: ChannelKind, key: &str| {
        let f = ws.view().iter().find(|f| f.name == kind.file())?;
        let b = f.blocks.first()?;
        Some(column(ws, f.id, b.id, key, key))
    };
    let choices = |kind: ChannelKind, key: &str| match column_of(kind, key).map(|c| c.editor) {
        Some(Editor::Choice { options }) => options,
        _ => Vec::new(),
    };
    let hint = |kind: ChannelKind, key: &str| match column_of(kind, key).map(|c| c.editor) {
        Some(Editor::Number { hint }) => hint,
        _ => String::new(),
    };
    let kind = kinds.first().copied().unwrap_or(ChannelKind::Digital);
    let name_max_len = match column_of(kind, "Channel Alias").map(|c| c.editor) {
        Some(Editor::Text { max_len }) => max_len,
        _ => 16,
    };
    let zones = crate::tables::table(ws, "zones");
    let zone = zones.as_ref().and_then(|t| t.rows.first().map(|r| r.cells[0].text.clone()));
    Some(ChannelForm {
        starter: if starter_fits(ws) { STARTER.iter().map(starter_view).collect() } else { Vec::new() },
        existing_digital: ws.channel_count(ChannelKind::Digital),
        existing_analog: ws.channel_count(ChannelKind::Analog),
        other_zones: zones.map_or(0, |t| t.rows.len().saturating_sub(1)),
        frequency_hint: hint(kind, "RX Frequency"),
        name_max_len,
        color_code_hint: hint(ChannelKind::Digital, "Color Code"),
        slots: choices(ChannelKind::Digital, "Slot Operation").into_iter().filter(|s| s.starts_with("Slot")).collect(),
        spacings: choices(ChannelKind::Analog, "Channel Spacing"),
        tones: choices(ChannelKind::Analog, "TX CTCorCDC Value").into_iter().filter(|t| t.parse::<f64>().is_ok()).collect(),
        zone,
        kinds,
    })
}

/// The channel list's file and block ids.
fn channel_list(ws: &Workspace, kind: ChannelKind) -> Option<(u16, u16, usize)> {
    let f = ws.view().iter().find(|f| f.name == kind.file())?;
    let b = f.blocks.first()?;
    Some((f.id, b.id, usize::from(b.capacity)))
}

/// The starter set fits this radio: both lists exist with room for it, the template exists, and
/// every value is allowed (e.g. the frequencies are in the model's band).
fn starter_fits(ws: &Workspace) -> bool {
    [ChannelKind::Digital, ChannelKind::Analog].into_iter().all(|kind| {
        let wanted: Vec<&Starter> = STARTER.iter().filter(|s| s.kind == kind).collect();
        let template = kind.file().replacen("QEU_C_", "QEU_C_Template_", 1);
        let Some((file, block, capacity)) = channel_list(ws, kind) else { return wanted.is_empty() };
        wanted.len() <= capacity
            && ws.view().iter().any(|f| f.name == template)
            && wanted.iter().all(|s| {
                starter_values(s).iter().all(|(key, text)| {
                    let at = FieldRef { file, block, record: 0, member: None, key: (*key).into() };
                    ws.def(&at).is_some_and(|d| parse_input(d, key, ws.options.as_ref(), text).is_ok())
                })
            })
    })
}

fn starter_view(s: &Starter) -> StarterView {
    let detail = match s.kind {
        ChannelKind::Digital => format!("colour code {STARTER_COLOR_CODE}, {STARTER_SLOT}, {STARTER_POWER} power"),
        ChannelKind::Analog => format!("{STARTER_SPACING} kHz, no tone, {STARTER_POWER} power"),
    };
    StarterView { kind: s.kind, name: s.name.into(), frequency: s.frequency.into(), detail }
}

/// Replace all channels with [`STARTER`]: existing channels are reset to CPS's template and
/// reused in place (so the current channel, scan lists and other references stay valid), extra
/// ones are removed from the end, missing ones added; the first zone then holds exactly the
/// starter channels. Returns a summary for the review.
fn replace_channels(ws: &mut Workspace) -> Result<String> {
    if !starter_fits(ws) {
        bail!("the starter channels do not fit this radio");
    }
    let mut parts = Vec::new();
    for kind in [ChannelKind::Digital, ChannelKind::Analog] {
        let wanted: Vec<&Starter> = STARTER.iter().filter(|s| s.kind == kind).collect();
        let existing = ws.channel_count(kind);
        let what = if kind == ChannelKind::Digital { "digital" } else { "analog" };
        // Remove the extra channels first, last one first, so the kept ones keep their numbers.
        for position in (wanted.len().max(1)..existing).rev() {
            ws.remove_channel(kind, position).with_context(|| format!("removing {what} channel {}", position + 1))?;
        }
        let (file, block, _) = channel_list(ws, kind).context("no channel list")?;
        for (i, s) in wanted.iter().enumerate() {
            let position = if i < existing {
                ws.reset_channel(kind, i)?;
                i
            } else {
                ws.add_channel(kind)?
            };
            for (key, text) in starter_values(s) {
                let at = FieldRef { file, block, record: position, member: None, key: key.into() };
                set(ws, &at, text).with_context(|| format!("{} {key}", s.name))?;
            }
        }
        if let (Some(first), Some(last)) = (wanted.first(), wanted.last()) {
            parts.push(format!(
                "{} {what} ({}–{}, {}–{} MHz)",
                wanted.len(),
                first.name,
                last.name,
                first.frequency,
                last.frequency
            ));
        }
    }
    let mut summary = format!("All channels replaced by {}.", parts.join(" and "));
    if crate::tables::table(ws, "zones").is_some_and(|t| !t.rows.is_empty()) {
        for member in (0..ws.zone_members(0).len()).rev() {
            ws.remove_zone_member(0, member)?;
        }
        let mut numbers: BTreeMap<bool, usize> = BTreeMap::new();
        for s in STARTER {
            let n = numbers.entry(s.kind == ChannelKind::Digital).or_default();
            *n += 1;
            ws.add_zone_member(0, s.kind, *n).with_context(|| format!("putting {} into the first zone", s.name))?;
        }
        summary.push_str(" The first zone now holds exactly these channels.");
        let others = crate::tables::table(ws, "zones").map_or(0, |t| t.rows.len().saturating_sub(1));
        if others > 0 {
            summary.push_str(&format!(
                " The other {others} zone{} still list channels by number: check them under Zones.",
                if others == 1 { "" } else { "s" }
            ));
        }
    }
    Ok(summary)
}

// ---------- applying ----------

/// Set `at` to `text` unless it already has that value. Returns whether it changed.
fn set(ws: &mut Workspace, at: &FieldRef, text: &str) -> Result<bool> {
    if parse(ws, at, text).is_some_and(|v| value_at(ws.view(), at) == Some(&v)) {
        return Ok(false);
    }
    ws.set_value(at, text)?;
    Ok(true)
}

/// Apply `replies`. All or nothing: on an error no edit is kept. With `dry_run` the edits are
/// undone afterwards, so the outcome is a preview.
pub fn apply(ws: &mut Workspace, replies: &[Reply], dry_run: bool) -> Result<Outcome> {
    let before: Vec<ChangeView> = change_list(ws);
    let checkpoint = ws.checkpoint();
    let mut notes = Vec::new();
    // Channels first, so that answers for "every channel" (power, time limit) also reach new ones.
    let mut ordered: Vec<&Reply> = replies.iter().collect();
    ordered.sort_by_key(|r| r.question != "channel");
    let result = ordered.into_iter().try_for_each(|r| {
        let q = QUESTIONS.iter().find(|q| q.id == r.question).ok_or_else(|| anyhow!("unknown question {:?}", r.question))?;
        apply_one(ws, q, r, &mut notes).with_context(|| q.title)
    });
    let outcome = result.map(|()| {
        let key = |c: &ChangeView| (c.place.clone(), c.setting.clone(), c.old.clone(), c.new.clone());
        let old: Vec<_> = before.iter().map(key).collect();
        Outcome { changes: change_list(ws).into_iter().filter(|c| !old.contains(&key(c))).collect(), notes }
    });
    if dry_run || outcome.is_err() {
        ws.restore(checkpoint);
    }
    outcome
}

fn apply_one(ws: &mut Workspace, q: &QuestionSpec, r: &Reply, notes: &mut Vec<String>) -> Result<()> {
    let shown = crate::settings::shown(ws);
    match &q.kind {
        Kind::Choice(answers) => {
            let Some(id) = &r.answer else { return Ok(()) };
            let a = answers.iter().find(|a| a.id == id).ok_or_else(|| anyhow!("unknown answer {id:?}"))?;
            for p in a.presets {
                match p.0 {
                    Setting(file, key) => {
                        let (at, _) = usable(ws, &shown, file, key).ok_or_else(|| anyhow!("{key} is not available on this radio"))?;
                        set(ws, &at, p.1)?;
                    }
                    Channels { digital, analog } => {
                        let mut skipped = 0;
                        for at in channel_fields(ws, digital, analog) {
                            if channel_settable(ws, &at) {
                                set(ws, &at, p.1)?;
                            } else {
                                skipped += 1;
                            }
                        }
                        if skipped > 0 {
                            notes.push(format!(
                                "{}: {skipped} channel{} left as {} (the setting is greyed out there, e.g. receive-only channels)",
                                q.title,
                                if skipped == 1 { "" } else { "s" },
                                if skipped == 1 { "it was" } else { "they were" }
                            ));
                        }
                    }
                }
            }
        }
        Kind::Values(specs) => {
            for (id, text) in &r.values {
                let s = specs.iter().find(|s| s.key == id).ok_or_else(|| anyhow!("unknown field {id:?}"))?;
                let (at, _) = usable(ws, &shown, s.file, s.key).ok_or_else(|| anyhow!("{} is not available on this radio", s.key))?;
                if crate::tables::cell(ws, at.clone(), true).text == text.trim() {
                    continue;
                }
                set(ws, &at, text).with_context(|| s.label)?;
            }
        }
        Kind::Channel => {
            if r.replace_channels {
                notes.push(replace_channels(ws)?);
            }
            if let Some(c) = &r.channel {
                notes.push(add_channel(ws, c)?);
            }
        }
    }
    Ok(())
}

/// Add the channel; returns a one-line summary of it for the review.
fn add_channel(ws: &mut Workspace, c: &NewChannel) -> Result<String> {
    let name = c.name.trim();
    if name.is_empty() {
        bail!("give the channel a name");
    }
    if c.rx.trim().is_empty() {
        bail!("give the channel's frequency");
    }
    let position = ws.add_channel(c.kind)?;
    let (file, block) = {
        let f = ws.view().iter().find(|f| f.name == c.kind.file()).context("no channel list")?;
        (f.id, f.blocks.first().context("no channel list")?.id)
    };
    let at = |key: &str| FieldRef { file, block, record: position, member: None, key: key.into() };
    let tx = if c.tx.trim().is_empty() { c.rx.as_str() } else { c.tx.as_str() };
    let mut steps: Vec<(&str, String)> = vec![
        ("Channel Alias", name.to_string()),
        ("RX Frequency", c.rx.trim().to_string()),
        ("TX Frequency", tx.trim().to_string()),
    ];
    let given = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    match c.kind {
        ChannelKind::Digital => {
            steps.extend(given(&c.color_code).map(|v| ("Color Code", v)));
            steps.extend(given(&c.slot).map(|v| ("Slot Operation", v)));
        }
        ChannelKind::Analog => {
            steps.extend(given(&c.spacing).map(|v| ("Channel Spacing", v)));
            if let Some(tone) = given(&c.tone) {
                steps.push(("TX CTCorCDC Type", "CTCSS".into()));
                steps.push(("TX CTCorCDC Value", tone.clone()));
                if c.tone_rx {
                    steps.push(("RX CTCorCDC Type", "CTCSS".into()));
                    steps.push(("RX CTCorCDC Value", tone));
                    steps.push(("RX Squelch Mode", "CTCSS/CDCSS".into()));
                }
            }
        }
    }
    for (key, text) in steps {
        set(ws, &at(key), &text).with_context(|| key.to_string())?;
    }
    if c.add_to_zone {
        ws.add_zone_member(0, c.kind, position + 1).context("adding the channel to the first zone")?;
    }
    let shown = |key: &str| cell(ws, at(key), true).text;
    let mut summary = format!(
        "New {} channel {} \"{}\": receives on {} MHz, transmits on {} MHz",
        if c.kind == ChannelKind::Digital { "digital" } else { "analog" },
        position + 1,
        shown("Channel Alias"),
        shown("RX Frequency"),
        shown("TX Frequency"),
    );
    match c.kind {
        ChannelKind::Digital => summary.push_str(&format!(", colour code {}, {}", shown("Color Code"), shown("Slot Operation"))),
        ChannelKind::Analog => {
            summary.push_str(&format!(", {} kHz spacing", shown("Channel Spacing")));
            if shown("TX CTCorCDC Type") != "None" {
                summary.push_str(&format!(", sends tone {} Hz", shown("TX CTCorCDC Value")));
            }
            if shown("RX CTCorCDC Type") != "None" {
                summary.push_str(&format!(", listens for tone {} Hz", shown("RX CTCorCDC Value")));
            }
        }
    }
    summary.push('.');
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_ids_are_unique() {
        let mut ids: Vec<&str> = QUESTIONS.iter().map(|q| q.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), QUESTIONS.len());
        for q in QUESTIONS {
            if let Kind::Choice(answers) = &q.kind {
                let mut a: Vec<&str> = answers.iter().map(|a| a.id).collect();
                a.sort_unstable();
                a.dedup();
                assert_eq!(a.len(), answers.len(), "{}", q.id);
            }
        }
    }
}
