//! CPS's link rules applied to an opened configuration: which fields CPS greys out or hides
//! because of another field's value, and which values CPS sets when a controlling field changes.
//! The rules come from `links.json` ([`hytera_codeplug::link_rules`]); without it nothing here
//! changes the app's behaviour.
//!
//! A rule is applied to a field only when every controlling field can be found for that field's
//! record: in the same record (e.g. a channel's `RX Only` for its TX settings) or in a single-record
//! block (a radio-wide switch). Rules whose controlling field is in another list are skipped.
//! When several rules apply, the most specific ones decide (see [`Links::restriction`]; inferred).
//! Forced values are applied only to the same record or a
//! single-record block, and only when the catalogue allows the value.

use std::collections::HashMap;

use hytera_codeplug::decode::{DecodedFile, keys};
use hytera_codeplug::layout::Layout;
use hytera_codeplug::link_rules::{Controlling, Effect, LinkRules, Logic};
use hytera_codeplug::options::Options;

use crate::workspace::{FieldRef, value_at};

/// A field of the layout, by where it is decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Loc {
    param: u32,
    file: u16,
    block: u16,
    key: String,
    /// In a single-record block (record 0).
    single: bool,
}

#[derive(Debug, Clone)]
struct Resolved {
    id: i64,
    logic: Logic,
    controlling: Vec<(Loc, Controlling)>,
    targets: Vec<(Loc, Vec<Effect>)>,
}

type Key = (u16, u16, String);

/// The rules that apply to a configuration's files.
#[derive(Debug, Clone, Default)]
pub struct Links {
    rules: Vec<Resolved>,
    /// Field → (rule, target) pairs that can disable or set it.
    by_target: HashMap<Key, Vec<(usize, usize)>>,
    /// Field → rules it controls.
    by_control: HashMap<Key, Vec<usize>>,
}

fn locate(layout: &Layout, files: &[DecodedFile], param: u32) -> Vec<Loc> {
    layout
        .find(param)
        .into_iter()
        .filter(|d| d.block_type != 2 && files.iter().any(|f| f.id == d.file_id))
        .filter_map(|d| {
            let defs = layout.fields(d.file_id, d.block_id);
            let key = keys(defs).into_iter().zip(defs).find(|(_, x)| x.id == d.id && x.offset == d.offset)?.0;
            Some(Loc { param, file: d.file_id, block: d.block_id, key, single: d.block_type == 0 })
        })
        .collect()
}

impl Loc {
    fn key(&self) -> Key {
        (self.file, self.block, self.key.clone())
    }

    /// Where this field is for the record at `at` (same record, or record 0 of a single block);
    /// `None` if it is in another list.
    fn for_record(&self, at: &FieldRef) -> Option<FieldRef> {
        let record = if self.single {
            0
        } else if (self.file, self.block) == (at.file, at.block) {
            at.record
        } else {
            return None;
        };
        Some(FieldRef { file: self.file, block: self.block, record, member: None, key: self.key.clone() })
    }
}

impl Resolved {
    /// Whether the rule holds for the record of `at`; `None` if a controlling value is missing.
    /// `value` gives a field's raw value.
    fn holds(&self, at: &FieldRef, value: &dyn Fn(&FieldRef) -> Option<i64>) -> Option<bool> {
        let mut results = Vec::with_capacity(self.controlling.len());
        for (loc, c) in &self.controlling {
            results.push(c.holds(value(&loc.for_record(at)?)?));
        }
        Some(match self.logic {
            Logic::And => results.iter().all(|&r| r),
            Logic::Or => results.iter().any(|&r| r),
        })
    }

    fn resolvable_for(&self, target: &Loc) -> bool {
        self.controlling.iter().all(|(c, _)| c.single || (c.file, c.block) == (target.file, target.block))
    }
}

/// One way a field is restricted by CPS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restriction {
    /// CPS's rule (`LinkMain.link_main_id`).
    pub rule: i64,
    /// The reason, e.g. `CPS greys this out while RX Only is On`.
    pub reason: String,
}

