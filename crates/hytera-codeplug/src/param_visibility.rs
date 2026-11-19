//! Which parameters CPS shows at all: `Param.visible` and `Param.delete_flag` of
//! `XMLToolCompactData.sdf`, exported by `tools/export-rcdb-options.ps1` to `visibility.json` next
//! to the option catalogue (rows `[id, visible, delete_flag]`). Hytera data: loaded from the user's
//! data folder, never shipped.
//!
//! Observed 2026-11-19 in CPS's code (`docs/research-cps-visibility.md`): `ParamBase.InitVisualState()`
//! shows a parameter in the external build (the one installed, `CurrentCpsVersion` = 2) when its
//! `visible` value (CPS's `ParamVisible` enum, replaced by a matching model-number rule) is 0, 3 or 4
//! and its `delete_flag` is 0.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, Result};

/// CPS's `ParamVisible` values that the external build shows: 0 IsVisible, 3 and 4 (RVM variants).
/// 1 UnVisible, 2 DevelopmentVisible (internal build only), 5 and 6 are not shown.
pub fn shown_by_external_build(visible: i64) -> bool {
    matches!(visible, 0 | 3 | 4)
}

/// One parameter's flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamFlags {
    /// CPS's `ParamVisible` value.
    pub visible: i64,
    /// `Param.delete_flag` != 0.
    pub deleted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ParamVisibility {
    pub rcdb_version: String,
    by_id: HashMap<u32, ParamFlags>,
}

#[derive(Deserialize)]
struct Raw {
    rcdb_version: String,
    params: Vec<(u32, i64, i64)>,
}

impl ParamVisibility {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: Raw = serde_json::from_str(json.trim_start_matches('\u{FEFF}'))
            .map_err(|e| Error::Invalid(format!("visibility JSON: {e}")))?;
        let mut by_id = HashMap::new();
        for (id, visible, delete_flag) in raw.params {
            by_id.entry(id).or_insert(ParamFlags { visible, deleted: delete_flag != 0 });
        }
        Ok(ParamVisibility { rcdb_version: raw.rcdb_version, by_id })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
        Self::parse(&text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
    }

    /// Where the exporter writes `visibility.json` for `version` (next to the option catalogue).
    pub fn default_path(version: &str) -> Option<PathBuf> {
        Some(crate::options::Options::default_path(version)?.with_file_name("visibility.json"))
    }

    pub fn get(&self, id: u32) -> Option<ParamFlags> {
        self.by_id.get(&id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_build_rule() {
        assert!(shown_by_external_build(0));
        assert!(!shown_by_external_build(1));
        assert!(!shown_by_external_build(2), "internal build only");
        assert!(shown_by_external_build(3) && shown_by_external_build(4));
        assert!(!shown_by_external_build(5) && !shown_by_external_build(6));
    }

    #[test]
    fn first_row_per_id_wins() {
        let v = ParamVisibility::parse(r#"{"rcdb_version":"V1","params":[[38,2,0],[4520,0,1],[6130,0,0],[6130,1,1]]}"#).unwrap();
        assert_eq!(v.get(38), Some(ParamFlags { visible: 2, deleted: false }));
        assert_eq!(v.get(4520), Some(ParamFlags { visible: 0, deleted: true }));
        assert_eq!(v.get(6130), Some(ParamFlags { visible: 0, deleted: false }));
        assert_eq!(v.get(1), None);
    }
}
