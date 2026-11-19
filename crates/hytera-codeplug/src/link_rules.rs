//! CPS's link rules: a setting greyed out, hidden or forced to a value while other settings have
//! given values. Exported from CPS's parameter database (`LinkMain`, `LinkInitiativeParam`,
//! `LinkInitiativeCondition`, `LinkPassivityParam`, `LinkPassivityResult`, `LinkModelNumber`) by
//! `tools/export-rcdb-links.ps1` to `links.json` next to the option catalogue. See
//! `docs/file-format.md` ("Link rules").
//!
//! Meaning of the numbers (observed: the enums in CPS's readable `G2CPS.DBService.dll`,
//! `DBService.Common.EnumsDefine`): relation type Normal 0 / Complex 1 / UnionLink 2; logic
//! And 0 / Or 1 / Not 2; condition kind RangeLink 0 / PointLink 1 / OnOff 2; comparison Equal 0 /
//! UnEqual 1 / Greater 2 / EqualOrGreater 3 / Smaller 4 / LessOrEqual 5; result kind IsEnable 0 /
//! IsVisible 1 / RangeValue 2 / PointValue 3. Values are raw (RAM) values (observed: PointValue
//! 65535 for "None" channel references, range conditions `RAM:[3,99],…`).
//!
//! Only "Normal" rules made of point and range conditions with And/Or logic and IsEnable,
//! IsVisible and PointValue results are supported; others are kept with [`Rule::unsupported`] set.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logic {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compare {
    Equal,
    UnEqual,
    Greater,
    EqualOrGreater,
    Smaller,
    LessOrEqual,
}

impl Compare {
    fn from_code(c: i64) -> Option<Self> {
        Some(match c {
            0 => Compare::Equal,
            1 => Compare::UnEqual,
            2 => Compare::Greater,
            3 => Compare::EqualOrGreater,
            4 => Compare::Smaller,
            5 => Compare::LessOrEqual,
            _ => return None,
        })
    }

    fn holds(self, value: i64, other: i64) -> bool {
        match self {
            Compare::Equal => value == other,
            Compare::UnEqual => value != other,
            Compare::Greater => value > other,
            Compare::EqualOrGreater => value >= other,
            Compare::Smaller => value < other,
            Compare::LessOrEqual => value <= other,
        }
    }
}

/// One condition on a controlling parameter's raw value.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// `value <cmp> point`.
    Point { cmp: Compare, value: i64 },
    /// Value inside `min..=max` (`Equal`) or outside it (`UnEqual`).
    Range { inside: bool, min: i64, max: i64 },
}

impl Condition {
    pub fn holds(&self, value: i64) -> bool {
        match *self {
            Condition::Point { cmp, value: point } => cmp.holds(value, point),
            Condition::Range { inside, min, max } => (min..=max).contains(&value) == inside,
        }
    }
}

/// A controlling ("initiative") parameter and its conditions.
#[derive(Debug, Clone, PartialEq)]
pub struct Controlling {
    /// Layout id.
    pub param: u32,
    /// How the conditions of this parameter combine.
    pub logic: Logic,
    pub conditions: Vec<Condition>,
}

impl Controlling {
    pub fn holds(&self, value: i64) -> bool {
        match self.logic {
            Logic::And => self.conditions.iter().all(|c| c.holds(value)),
            Logic::Or => self.conditions.iter().any(|c| c.holds(value)),
        }
    }
}

/// What a rule does to a dependent parameter while its conditions hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// CPS's `IsEnable`: `false` greys the field out.
    Enabled(bool),
    /// CPS's `IsVisible`: `false` hides the field.
    Visible(bool),
    /// CPS's `PointValue`: the field is set to this raw value.
    Value(i64),
}

/// A dependent ("passivity") parameter and what the rule does to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Dependent {
    /// Layout id.
    pub param: u32,
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// `LinkMain.link_main_id`.
    pub id: i64,
    /// CPS's title (Chinese, Hytera data).
    pub title: String,
    /// How the controlling parameters combine.
    pub logic: Logic,
    pub controlling: Vec<Controlling>,
    pub dependent: Vec<Dependent>,
    /// Why hyt does not apply this rule, when it does not.
    pub unsupported: Option<String>,
}

impl Rule {
    /// Whether the conditions hold, given each controlling parameter's raw value (`None` if a
    /// value is unknown).
    pub fn holds(&self, value_of: impl Fn(u32) -> Option<i64>) -> Option<bool> {
        if self.unsupported.is_some() {
            return None;
        }
        let mut results = Vec::with_capacity(self.controlling.len());
        for c in &self.controlling {
            results.push(c.holds(value_of(c.param)?));
        }
        Some(match self.logic {
            Logic::And => results.iter().all(|&r| r),
            Logic::Or => results.iter().any(|&r| r),
        })
    }