impl Links {
    /// The `rules` that apply to `files` (decoded with `layout`).
    pub fn new(rules: &LinkRules, layout: &Layout, files: &[DecodedFile]) -> Self {
        let mut out = Links::default();
        'rules: for rule in rules.supported() {
            let mut controlling = Vec::new();
            for c in &rule.controlling {
                match &locate(layout, files, c.param)[..] {
                    [loc] => controlling.push((loc.clone(), c.clone())),
                    // Missing or ambiguous: the rule cannot be evaluated.
                    _ => continue 'rules,
                }
            }
            let mut resolved = Resolved { id: rule.id, logic: rule.logic, controlling, targets: Vec::new() };
            for d in &rule.dependent {
                for loc in locate(layout, files, d.param) {
                    if resolved.resolvable_for(&loc) {
                        resolved.targets.push((loc, d.effects.clone()));
                    }
                }
            }
            if resolved.targets.is_empty() {
                continue;
            }
            let index = out.rules.len();
            for (t, (loc, _)) in resolved.targets.iter().enumerate() {
                out.by_target.entry(loc.key()).or_default().push((index, t));
            }
            for (loc, _) in &resolved.controlling {
                let list = out.by_control.entry(loc.key()).or_default();
                if !list.contains(&index) {
                    list.push(index);
                }
            }
            out.rules.push(resolved);
        }
        out
    }

    /// Number of CPS rules that apply to this configuration.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Why CPS would not let the field at `at` be changed now (greyed out or hidden), if so.
    ///
    /// Of the rules that hold and say whether the field is enabled (or visible), the most specific
    /// ones decide (most controlling fields; e.g. "TX tone type set and RX Only off: TX tone value
    /// enabled" over "RX Only off: TX tone value greyed"); among equally specific ones, greying out
    /// wins. Inferred: CPS applies rules as fields change, so its order is not stored.
    ///
    /// `name` may give a controlling value's name (e.g. a referenced record's), else CPS's option
    /// text from `options` (or the number) is used.
    pub fn restriction(
        &self,
        files: &[DecodedFile],
        options: Option<&Options>,
        name: &dyn Fn(&FieldRef, i64) -> Option<String>,
        at: &FieldRef,
    ) -> Option<Restriction> {
        if at.member.is_some() {
            return None;
        }
        let value = |f: &FieldRef| value_at(files, f).and_then(serde_json::Value::as_i64);
        // (specificity, says enabled/visible, rule) per kind: [enable, visible].
        let mut found: [Vec<(usize, bool, usize)>; 2] = [Vec::new(), Vec::new()];
        for &(r, t) in self.by_target.get(&(at.file, at.block, at.key.clone()))? {
            let rule = &self.rules[r];
            let effects = &rule.targets[t].1;
            if !effects.iter().any(|e| matches!(e, Effect::Enabled(_) | Effect::Visible(_))) || rule.holds(at, &value) != Some(true) {
                continue;
            }
            let specificity = if rule.logic == Logic::And { rule.controlling.len() } else { 1 };
            for e in effects {
                match *e {
                    Effect::Enabled(on) => found[0].push((specificity, on, r)),
                    Effect::Visible(on) => found[1].push((specificity, on, r)),
                    Effect::Value(_) => {}
                }
            }
        }
        let decide = |list: &[(usize, bool, usize)]| -> Option<usize> {
            let top = list.iter().map(|x| x.0).max()?;
            list.iter().find(|x| x.0 == top && !x.1).map(|x| x.2)
        };
        let (verb, r) = match (decide(&found[1]), decide(&found[0])) {
            (Some(r), _) => ("hides this", r),
            (None, Some(r)) => ("greys this out", r),
            (None, None) => return None,
        };
        let rule = &self.rules[r];
        let why = describe(rule, at, &value, options, name);
        Some(Restriction { rule: rule.id, reason: format!("CPS {verb} while {why}") })
    }

    /// Values CPS sets when the field at `at` is changed to `new`: (field, raw value) for every
    /// rule controlled by `at` that holds afterwards. `files` is the configuration before the change.
    pub fn forced(&self, files: &[DecodedFile], at: &FieldRef, new: i64) -> Vec<(FieldRef, i64)> {
        let Some(rules) = self.by_control.get(&(at.file, at.block, at.key.clone())) else { return Vec::new() };
        let value = |f: &FieldRef| {
            if f == at { Some(new) } else { value_at(files, f).and_then(serde_json::Value::as_i64) }
        };
        let mut out: Vec<(FieldRef, i64)> = Vec::new();
        for &r in rules {
            let rule = &self.rules[r];
            for (loc, effects) in &rule.targets {
                let Some(target) = loc.for_record(at) else { continue };
                let Some(v) = effects.iter().find_map(|e| if let Effect::Value(v) = e { Some(*v) } else { None }) else {
                    continue;
                };
                if rule.holds(&target, &value) == Some(true) && !out.iter().any(|(f, _)| *f == target) {
                    out.push((target, v));
                }
            }
        }
        out
    }
}

/// `RX Only is On`, `TX Admit is Always or RX Only is On`: the conditions that hold.
fn describe(
    rule: &Resolved,
    at: &FieldRef,
    value: &dyn Fn(&FieldRef) -> Option<i64>,
    options: Option<&Options>,
    name_of: &dyn Fn(&FieldRef, i64) -> Option<String>,
) -> String {
    let parts: Vec<String> = rule
        .controlling
        .iter()
        .filter_map(|(loc, c)| {
            let field = loc.for_record(at)?;
            let raw = value(&field)?;
            if rule.logic == Logic::Or && !c.holds(raw) {
                return None;
            }
            let name = options.and_then(|o| o.param(loc.param)).map_or(loc.key.clone(), |p| p.name.clone());
            let shown = name_of(&field, raw)
                .or_else(|| options.and_then(|o| o.display(loc.param, raw)))
                .unwrap_or_else(|| raw.to_string());
            Some(format!("{name} is {shown}"))
        })
        .collect();
    parts.join(if rule.logic == Logic::Or { " or " } else { " and " })
}
