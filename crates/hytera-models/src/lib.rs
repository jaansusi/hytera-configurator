//! Per-model knowledge. Everything shared by the G2/Auctus platform lives in `hytera-proto` and
//! `hytera-codeplug`; a profile records only what differs per model and what has been verified.
//!
//! Adding a model: record CPS reading it (docs/capture-howto.md), add a profile here, and add a
//! replay test for the recording.

use hytera_proto::RadioInfo;
use hytera_proto::session::DEFAULT_CHUNK_SIZE;

#[derive(Debug)]
pub struct ModelProfile {
    /// Stable identifier, e.g. `bp515`.
    pub key: &'static str,
    pub display_name: &'static str,
    /// Model-name prefixes (info item 0x11) that select this profile.
    pub model_name_prefixes: &'static [&'static str],
    /// RCDB versions verified against a real radio of this model.
    pub verified_rcdb_versions: &'static [&'static str],
    /// Chunk size used when reading and writing files.
    pub chunk_size: u16,
    /// Files that are read but never written back (as observed from CPS).
    pub write_excluded_files: &'static [u16],
    /// Whether writing has been verified on a real radio of this model. Until then the CLI only
    /// writes with an explicit override.
    pub write_supported: bool,
}

impl ModelProfile {
    pub fn is_verified_rcdb(&self, version: &str) -> bool {
        self.verified_rcdb_versions.contains(&version)
    }

    /// Whether writing has been verified for this model with this RCDB version. A radio with older
    /// or newer firmware has another layout and file list, so a write verified on one version says
    /// nothing about another.
    pub fn write_verified(&self, rcdb_version: &str) -> bool {
        self.write_supported && self.is_verified_rcdb(rcdb_version)
    }
}

pub static BP515: ModelProfile = ModelProfile {
    key: "bp515",
    display_name: "Hytera BP515",
    model_name_prefixes: &["BP515"],
    // V1.5.24.000: firmware V1.5.36 (2026-11-18). V1.5.08.000: firmware V1.5.13 (2026-11-26: CPS's read
    // and write replayed byte for byte, then a live `hyt write` read back as written).
    verified_rcdb_versions: &["V1.5.24.000", "V1.5.08.000"],
    chunk_size: DEFAULT_CHUNK_SIZE,
    // CPS skipped these on 2026-11-18: E2EE key list, Bluetooth paired list, radio info.
    write_excluded_files: &[242, 245, 253],
    // 2026-11-18: `hyt write` of an unchanged configuration; all 55 files read back as written.
    write_supported: true,
};

/// Used for G2 radios without a profile: reading with CPS defaults only, never writing.
pub static GENERIC_G2: ModelProfile = ModelProfile {
    key: "generic-g2",
    display_name: "Unrecognised Hytera G2 radio",
    model_name_prefixes: &[],
    verified_rcdb_versions: &[],
    chunk_size: DEFAULT_CHUNK_SIZE,
    write_excluded_files: &[],
    write_supported: false,
};

pub static PROFILES: &[&ModelProfile] = &[&BP515];

/// The profile for an identified radio, or [`GENERIC_G2`] when the model is unknown.
pub fn profile_for(info: &RadioInfo) -> &'static ModelProfile {
    let name = info.model_name().unwrap_or_default();
    PROFILES
        .iter()
        .copied()
        .find(|p| p.model_name_prefixes.iter().any(|prefix| name.starts_with(prefix)))
        .unwrap_or(&GENERIC_G2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hytera_proto::InfoItem;
    use hytera_proto::session::item;

    fn info_with_model(name: &str) -> RadioInfo {
        let mut info = RadioInfo::default();
        let value = name.encode_utf16().flat_map(u16::to_be_bytes).collect();
        info.items.insert(item::MODEL_NAME, InfoItem { item: item::MODEL_NAME, status: 0, value });
        info
    }

    #[test]
    fn selects_profiles() {
        assert_eq!(profile_for(&info_with_model("BP515 U1")).key, "bp515");
        assert_eq!(profile_for(&info_with_model("HP785")).key, "generic-g2");
        assert_eq!(profile_for(&RadioInfo::default()).key, "generic-g2");
    }

    #[test]
    fn writing_is_verified_per_rcdb_version() {
        assert!(BP515.write_verified("V1.5.24.000"));
        assert!(BP515.write_verified("V1.5.08.000"));
        // Any other firmware's layout is unverified until a write to it has been checked.
        assert!(!BP515.write_verified("V1.5.16.000"));
        assert!(!GENERIC_G2.write_verified("V1.5.24.000"));
    }
}