    /// The effects on dependent parameter `param`.
    pub fn effects_on(&self, param: u32) -> &[Effect] {
        self.dependent.iter().find(|d| d.param == param).map_or(&[], |d| d.effects.as_slice())
    }
}

/// All link rules of one RCDB version.
#[derive(Debug, Clone, Default)]
pub struct LinkRules {
    pub rcdb_version: String,
    pub rules: Vec<Rule>,
}

/// (row id, kind, text, comparison or logic, position): a condition or a result row.
type RawItem = (i64, i64, String, i64, i64);

#[derive(Deserialize)]
struct RawLinks {
    rcdb_version: String,
    rules: Vec<(i64, i64, i64, i64, i64, i64, String)>,
    initiative: Vec<(i64, i64, i64, i64, i64)>,
    conditions: Vec<RawItem>,
    passivity: Vec<(i64, i64, i64, i64, i64)>,
    results: Vec<RawItem>,
    #[serde(default)]
    model_numbers: Vec<(i64, String, i64)>,
}

fn logic(code: i64) -> Option<Logic> {
    match code {
        0 => Some(Logic::And),
        1 => Some(Logic::Or),
        _ => None,
    }
}

/// Raw value from a condition/result text: a number, or `true`/`false`.
fn number(text: &str) -> Option<i64> {
    match text.trim() {
        "true" | "True" => Some(1),
        "false" | "False" => Some(0),
        t => t.parse().ok(),
    }
}

/// `RAM:[a,b],…` → (a, b).
fn ram_range(text: &str) -> Option<(i64, i64)> {
    match crate::options::parse_option(text)? {
        crate::options::ParsedOption::Scale { raw: (a, b), .. } => Some((a as i64, b as i64)),
        crate::options::ParsedOption::Choice { raw, .. } => Some((raw, raw)),
    }
}

