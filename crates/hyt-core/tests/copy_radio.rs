//! "Copy to this radio" (`hyt_core::copy`) and Radio IP following Radio ID, on the BP515's
//! configuration (captures/gui-write_2026-11-19/after) and a made-up second BP515 derived from it
//! (other serial number, Radio ID, alias, radio data and channel 1), offline and on a simulated
//! radio. Skipped when the private captures or the CPS install are missing.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use hyt_core::copy::{CopyEvent, CopyJob, CopyOptions, copy_to_radio, prepare, prepare_folder};
use hyt_core::workspace::{Workspace, radio_ip, single_field, value_at};
use hyt_core::write_flow::{WriteEvent, WriteSettings};
use hyt_core::{SavedRead, load_config_default, sim_radio};
use hytera_proto::Session;

use common::{Fx, channel_1_alias, field, fixture, temp};
use serde_json::json;

const OTHER_SERIAL: &str = "R23N999999";

/// A second BP515 with serial [`OTHER_SERIAL`], Radio ID `id` and alias "Unit 2".
fn other_radio(fx: &Fx, name: &str, id: &str) -> (SavedRead, PathBuf) {
    common::other_radio(fx, name, OTHER_SERIAL, id, "Unit 2")
}

#[test]
fn radio_ip_follows_cps_rule() {
    assert_eq!(radio_ip(10, 1), Some(16777226)); // 10.0.0.1, as read from the BP515
    assert_eq!(radio_ip(10, 2470001), Some(0x71B0_250A)); // 10.37.176.113
    assert_eq!(hyt_core::copy::dotted_ip(0x71B0_250A).as_deref(), Some("10.37.176.113"));
    assert_eq!(radio_ip(0, 1), None);
    assert_eq!(radio_ip(127, 1), None);
    assert_eq!(radio_ip(10, 0), None);
    assert_eq!(radio_ip(10, 16_776_416), None);
}

#[test]
fn changing_radio_id_updates_radio_ip_like_cps() {
    let Some(fx) = fixture() else { return };
    let mut ws = Workspace::open(&fx.folder, &fx.rcdb, None).unwrap();
    let id = single_field(ws.view(), "QEU_C_Config", "Radio ID").unwrap();
    let ip = single_field(ws.view(), "QEU_C_Config", "Radio IP").unwrap();
    assert_eq!(value_at(ws.view(), &ip), Some(&json!(16777226)));
    ws.set_value(&id, "2470001").unwrap();
    assert_eq!(value_at(ws.view(), &ip), Some(&json!(0x71B0_250A_i64)));
    // Back to the original ID: nothing pending any more.
    ws.set_value(&id, "1").unwrap();
    assert!(!ws.has_changes(), "{:?}", ws.changes());
}

#[test]
fn copying_to_the_radio_it_came_from_changes_nothing() {
    let Some(fx) = fixture() else { return };
    let (ws, plan) = prepare(&fx.config, &fx.config, &fx.folder, &fx.rcdb, None, &CopyOptions::default()).unwrap();
    assert_eq!(ws.files(), &fx.config.files);
    assert_eq!(plan.changed_settings, 0);
    assert_eq!(plan.before, plan.after);
}

