//! Model-number rules: how CPS narrows parameter ranges, defaults and visibility by the radio's
//! model number (info item 0x0000, e.g. `55P10B00-1L0BAPU1U1…`). Exported from CPS's parameter
//! database (`ModelNumber*` tables) by `tools/export-rcdb-options.ps1` into `models.json`, next to
//! `options.json`. See `docs/file-format.md` section 2b.
//!
//! A rule *group* matches a model number when all its conditions hold. A condition names a
//! 1-based character position and length in the model number **without its `-` separators** and a
//! comma-separated list of allowed values (observed: CPS's `XML Model Number Rule.xml` numbers the
//! 80 characters that way, and e.g. position 17, length 2 is the "flash burned frequency band",
//! `U1` = 400–470 MHz). Ranges, defaults and visibility rows belong to a group and a parameter.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

/// One condition of a rule group: characters `start..start+len` (1-based) are one of `values`.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub start: usize,
    pub len: usize,
    pub values: Vec<String>,
}

impl Condition {
    /// Whether `model` (already [`normalize`]d) satisfies this condition.
    pub fn matches(&self, model: &[char]) -> bool {
        let Some(from) = self.start.checked_sub(1) else { return false };
        let Some(part) = model.get(from..from + self.len) else { return false };
        let part: String = part.iter().collect();
        self.values.contains(&part)
    }
}

/// The model number as CPS's rules index it: without `-` separators and surrounding spaces.
pub fn normalize(model_number: &str) -> Vec<char> {
    model_number.trim().chars().filter(|c| *c != '-').collect()
}

/// All model-number rules of one RCDB version.
#[derive(Debug, Clone, Default)]
pub struct ModelRules {
    pub rcdb_version: String,
    /// Conditions by group id.
    groups: BTreeMap<u32, Vec<Condition>>,
    /// CPS option texts (`RAM:…,UI:…`) by (group id, layout parameter id), in CPS's order.
    ranges: BTreeMap<(u32, u32), Vec<String>>,
    defaults: BTreeMap<(u32, u32), String>,
    visible: BTreeMap<(u32, u32), bool>,
    enable: BTreeMap<(u32, u32), bool>,
}

/// What the rules say for one model number. Each map is keyed by layout parameter id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelOverrides {
    /// The model number as given (with separators).
    pub model_number: String,
    /// Allowed values (CPS option texts) replacing the parameter's generic options.
    pub ranges: BTreeMap<u32, Vec<String>>,
    /// Default value text (raw number, e.g. `429075000`, or text such as `400-470`).
    pub defaults: BTreeMap<u32, String>,
    /// Whether CPS (external build) shows the parameter for this model; replaces `Param.visible`.
    pub visible: BTreeMap<u32, bool>,
    /// Whether the parameter is editable for this model (`is_enable` != 0; meaning not confirmed).
    pub enable: BTreeMap<u32, bool>,
}

#[derive(Deserialize)]
struct RawModels {
    rcdb_version: String,
    #[serde(default)]
    conditions: Vec<(u32, usize, usize, String, u32)>,
    #[serde(default)]
    ranges: Vec<(u32, u32, i64, String)>,
    #[serde(default)]
    defaults: Vec<(u32, u32, String)>,
    #[serde(default)]
    visible: Vec<(u32, u32, i64)>,
    #[serde(default)]
    enable: Vec<(u32, u32, i64)>,
}

impl ModelRules {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: RawModels = serde_json::from_str(json.trim_start_matches('\u{FEFF}'))
            .map_err(|e| Error::Invalid(format!("model rules JSON: {e}")))?;
        let mut rules = ModelRules { rcdb_version: raw.rcdb_version, ..Default::default() };
        for (group, start, len, values, _param) in raw.conditions {
            let values = values.split(',').map(str::trim).filter(|v| !v.is_empty()).map(String::from).collect();
            rules.groups.entry(group).or_default().push(Condition { start, len, values });
        }
        let mut ranges: BTreeMap<(u32, u32), Vec<(i64, String)>> = BTreeMap::new();
        for (group, param, index, text) in raw.ranges {
            ranges.entry((group, param)).or_default().push((index, text));
        }
        rules.ranges = ranges
            .into_iter()
            .map(|(k, mut v)| {
                v.sort_by_key(|(i, _)| *i);
                (k, v.into_iter().map(|(_, t)| t).collect())
            })
            .collect();
        rules.defaults = raw.defaults.into_iter().map(|(g, p, d)| ((g, p), d)).collect();
        // `is_visible` is CPS's ParamVisible value, which replaces `Param.visible` (0 = shown): see
        // [`crate::param_visibility::shown_by_external_build`]. Observed 2026-11-19: the BP515's rows set
        // 0 for Pure Digital/Pure Analog/Roam/Two Tone, the four flags CPS's Feature Control shows.
        rules.visible =
            raw.visible.into_iter().map(|(g, p, v)| ((g, p), crate::param_visibility::shown_by_external_build(v))).collect();
        rules.enable = raw.enable.into_iter().map(|(g, p, v)| ((g, p), v != 0)).collect();
        Ok(rules)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
        Self::parse(&text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
    }