impl LinkRules {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: RawLinks = serde_json::from_str(json.trim_start_matches('\u{FEFF}'))
            .map_err(|e| Error::Invalid(format!("links JSON: {e}")))?;
        let mut conditions: HashMap<i64, Vec<&RawItem>> = HashMap::new();
        for c in &raw.conditions {
            conditions.entry(c.0).or_default().push(c);
        }
        let mut results: HashMap<i64, Vec<&RawItem>> = HashMap::new();
        for r in &raw.results {
            results.entry(r.0).or_default().push(r);
        }
        let mut rules = Vec::with_capacity(raw.rules.len());
        for &(id, _link_id, _enable, relation, initiative_logic, _passivity_logic, ref title) in &raw.rules {
            let mut unsupported: Option<String> = None;
            let mut note = |why: String| {
                unsupported.get_or_insert(why);
            };
            if relation != 0 {
                note(format!("relation type {relation} (complex/union rule)"));
            }
            if raw.model_numbers.iter().any(|m| m.0 == id) {
                note("limited to some model numbers".into());
            }
            let rule_logic = logic(initiative_logic).unwrap_or_else(|| {
                note(format!("controlling logic {initiative_logic}"));
                Logic::And
            });
            let mut controlling = Vec::new();
            for &(_, ip_id, param, cond_logic, _) in raw.initiative.iter().filter(|i| i.0 == id) {
                let mut conds = Vec::new();
                for &&(_, kind, ref text, cmp, _) in conditions.get(&ip_id).map_or(&[][..], Vec::as_slice) {
                    let Some(cmp) = Compare::from_code(cmp) else {
                        note(format!("comparison {cmp}"));
                        continue;
                    };
                    let parsed = match kind {
                        1 | 2 => number(text).map(|value| Condition::Point { cmp, value }),
                        0 => match (ram_range(text), cmp) {
                            (Some((min, max)), Compare::Equal | Compare::UnEqual) => {
                                Some(Condition::Range { inside: cmp == Compare::Equal, min, max })
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    match parsed {
                        Some(c) => conds.push(c),
                        None => note(format!("condition kind {kind} {text:?}")),
                    }
                }
                if conds.is_empty() {
                    note("a controlling parameter without conditions".into());
                }
                let param_logic = logic(cond_logic).unwrap_or_else(|| {
                    note(format!("condition logic {cond_logic}"));
                    Logic::And
                });
                match u32::try_from(param) {
                    Ok(param) => controlling.push(Controlling { param, logic: param_logic, conditions: conds }),
                    Err(_) => note("a controlling parameter missing from the parameter table".into()),
                }
            }
            if controlling.is_empty() {
                note("no controlling parameter".into());
            }
            let mut dependent = Vec::new();
            for &(_, pp_id, param, _, _) in raw.passivity.iter().filter(|p| p.0 == id) {
                let mut effects = Vec::new();
                for &&(_, kind, ref text, self_logic, _) in results.get(&pp_id).map_or(&[][..], Vec::as_slice) {
                    let value = number(text);
                    let effect = match (kind, value, self_logic) {
                        (0, Some(v), 0) => Some(Effect::Enabled(v != 0)),
                        (1, Some(v), 0) => Some(Effect::Visible(v != 0)),
                        (3, Some(v), 0) => Some(Effect::Value(v)),
                        _ => None,
                    };
                    // An unknown result only affects that parameter; the rule's others still apply.
                    if let Some(e) = effect {
                        effects.push(e);
                    }
                }
                if let Ok(param) = u32::try_from(param)
                    && !effects.is_empty()
                {
                    dependent.push(Dependent { param, effects });
                }
            }
            rules.push(Rule { id, title: title.clone(), logic: rule_logic, controlling, dependent, unsupported });
        }
        Ok(LinkRules { rcdb_version: raw.rcdb_version, rules })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
        Self::parse(&text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
    }

    /// Where `tools/export-rcdb-links.ps1` writes the rules for `version` by default (next to
    /// the option catalogue).
    pub fn default_path(version: &str) -> Option<PathBuf> {
        Some(crate::options::data_dir()?.join(version).join("links.json"))
    }

    /// Rules hyt applies.
    pub fn supported(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter().filter(|r| r.unsupported.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like CPS's rule 1 (TX Admit "Always" or RX Only on: Private Call Confirmed greyed
    /// out and set to 0), rule 88 (a range condition) and a complex rule.
    const JSON: &str = r#"{"rcdb_version":"V1","source":"test",
        "rules":[[1,1,0,0,1,0,"r1"],[88,87,0,0,0,0,"r88"],[466,0,0,1,0,0,"complex"]],
        "initiative":[[1,1,9,0,1],[1,580,5160,0,2],[88,104,98,0,1],[466,5,1,0,1]],
        "conditions":[[1,1,"0",0,1],[580,1,"1",0,1],[104,0,"RAM:[3,99],UI:[15,495],Step:1",0,1],[104,1,"0",1,2]],
        "passivity":[[1,1,15,0,1],[88,242,99,0,1],[466,9,135,0,1]],
        "results":[[1,3,"0",0,1],[1,0,"false",0,2],[242,0,"true",0,1]],
        "model_numbers":[]}"#;

    #[test]
    fn parses_and_evaluates_rules() {
        let rules = LinkRules::parse(JSON).unwrap();
        assert_eq!(rules.rules.len(), 3);
        let r1 = &rules.rules[0];
        assert_eq!(r1.logic, Logic::Or);
        assert_eq!(r1.effects_on(15), &[Effect::Value(0), Effect::Enabled(false)]);
        let values = |admit: i64, rx_only: i64| move |p: u32| match p {
            9 => Some(admit),
            5160 => Some(rx_only),
            _ => None,
        };
        assert_eq!(r1.holds(values(0, 0)), Some(true));
        assert_eq!(r1.holds(values(2, 1)), Some(true));
        assert_eq!(r1.holds(values(2, 0)), Some(false));
        assert_eq!(r1.holds(|_| None), None);

        let r88 = &rules.rules[1];
        assert_eq!(r88.controlling[0].conditions[0], Condition::Range { inside: true, min: 3, max: 99 });
        assert_eq!(r88.holds(|_| Some(10)), Some(true));
        assert_eq!(r88.holds(|_| Some(0)), Some(false), "0 = Infinite is outside the range");

        assert!(rules.rules[2].unsupported.is_some(), "complex rules are not applied");
        assert_eq!(rules.supported().count(), 2);
    }

    #[test]
    fn loads_the_exported_rules_when_present() {
        let Some(path) = LinkRules::default_path("V1.5.24.000").filter(|p| p.is_file()) else {
            eprintln!("skipping: links.json not exported (tools/export-rcdb-links.ps1)");
            return;
        };
        let rules = LinkRules::load(&path).unwrap();
        // Observed for V1.5.24.000: 165 rules, the three zone/scan/roam "complex" ones unsupported.
        assert_eq!(rules.rules.len(), 165);
        let rx_only = rules.rules.iter().find(|r| r.id == 459).unwrap();
        assert!(rx_only.unsupported.is_none());
        assert_eq!(rx_only.controlling[0].param, 5160);
        assert!(rx_only.dependent.iter().all(|d| d.effects == [Effect::Enabled(false)]));
    }
}
