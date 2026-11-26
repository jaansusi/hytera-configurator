//! Configurations as data: rebuilt onto radios of the same and of another RCDB layout.
//! Skipped when the private captures or the CPS install are missing.

use std::path::{Path, PathBuf};

use hyt_core::config::overlay;
use hyt_core::{decode_read, load_config_default, load_read};
use hytera_codeplug::container::prepare_for_write;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};

const NEW: &str = "captures/hyt-read_2026-11-18"; // BP515 R23N141382, RCDB V1.5.24.000
const OLD: &str = "captures/old-bp515-read_2026-11-26"; // BP515 R225105080, RCDB V1.5.08.000
const EDITED: &str = "captures/unattended_2026-11-19/end"; // radio 1's configuration, 3 channels etc.

fn setup() -> Option<(RcdbRoot, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !dir.is_dir() || [NEW, OLD, EDITED].iter().any(|f| !root.join(f).join("manifest.json").is_file()) {
        eprintln!("skipping: captures or CPS RCDB data not present");
        return None;
    }
    Some((RcdbRoot::new(dir), root))
}

#[test]
fn same_layout_overlay_is_exact() {
    let Some((rcdb, root)) = setup() else { return };
    for folder in [NEW, OLD] {
        let saved = load_config_default(&root.join(folder)).unwrap();
        let layout = rcdb.layout(&saved.manifest.rcdb_version).unwrap();
        let data = decode_read(&saved, &layout).unwrap();
        let o = overlay(&data, &layout, &saved, &layout, &[]).unwrap();
        assert!(o.report.is_exact() && !o.report.converted(), "{folder}: {:?}", o.report.lines());
        for (id, bytes) in &saved.files {
            assert_eq!(prepare_for_write(&o.files[id]), prepare_for_write(bytes), "{folder}: file {id}");
        }
    }
}

#[test]
fn configuration_of_newer_firmware_goes_onto_the_older_radio() {
    let Some((rcdb, root)) = setup() else { return };
    let config = load_config_default(&root.join(EDITED)).unwrap();
    let old = load_config_default(&root.join(OLD)).unwrap();
    let (cl, ol) = (rcdb.layout(&config.manifest.rcdb_version).unwrap(), rcdb.layout(&old.manifest.rcdb_version).unwrap());
    let data = decode_read(&config, &cl).unwrap();
    let o = overlay(&data, &cl, &old, &ol, &[242, 245, 253]).unwrap();
    for line in o.report.lines() {
        eprintln!("{line}");
    }
    assert!(o.report.converted());
    assert!(o.report.files_left_out.iter().any(|f| f.starts_with("254 ")), "{:?}", o.report.files_left_out);
    // Every shared setting carries over: decode the result and compare channels, zones, contacts.
    let result = hyt_core::SavedRead { manifest: old.manifest, files: o.files, radio: None };
    let after = decode_read(&result, &ol).unwrap();
    for name in ["QEU_C_DChannel", "QEU_C_AChannel", "QEU_C_Zone", "QEU_C_DContact", "QEU_C_ZoneAlias"] {
        let a = data.iter().find(|f| f.name == name).unwrap();
        let b = after.iter().find(|f| f.name == name).unwrap();
        assert_eq!(a.blocks[0].records.len(), b.blocks[0].records.len(), "{name}");
        for (ra, rb) in a.blocks[0].records.iter().zip(&b.blocks[0].records) {
            if let (Some(fa), Some(fb)) = (&ra.fields, &rb.fields) {
                for (k, v) in &fb.0 {
                    if let Some(va) = fa.0.get(k) {
                        assert_eq!(va, v, "{name} {k}");
                    }
                }
            }
            assert_eq!(ra.members, rb.members, "{name} members");
        }
    }
}

