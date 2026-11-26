//! Display names, units and option values for parameters, exported from CPS's parameter database
//! by `tools/export-rcdb-options.ps1`. See `docs/file-format.md` section 2a.
//!
//! Each option is CPS's text, in one of two shapes:
//! - `RAM:<raw>,UI:<label>`: a named choice (e.g. `RAM:0,UI:High`);
//! - `RAM:[a,b],UI:[c,d],Step:s`: raw values a..b shown linearly as c..d (e.g. Hz shown as MHz).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::models::{ModelOverrides, ModelRules};
use crate::{Error, Result};

/// hyt's per-user folder for Hytera data: `%LOCALAPPDATA%\hyt\rcdb` on Windows,
/// `$XDG_DATA_HOME/hyt/rcdb` (or `~/.local/share/hyt/rcdb`) elsewhere.
pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))?;
    Some(base.join("hyt").join("rcdb"))
}

/// Which exported data version (options, ui, models, visibility, links) to use for a radio with
/// RCDB `version`: `version` itself when it was exported, else the newest version exported.
///
/// The exporters read CPS's parameter database, which holds a single (current) RCDB version
/// (`DBVersion.is_current`); CPS V1.5.24 reads radios of older versions (e.g. V1.5.08.000) with that
/// version's layout and this same database, keyed by parameter id (inferred). The layout still comes
/// from the radio's own version, so fields the radio does not have never show.
pub fn data_version(version: &str) -> String {
    match data_dir() {
        Some(dir) => data_version_in(&dir, version),
        None => version.to_string(),
    }
}

fn data_version_in(dir: &Path, version: &str) -> String {
    if dir.join(version).join("options.json").is_file() {
        return version.to_string();
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return version.to_string() };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("options.json").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .max_by_key(|v| version_key(v))
        .unwrap_or_else(|| version.to_string())
}