    /// Where the exporter writes the rules for `version`: next to `options.json`.
    pub fn default_path(version: &str) -> Option<PathBuf> {
        Some(crate::options::Options::default_path(version)?.with_file_name("models.json"))
    }

    /// Ids of the rule groups whose conditions all hold for `model_number`.
    pub fn matching_groups(&self, model_number: &str) -> BTreeSet<u32> {
        let model = normalize(model_number);
        self.groups
            .iter()
            .filter(|(_, conds)| !conds.is_empty() && conds.iter().all(|c| c.matches(&model)))
            .map(|(g, _)| *g)
            .collect()
    }

    /// The ranges, defaults and visibility that apply to `model_number`. When several matching
    /// groups give a value for the same parameter, the lowest group id wins (CPS's rule for that is
    /// unknown; no such overlap exists for the models in CPS's `ModelNumber.csv` except two
    /// visibility rows of the AP320).
    pub fn for_model(&self, model_number: &str) -> ModelOverrides {
        let groups = self.matching_groups(model_number);
        fn pick<T: Clone>(map: &BTreeMap<(u32, u32), T>, groups: &BTreeSet<u32>) -> BTreeMap<u32, T> {
            let mut out = BTreeMap::new();
            for ((g, p), v) in map {
                if groups.contains(g) {
                    out.entry(*p).or_insert_with(|| v.clone());
                }
            }
            out
        }
        ModelOverrides {
            model_number: model_number.trim().to_string(),
            ranges: pick(&self.ranges, &groups),
            defaults: pick(&self.defaults, &groups),
            visible: pick(&self.visible, &groups),
            enable: pick(&self.enable, &groups),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "55P10B00-1L0BAPU1U100000000CB000000000000-00000NN0000000000000000000000000-00040000";

    const SAMPLE: &str = r#"{"rcdb_version":"V1.5.24.000","source":"test",
        "conditions":[[113,17,2,"U1,",7],[114,17,2,"U3,",7],[116,17,2,"Ux,",7],
                      [42,14,1,"P,",38],[38,20,1,"A,B,",929],[38,1,1,"7,",929],[500,1,1,"5,",7]],
        "ranges":[[113,7,0,"RAM:[400000000,470000000],UI:[400.000000,470.000000],Step:5"],
                  [114,7,0,"RAM:[350000000,400000000],UI:[350.000000,400.000000],Step:5"],
                  [42,38,2,"RAM:2,UI:Priority Interrupt"],[42,38,0,"RAM:1,UI:Follow TX Admit"],
                  [42,38,1,"RAM:3,UI:Color Code Free"],
                  [500,7,0,"RAM:[136000000,174000000],UI:[136.000000,174.000000],Step:5"]],
        "defaults":[[113,7,"429075000"],[114,7,"353075000"]],
        "visible":[[38,929,0],[42,38,1]],
        "enable":[]}"#;

    #[test]
    fn positions_count_without_separators() {
        let m = normalize(MODEL);
        assert_eq!(m.len(), 80);
        let c = |start, len, v: &str| Condition { start, len, values: vec![v.to_string()] };
        assert!(c(1, 1, "5").matches(&m), "level");
        assert!(c(3, 1, "P").matches(&m), "portable");
        assert!(c(14, 1, "P").matches(&m), "protocol standard");
        assert!(c(15, 2, "U1").matches(&m), "hardware band");
        assert!(c(17, 2, "U1").matches(&m), "flash burned band");
        assert!(!c(17, 2, "U3").matches(&m));
        assert!(!c(80, 2, "00").matches(&m), "past the end");
        assert!(!c(0, 1, "5").matches(&m), "positions are 1-based");
    }

    #[test]
    fn picks_the_groups_of_the_model() {
        let r = ModelRules::parse(SAMPLE).unwrap();
        assert_eq!(r.matching_groups(MODEL), BTreeSet::from([42, 113, 500]));
        let o = r.for_model(MODEL);
        assert_eq!(o.ranges[&7], vec!["RAM:[400000000,470000000],UI:[400.000000,470.000000],Step:5".to_string()], "lowest group wins");
        assert_eq!(
            o.ranges[&38],
            ["RAM:1,UI:Follow TX Admit", "RAM:3,UI:Color Code Free", "RAM:2,UI:Priority Interrupt"],
            "in CPS's index order"
        );
        assert_eq!(o.defaults.get(&7).map(String::as_str), Some("429075000"));
        assert_eq!(o.visible.get(&38), Some(&false), "is_visible 1 = UnVisible");
        assert_eq!(o.visible.get(&929), None, "group 38 needs level 7");
        let other = r.for_model("58P10B00-1L0DAPU3U300000000CB000000000000-00000NN0000000000000000000000000-00040000");
        assert_eq!(other.ranges[&7][0], "RAM:[350000000,400000000],UI:[350.000000,400.000000],Step:5");
        assert!(r.for_model("").ranges.is_empty());
    }
}