/// The owner's case of 2026-11-26: writing a configuration made from radio 1 (RCDB V1.5.24.000)
/// onto the older radio (RCDB V1.5.08.000) with a plain write. It is converted to the older
/// layout, the older radio keeps its own identity, and the write reads back as written.
#[test]
fn plain_write_of_another_radios_configuration_onto_the_older_radio() {
    use hyt_core::write_flow::{WriteJob, WriteSettings, write_config};
    let Some((rcdb, root)) = setup() else { return };
    let config = load_config_default(&root.join(EDITED)).unwrap();
    let old = load_config_default(&root.join(OLD)).unwrap();
    let backup = std::env::temp_dir().join(format!("hyt-test-cross-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&backup);
    let job = WriteJob { saved: &config, source: EDITED, backup: &backup, port: "sim" };
    let settings = WriteSettings {
        restart_pause: std::time::Duration::ZERO,
        restart_attempts: 3,
        programming_date: Some("2030-01-02".into()),
        ..WriteSettings::default()
    };
    let mut session = hytera_proto::Session::new(hyt_core::sim_radio(&old));
    let report = write_config(&mut session, &rcdb, &job, &settings, |_| {}).unwrap();
    let plan = report.prepared.expect("plan");
    assert!(plan.other_radio() && plan.conversion.converted(), "{:?}", plan.conversion.lines());
    assert_eq!(report.written, 54);
    assert_eq!(plan.after.serial.as_deref(), Some("R225105080"));
    assert_eq!(plan.after.radio_id, plan.before.radio_id, "the older radio keeps its Radio ID");
    assert_eq!(plan.after.alias, plan.before.alias, "and its name");

    // What the radio now holds: radio 1's channels in the older layout.
    let radio = session.into_transport();
    let ol = rcdb.layout("V1.5.08.000").unwrap();
    let cl = rcdb.layout(&config.manifest.rcdb_version).unwrap();
    let alias = |files: &std::collections::BTreeMap<u16, Vec<u8>>, l| {
        let d = hytera_codeplug::decode::decode(&files[&3], "QEU_C_DChannel", l).unwrap();
        d.blocks[0].records.iter().map(|r| r.fields.as_ref().unwrap().0["Channel Alias"].clone()).collect::<Vec<_>>()
    };
    assert_eq!(alias(&radio.files, &ol), alias(&config.files, &cl));
    // Its programming passwords are its own, not radio 1's.
    let secrets = |files: &std::collections::BTreeMap<u16, Vec<u8>>| {
        let d = hytera_codeplug::decode::decode(&files[&18], "Comm_Basic", &ol).unwrap();
        d.blocks.iter().flat_map(|b| &b.records).filter_map(|r| r.fields.as_ref())
            .flat_map(|f| f.0.iter().filter(|(k, _)| hyt_core::view::is_secret(k)).map(|(k, v)| (k.clone(), v.clone())))
            .collect::<Vec<_>>()
    };
    assert!(!secrets(&old.files).is_empty());
    assert_eq!(secrets(&radio.files), secrets(&old.files));
    assert!(plan.kept.iter().any(|k| k.contains("passwords")), "{:?}", plan.kept);
    let log = std::fs::read_to_string(backup.join(hyt_core::write_flow::WRITE_LOG)).unwrap();
    assert!(log.contains("wrote 54/54") && log.contains("finished"), "{log}");
    std::fs::remove_dir_all(&backup).unwrap();
}

/// A read saved as a configuration folder holds its data and the untouched read; edits saved from
/// it are data too, and both open again to the same settings.
#[test]
fn folders_hold_data_and_the_read_they_came_from() {
    let Some((rcdb, root)) = setup() else { return };
    let read = load_config_default(&root.join(OLD)).unwrap();
    let mut ws = hyt_core::workspace::Workspace::from_saved(read, &root.join(OLD), &rcdb, None).unwrap();
    let at = hyt_core::workspace::single_field(ws.view(), "Comm_Basic", "Radio Alias").unwrap();
    ws.set_value(&at, "Data test").unwrap();
    let out = std::env::temp_dir().join(format!("hyt-test-config-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    ws.save_as(&out).unwrap();
    assert!(out.join(hyt_core::CONFIGURATION_FILE).is_file());
    let raw = load_read(&out.join(hyt_core::RADIO_DIR)).unwrap();
    assert_eq!(raw.files, load_read(&root.join(OLD)).unwrap().files, "radio/ is the untouched read");
    let text = std::fs::read_to_string(out.join(hyt_core::CONFIGURATION_FILE)).unwrap();
    assert!(text.contains("\"Data test\"") && text.contains("\"layout\": \"V1.5.08.000\""));
    let again = hyt_core::workspace::Workspace::open(&out, &rcdb, None).unwrap();
    assert_eq!(hyt_core::workspace::value_at(again.view(), &at), Some(&serde_json::json!("Data test")));
    std::fs::remove_dir_all(&out).unwrap();
}
