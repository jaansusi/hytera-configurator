//! Decoding is exact and complete for every file read from the BP515.
//!
//! For each file: overwrite every record byte (members, for two-level blocks) with filler, write
//! the decoded field values back, and require the original bytes. Passing proves that the codec
//! round-trips and that the RCDB fields cover every record byte.
//! Skipped when the private captures or the CPS install are missing.

use std::path::{Path, PathBuf};

use hyt_core::{decode_read, load_config_default};
use hytera_codeplug::decode::apply;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};
use hytera_codeplug::structure::{parse, parse_sublist};

fn scramble(file: &[u8]) -> Vec<u8> {
    let mut out = file.to_vec();
    for block in parse(file).unwrap().blocks {
        for range in block.records {
            if block.two_dim {
                let sub = parse_sublist(&file[range.clone()]).unwrap();
                for m in sub.members {
                    out[range.start + m.start..range.start + m.end].fill(0xA5);
                }
            } else {
                out[range].fill(0xA5);
            }
        }
    }
    out
}

#[test]
fn every_file_round_trips_through_named_fields() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rcdb_dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    // The last one is an older BP515 (firmware V1.5.13, RCDB V1.5.08.000, 57 files).
    let folders = [
        "captures/hyt-read_2026-11-18",
        "captures/hyt-read-after-write_2026-11-18",
        "captures/old-bp515-read_2026-11-26",
    ];
    if !rcdb_dir.is_dir() || folders.iter().any(|f| !root.join(f).is_dir()) {
        eprintln!("skipping: captures or CPS RCDB data not present");
        return;
    }
    let rcdb = RcdbRoot::new(rcdb_dir);
    for folder in folders {
        let saved = load_config_default(&root.join(folder)).unwrap();
        let layout = rcdb.layout(&saved.manifest.rcdb_version).unwrap();
        let decoded = decode_read(&saved, &layout).unwrap();
        assert_eq!(decoded.len(), saved.manifest.files.len());
        assert!(decoded.len() >= 57, "{folder}: only {} files decoded", decoded.len());
        for d in &decoded {
            let original = &saved.files[&d.id];
            let rebuilt = apply(&scramble(original), d, &layout).unwrap();
            assert!(rebuilt == *original, "{folder}: file {} ({}) did not round-trip", d.id, d.name);
        }
    }
}