/// `V1.5.24.000` → `[1, 5, 24, 0]` for ordering versions (non-numbers count as 0).
fn version_key(v: &str) -> Vec<u32> {
    v.trim_start_matches(|c: char| c.is_ascii_alphabetic()).split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub raw: i64,
    pub label: String,
    /// CPS's `different_version`: 0 = always, others = model/firmware variants.
    pub variant: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scale {
    pub raw_min: f64,
    pub raw_max: f64,
    pub ui_min: f64,
    pub ui_max: f64,
    pub step: f64,
    pub variant: u32,
}

impl Scale {
    fn contains(&self, raw: f64) -> bool {
        raw >= self.raw_min.min(self.raw_max) && raw <= self.raw_min.max(self.raw_max)
    }

    fn contains_ui(&self, ui: f64) -> bool {
        let eps = 1e-9 * (1.0 + ui.abs());
        ui >= self.ui_min.min(self.ui_max) - eps && ui <= self.ui_min.max(self.ui_max) + eps
    }

    /// Raw value for display value `ui`, if it lies in this scale.
    pub fn to_raw(&self, ui: f64) -> Option<f64> {
        if !self.contains_ui(ui) {
            return None;
        }
        if self.ui_max == self.ui_min {
            return Some(self.raw_min);
        }
        Some(self.raw_min + (ui - self.ui_min) * (self.raw_max - self.raw_min) / (self.ui_max - self.ui_min))
    }

    /// Whether `raw` is in range and on the step grid (when the step is a whole number).
    pub fn accepts(&self, raw: i64) -> bool {
        if !self.contains(raw as f64) {
            return false;
        }
        let step = self.step;
        if step >= 1.0 && step.fract() == 0.0 && self.raw_min.fract() == 0.0 {
            let lo = self.raw_min.min(self.raw_max) as i64;
            return (raw - lo).rem_euclid(step as i64) == 0;
        }
        true
    }

    pub fn to_ui(&self, raw: f64) -> f64 {
        if self.raw_max == self.raw_min {
            return self.ui_min;
        }
        self.ui_min + (raw - self.raw_min) * (self.ui_max - self.ui_min) / (self.raw_max - self.raw_min)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ParamInfo {
    pub name: String,
    pub unit: String,
    pub choices: Vec<Choice>,
    pub scales: Vec<Scale>,
}

impl ParamInfo {
    /// Add one CPS option text.
    fn push(&mut self, text: &str, variant: u32) {
        match parse_option(text) {
            Some(ParsedOption::Choice { raw, label }) => self.choices.push(Choice { raw, label, variant }),
            Some(ParsedOption::Scale { raw, ui, step }) => self.scales.push(Scale {
                raw_min: raw.0,
                raw_max: raw.1,
                ui_min: ui.0,
                ui_max: ui.1,
                step,
                variant,
            }),
            // All 3,552 options of V1.5.24.000 parse; unknown future shapes are skipped.
            None => {}
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub rcdb_version: String,
    params: HashMap<u32, ParamInfo>,
    /// Generic entries of parameters that [`Options::with_model`] narrowed, kept so values outside
    /// the model's range (e.g. read from the radio) are still shown with CPS's label.
    generic: HashMap<u32, ParamInfo>,
    model: Option<ModelOverrides>,
}

#[derive(Deserialize)]
struct RawOption {
    variant: u32,
    range: String,
}

#[derive(Deserialize)]
struct RawParam {
    id: u32,
    name: String,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    options: Vec<RawOption>,
}

#[derive(Deserialize)]
struct RawOptions {
    rcdb_version: String,
    params: Vec<RawParam>,
}

/// One parsed option string.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedOption {
    Choice { raw: i64, label: String },
    Scale { raw: (f64, f64), ui: (f64, f64), step: f64 },
}

fn pair(s: &str) -> Option<(f64, f64)> {
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    let (a, b) = inner.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Parse CPS's option text; `None` for shapes not seen so far.
pub fn parse_option(s: &str) -> Option<ParsedOption> {
    let rest = s.strip_prefix("RAM:")?;
    let (raw, ui) = rest.split_once(",UI:")?;
    if raw.starts_with('[') {
        let (ui_range, step) = ui.split_once(",Step:")?;
        Some(ParsedOption::Scale { raw: pair(raw)?, ui: pair(ui_range)?, step: step.trim().parse().ok()? })
    } else {
        Some(ParsedOption::Choice { raw: raw.trim().parse().ok()?, label: ui.to_string() })
    }
}

impl Options {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: RawOptions = serde_json::from_str(json.trim_start_matches('\u{FEFF}'))
            .map_err(|e| Error::Invalid(format!("options JSON: {e}")))?;
        let mut params = HashMap::new();
        for p in raw.params {
            let mut info = ParamInfo { name: p.name, unit: p.unit, ..Default::default() };
            for o in p.options {
                info.push(&o.range, o.variant);
            }
            params.insert(p.id, info);
        }
        Ok(Options { rcdb_version: raw.rcdb_version, params, ..Default::default() })
    }

    /// These options narrowed to `model_number` by `rules` (see [`Options::with_model`]).
    pub fn for_model(&self, rules: &ModelRules, model_number: &str) -> Options {
        self.with_model(rules.for_model(model_number))
    }

    /// These options with the model's ranges replacing the generic choices and scales of each
    /// parameter they name (CPS shows and accepts only those for that model; inferred from the
    /// `ModelNumberRangeGroupLink` rows, which repeat the generic entries minus the excluded ones).
    /// Parameters without a model range keep their generic options. Defaults and visibility are
    /// kept for [`Options::model_default`] / [`Options::model_visible`].
    pub fn with_model(&self, model: ModelOverrides) -> Options {
        let mut out = self.clone();
        for (id, texts) in &model.ranges {
            let Some(generic) = self.params.get(id) else { continue };
            let mut info = ParamInfo { name: generic.name.clone(), unit: generic.unit.clone(), ..Default::default() };
            for text in texts {
                info.push(text, 0);
            }
            if info.choices.is_empty() && info.scales.is_empty() {
                continue;
            }
            out.generic.entry(*id).or_insert_with(|| generic.clone());
            out.params.insert(*id, info);
        }
        out.model = Some(model);
        out
    }

    /// The model rules applied by [`Options::with_model`], if any.
    pub fn model(&self) -> Option<&ModelOverrides> {
        self.model.as_ref()
    }

    /// The model's default value text for parameter `id` (raw number or CPS text), if it has one.
    pub fn model_default(&self, id: u32) -> Option<&str> {
        self.model.as_ref()?.defaults.get(&id).map(String::as_str)
    }

    /// Whether CPS shows parameter `id` for this model, when a model rule says so.
    pub fn model_visible(&self, id: u32) -> Option<bool> {
        self.model.as_ref()?.visible.get(&id).copied()
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
        Self::parse(&text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
    }

    /// Where the exporter writes the catalogue for `version` by default:
    /// `%LOCALAPPDATA%\hyt\rcdb\<version>\options.json` on Windows,
    /// `$XDG_DATA_HOME/hyt/rcdb/<version>/options.json` (or `~/.local/share/...`) elsewhere.
    pub fn default_path(version: &str) -> Option<PathBuf> {
        Some(data_dir()?.join(version).join("options.json"))
    }

    pub fn param(&self, id: u32) -> Option<&ParamInfo> {
        self.params.get(&id)
    }

    /// How CPS would show raw value `raw` of parameter `id`: the choice's label, or the scaled
    /// number with its unit. `None` when nothing is known.
    pub fn display(&self, id: u32, raw: i64) -> Option<String> {
        let p = self.params.get(&id)?;
        Self::display_in(p, raw).or_else(|| Self::display_in(self.generic.get(&id)?, raw))
    }

    fn display_in(p: &ParamInfo, raw: i64) -> Option<String> {
        if let Some(c) = p.choices.iter().filter(|c| c.raw == raw).min_by_key(|c| c.variant) {
            return Some(c.label.clone());
        }
        let s = p
            .scales
            .iter()
            .filter(|s| s.contains(raw as f64))
            .min_by_key(|s| s.variant)?;
        let ui = s.to_ui(raw as f64);
        let mut text = format_number(ui);
        if !p.unit.is_empty() {
            text.push(' ');
            text.push_str(&p.unit);
        }
        Some(text)
    }
}

impl Options {
    /// Raw value for display text of parameter `id`: a choice label (exact match), or a number
    /// (optionally followed by the unit) inside one of the parameter's scales.
    pub fn parse_display(&self, id: u32, text: &str) -> Option<i64> {
        let p = self.params.get(&id)?;
        let text = text.trim();
        let mut choices: Vec<&Choice> = p.choices.iter().filter(|c| c.label == text).collect();
        choices.sort_by_key(|c| c.variant);
        if let Some(c) = choices.first() {
            return Some(c.raw);
        }
        let number = if p.unit.is_empty() { text } else { text.strip_suffix(p.unit.as_str()).unwrap_or(text).trim() };
        let ui: f64 = number.parse().ok()?;
        let mut scales: Vec<&Scale> = p.scales.iter().collect();
        scales.sort_by_key(|s| s.variant);
        scales.into_iter().find_map(|s| s.to_raw(ui)).map(|raw| raw.round() as i64)
    }

    /// Display text for `raw` that [`Options::parse_display`] turns back into exactly `raw`;
    /// `None` when no such text exists (then the raw number must be used).
    pub fn label_for_edit(&self, id: u32, raw: i64) -> Option<String> {
        let text = self.display(id, raw)?;
        (self.parse_display(id, &text) == Some(raw)).then_some(text)
    }

    /// Whether `raw` is one of the parameter's choices or inside one of its scales. `None` when
    /// the catalogue says nothing about allowed values for this parameter.
    pub fn allows(&self, id: u32, raw: i64) -> Option<bool> {
        let p = self.params.get(&id)?;
        if p.choices.is_empty() && p.scales.is_empty() {
            return None;
        }
        Some(p.choices.iter().any(|c| c.raw == raw) || p.scales.iter().any(|s| s.accepts(raw)))
    }

    /// Short description of the allowed values, for error messages.
    pub fn describe_allowed(&self, id: u32) -> String {
        let Some(p) = self.params.get(&id) else { return "unknown".into() };
        let mut parts: Vec<String> = p.choices.iter().map(|c| format!("{:?}", c.label)).collect();
        parts.extend(p.scales.iter().map(|s| {
            format!("{}..{}{}", format_number(s.ui_min), format_number(s.ui_max), if p.unit.is_empty() { String::new() } else { format!(" {}", p.unit) })
        }));
        parts.dedup();
        parts.join(", ")
    }
}

fn format_number(x: f64) -> String {
    if (x - x.round()).abs() < 1e-9 {
        format!("{}", x.round() as i64)
    } else {
        let s = format!("{x:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_version_falls_back_to_newest_export() {
        let dir = std::env::temp_dir().join(format!("hyt-data-version-{}", std::process::id()));
        for v in ["V1.5.9.000", "V1.5.24.000", "V1.5.10.000", "notes"] {
            std::fs::create_dir_all(dir.join(v)).unwrap();
        }
        for v in ["V1.5.9.000", "V1.5.24.000", "V1.5.10.000"] {
            std::fs::write(dir.join(v).join("options.json"), "{}").unwrap();
        }
        assert_eq!(data_version_in(&dir, "V1.5.10.000"), "V1.5.10.000");
        assert_eq!(data_version_in(&dir, "V1.5.08.000"), "V1.5.24.000");
        assert_eq!(data_version_in(&dir.join("missing"), "V1.5.08.000"), "V1.5.08.000");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    const SAMPLE: &str = r#"{"rcdb_version":"V1.5.24.000","source":"test","params":[
        {"id":4,"name":"Power Level","unit":"","options":[
            {"variant":0,"index":0,"range":"RAM:0,UI:High"},{"variant":0,"index":1,"range":"RAM:2,UI:Low"}]},
        {"id":7,"name":"RX Frequency","unit":"MHz","options":[
            {"variant":0,"index":0,"range":"RAM:[350000000,527000000],UI:[350.000000,527.000000],Step:5"}]},
        {"id":6064,"name":"High Power RSSI Threshold","unit":"dBm","options":[
            {"variant":0,"index":0,"range":"RAM:[75,115],UI:[-75,-115],Step:1"}]},
        {"id":136,"name":"Channel Type","unit":"","options":[
            {"variant":0,"index":0,"range":"RAM:0,UI:Digital"},{"variant":1,"index":3,"range":"RAM:255,UI:None"}]}
    ]}"#;

    #[test]
    fn labels_and_scales_values() {
        let o = Options::parse(SAMPLE).unwrap();
        assert_eq!(o.display(4, 2).as_deref(), Some("Low"));
        assert_eq!(o.display(7, 444_725_000).as_deref(), Some("444.725 MHz"));
        assert_eq!(o.display(6064, 80).as_deref(), Some("-80 dBm"));
        assert_eq!(o.display(136, 255).as_deref(), Some("None"));
        assert_eq!(o.display(4, 1), None);
        assert_eq!(o.display(999, 1), None);
    }

    #[test]
    fn display_text_converts_back_to_raw() {
        let o = Options::parse(SAMPLE).unwrap();
        assert_eq!(o.parse_display(4, "Low"), Some(2));
        assert_eq!(o.parse_display(7, "444.725 MHz"), Some(444_725_000));
        assert_eq!(o.parse_display(7, "446.13125"), Some(446_131_250));
        assert_eq!(o.parse_display(6064, "-80 dBm"), Some(80));
        assert_eq!(o.parse_display(7, "600 MHz"), None, "outside the range");
        assert_eq!(o.label_for_edit(7, 444_725_000).as_deref(), Some("444.725 MHz"));
        assert_eq!(o.allows(7, 444_725_000), Some(true));
        assert_eq!(o.allows(7, 444_725_001), Some(false), "off the 5 Hz step grid");
        assert_eq!(o.allows(4, 1), Some(false));
        assert_eq!(o.allows(999, 1), None);
    }

    #[test]
    fn model_ranges_narrow_validation_but_not_display() {
        let generic = Options::parse(SAMPLE).unwrap();
        let mut model = ModelOverrides { model_number: "55P10B00-…".into(), ..Default::default() };
        model.ranges.insert(7, vec!["RAM:[400000000,470000000],UI:[400.000000,470.000000],Step:5".into()]);
        model.ranges.insert(4, vec!["RAM:2,UI:Low".into()]);
        model.ranges.insert(999, vec!["RAM:1,UI:Unknown parameter".into()]);
        model.defaults.insert(7, "429075000".into());
        model.visible.insert(136, false);
        let o = generic.with_model(model);

        assert_eq!(o.allows(7, 444_725_000), Some(true));
        assert_eq!(o.allows(7, 380_000_000), Some(false), "below the U1 band");
        assert_eq!(o.allows(7, 500_000_000), Some(false), "above the U1 band");
        assert_eq!(generic.allows(7, 500_000_000), Some(true), "the generic range is wider");
        assert_eq!(o.parse_display(7, "480 MHz"), None);
        assert_eq!(o.parse_display(7, "444.725"), Some(444_725_000));
        assert_eq!(o.describe_allowed(7), "400..470 MHz");
        assert_eq!(o.allows(4, 0), Some(false), "High is not offered for this model");
        assert_eq!(o.display(4, 0).as_deref(), Some("High"), "still shown with CPS's label");
        assert_eq!(o.label_for_edit(4, 0), None, "but not offered as an edit label");
        assert_eq!(o.display(7, 500_000_000).as_deref(), Some("500 MHz"));
        assert_eq!(o.allows(6064, 80), Some(true), "parameters without a model range are unchanged");
        assert_eq!(o.param(999).map(|p| p.name.as_str()), None, "unknown parameters are not invented");
        assert_eq!(o.model_default(7), Some("429075000"));
        assert_eq!(o.model_visible(136), Some(false));
        assert_eq!(generic.model(), None);
        assert!(o.model().is_some());
    }

    #[test]
    fn parses_option_text() {
        assert_eq!(
            parse_option("RAM:1072,UI:107.2"),
            Some(ParsedOption::Choice { raw: 1072, label: "107.2".into() })
        );
        assert_eq!(
            parse_option("RAM:[3,99],UI:[15,495],Step:1"),
            Some(ParsedOption::Scale { raw: (3.0, 99.0), ui: (15.0, 495.0), step: 1.0 })
        );
        assert_eq!(parse_option("RAM:00,UI:None"), Some(ParsedOption::Choice { raw: 0, label: "None".into() }));
        assert_eq!(parse_option("nonsense"), None);
    }
}