#[test]
fn a_copy_keeps_the_target_radios_own_identity_and_files() {
    let Some(fx) = fixture() else { return };
    let (target, target_dir) = other_radio(&fx, "keep-target", "2470002");
    let out = temp("keep-out");
    let plan = prepare_folder(&fx.config, "config", &target_dir, &out, &fx.rcdb, None, &CopyOptions::default()).unwrap();
    let copied = load_config_default(&out).unwrap();

    assert_eq!(copied.manifest.serial_number.as_deref(), Some(OTHER_SERIAL));
    assert_eq!(field(&fx, &copied, "QEU_C_Config", "Radio ID"), field(&fx, &target, "QEU_C_Config", "Radio ID"));
    assert_eq!(field(&fx, &copied, "Comm_Basic", "Radio Alias"), json!("Unit 2"));
    assert_eq!(field(&fx, &copied, "Comm_Basic", "Serial Number"), json!(23_999_999));
    // Radio IP is the target's (derived from its own ID), not the configuration's.
    assert_eq!(field(&fx, &copied, "QEU_C_Config", "Radio IP"), json!(radio_ip(10, 2470002).unwrap()));
    // Settings come from the configuration.
    assert_eq!(channel_1_alias(&fx, &copied.files), channel_1_alias(&fx, &fx.config.files));
    assert_ne!(channel_1_alias(&fx, &target.files), channel_1_alias(&fx, &fx.config.files));
    for id in hytera_models::BP515.write_excluded_files {
        assert_eq!(copied.files[id], target.files[id], "never-written file {id}");
    }
    assert_eq!(plan.before.serial.as_deref(), Some(OTHER_SERIAL));
    assert_eq!(plan.before.radio_id.as_deref(), Some("2470002"));
    assert_eq!(plan.after, plan.before);
    assert_eq!(plan.radio_ip.as_deref(), Some("10.37.176.114"));
    assert!(plan.changed_settings >= 1);
    for d in [&out, &target_dir] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn a_copy_can_give_the_radio_a_new_id_and_name() {
    let Some(fx) = fixture() else { return };
    let (target, target_dir) = other_radio(&fx, "new-id", "2470002");
    let opts = CopyOptions { radio_id: Some("2470003".into()), alias: Some("Unit 3".into()), ..Default::default() };
    let (ws, plan) = prepare(&fx.config, &target, &target_dir, &fx.rcdb, None, &opts).unwrap();
    assert_eq!(plan.after.radio_id.as_deref(), Some("2470003"));
    assert_eq!(plan.after.alias.as_deref(), Some("Unit 3"));
    assert_eq!(plan.radio_ip.as_deref(), Some("10.37.176.115"));
    assert_eq!(plan.before.alias.as_deref(), Some("Unit 2"));
    drop(ws);
    // A value CPS would not accept is refused.
    let bad = CopyOptions { radio_id: Some("99999999".into()), ..Default::default() };
    let err = prepare(&fx.config, &target, &target_dir, &fx.rcdb, None, &bad).err().expect("refused");
    assert!(format!("{err:#}").contains("Radio ID"), "{err:#}");
    std::fs::remove_dir_all(&target_dir).unwrap();
}

#[test]
fn the_configurations_own_radio_id_is_refused_for_another_radio_unless_allowed() {
    let Some(fx) = fixture() else { return };
    let source_id = field(&fx, &fx.config, "QEU_C_Config", "Radio ID").to_string();
    // The target already has the configuration's ID (e.g. both still have the factory ID).
    let (target, target_dir) = other_radio(&fx, "dup", &source_id);
    let err = prepare(&fx.config, &target, &target_dir, &fx.rcdb, None, &CopyOptions::default()).err().expect("refused");
    assert!(format!("{err:#}").contains("same as the radio the configuration came from"), "{err:#}");
    let opts = CopyOptions { radio_id: Some(source_id.clone()), ..Default::default() };
    assert!(prepare(&fx.config, &target, &target_dir, &fx.rcdb, None, &opts).is_err());
    let allowed = CopyOptions { allow_duplicate_id: true, ..Default::default() };
    assert!(prepare(&fx.config, &target, &target_dir, &fx.rcdb, None, &allowed).is_ok());
    let fresh = CopyOptions { radio_id: Some("2470009".into()), ..Default::default() };
    assert!(prepare(&fx.config, &target, &target_dir, &fx.rcdb, None, &fresh).is_ok());
    std::fs::remove_dir_all(&target_dir).unwrap();
}

#[test]
fn another_model_number_is_refused_and_an_unknown_layout_fails() {
    let Some(fx) = fixture() else { return };
    let (mut target, target_dir) = other_radio(&fx, "band", "2470002");
    let code = target.manifest.info.iter_mut().find(|e| e.item == "0x0000").unwrap();
    code.text = Some("XX9999".into());
    let err = prepare(&fx.config, &target, &target_dir, &fx.rcdb, None, &CopyOptions::default()).err().expect("refused");
    assert!(format!("{err:#}").contains("model number"), "{err:#}");
    let mut target = load_config_default(&target_dir).unwrap();
    // Another RCDB version is converted (tests/configuration.rs); one CPS has no layout for fails.
    target.manifest.rcdb_version = "V9.9.99.000".into();
    let err = prepare(&fx.config, &target, &target_dir, &fx.rcdb, None, &CopyOptions::default()).err().expect("refused");
    assert!(format!("{err:#}").contains("V9.9.99.000"), "{err:#}");
    std::fs::remove_dir_all(&target_dir).unwrap();
}

#[test]
fn copy_to_a_simulated_second_radio_reads_once_writes_and_verifies() {
    let Some(fx) = fixture() else { return };
    let (target, target_dir) = other_radio(&fx, "sim-target", "2470002");
    let mut radio = sim_radio(&target);
    radio.faults.restart_requests = 2;
    let (backup, out) = (temp("sim-backup"), temp("sim-out"));
    let opts = CopyOptions { radio_id: Some("2470004".into()), alias: Some("Unit 4".into()), ..Default::default() };
    let job = CopyJob {
        config: &fx.config,
        config_source: "config",
        backup: &backup,
        out: &out,
        port: "sim",
        options_path: None,
    };
    let settings = WriteSettings {
        allow_unverified: false,
        restart_pause: Duration::ZERO,
        restart_attempts: 5,
        programming_date: Some("2030-01-02".into()),
    };
    let mut session = Session::new(radio);
    let mut backup_reads = 0;
    let mut prepared = false;
    let report = copy_to_radio(&mut session, &fx.rcdb, &job, &opts, &settings, |e| match e {
        CopyEvent::Step(WriteEvent::BackupSaved { .. }) => backup_reads += 1,
        CopyEvent::Prepared(_) => prepared = true,
        _ => {}
    })
    .unwrap();
    let radio = session.into_transport();
    assert_eq!((backup_reads, prepared), (1, true));
    assert_eq!(report.write.written, 55);
    assert_eq!(load_config_default(&backup).unwrap().files, target.files, "the backup is the radio before the copy");
    let written = load_config_default(&out).unwrap();
    assert_eq!(written.manifest.serial_number.as_deref(), Some(OTHER_SERIAL));
    assert_eq!(field(&fx, &written, "QEU_C_Config", "Radio ID"), json!(2470004));
    assert_eq!(channel_1_alias(&fx, &radio.files), channel_1_alias(&fx, &fx.config.files));
    assert_eq!(report.plan.after.alias.as_deref(), Some("Unit 4"));

    // Asking for the configuration's own Radio ID is refused before anything is written.
    let (backup2, out2) = (temp("sim-backup2"), temp("sim-out2"));
    let job2 = CopyJob { backup: &backup2, out: &out2, ..job };
    let mut s2 = Session::new(sim_radio(&target));
    let source_id = field(&fx, &fx.config, "QEU_C_Config", "Radio ID").to_string();
    let dup = CopyOptions { radio_id: Some(source_id), ..Default::default() };
    let err = copy_to_radio(&mut s2, &fx.rcdb, &job2, &dup, &settings, |_| {}).expect_err("refused");
    let text = format!("{err:#}");
    assert!(text.contains("nothing was written") && text.contains("same as the radio"), "{text}");
    assert!(s2.into_transport().written.is_empty());
    assert!(!out2.exists());
    for d in [&backup, &out, &backup2, &target_dir] {
        std::fs::remove_dir_all(d).unwrap();
    }
}
