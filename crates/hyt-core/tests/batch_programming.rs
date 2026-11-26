//! Programming several radios as a batch (`hyt_core::batch`) on simulated BP515s made from the
//! BP515's configuration (captures/gui-write_2026-11-19/after) with other serial numbers. Skipped
//! when the private captures or the CPS install are missing.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use hyt_core::SavedRead;
use hyt_core::batch::{Batch, Entry, IdRule, Outcome, Planned, Rules, parse_plan};
use hyt_core::sim_radio;
use hyt_core::write_flow::WriteSettings;
use hytera_proto::Session;
use hytera_proto::sim::SimRadio;
use serde_json::json;

use common::{Fx, field, fixture, other_radio, temp};

fn settings() -> WriteSettings {
    WriteSettings {
        allow_unverified: false,
        restart_pause: Duration::ZERO,
        restart_attempts: 5,
        programming_date: Some("2030-01-02".into()),
    }
}

/// The configuration's own Radio ID.
fn own_id(fx: &Fx) -> u64 {
    field(fx, &fx.config, "QEU_C_Config", "Radio ID").as_u64().unwrap()
}

fn rules(first: u64) -> Rules {
    Rules { radio_id: IdRule::Sequence { first }, name: Some("Unit {n}".into()), plan: Vec::new(), allow_duplicate_id: false }
}

/// A simulated radio: another BP515 with this serial, Radio ID and name.
struct Radio {
    saved: SavedRead,
    dir: PathBuf,
}

fn radio(fx: &Fx, serial: &str, id: &str) -> Radio {
    let (saved, dir) = other_radio(fx, &format!("batch-{serial}"), serial, id, "Old name");
    Radio { saved, dir }
}

fn program(fx: &Fx, batch: &mut Batch, sim: SimRadio, again: bool) -> (Entry, SimRadio) {
    let mut session = Session::new(sim);
    let entry = batch.program(&mut session, &fx.rcdb, "sim", None, &settings(), again, |_| {}).unwrap();
    (entry, session.into_transport())
}

fn radio_id_on(fx: &Fx, sim: &SimRadio, like: &SavedRead) -> serde_json::Value {
    let now = SavedRead { manifest: serde_json::from_value(serde_json::to_value(&like.manifest).unwrap()).unwrap(), files: sim.files.clone(), radio: None };
    field(fx, &now, "QEU_C_Config", "Radio ID")
}

fn after(e: &Entry) -> (Option<&str>, Option<&str>) {
    let a = e.after.as_ref().expect("after");
    (a.radio_id.as_deref(), a.alias.as_deref())
}

#[test]
fn a_batch_numbers_radios_skips_ones_done_and_keeps_a_log() {
    let Some(fx) = fixture() else { return };
    let own = own_id(&fx);
    let (a, b, c) = (radio(&fx, "R23N900001", "500"), radio(&fx, "R23N900002", "501"), radio(&fx, "R23N900003", "502"));
    let dir = temp("batch-main");
    let mut r = rules(own);
    r.plan.push(Planned { serial: "r23n900003".into(), radio_id: Some((own + 50).to_string()), name: Some("Base".into()) });
    let mut batch = Batch::create(&dir, &fx.folder, r, &fx.rcdb, None).unwrap();
    assert_eq!(batch.file.configuration.radio_id, Some(own.to_string()));

    // The configuration's own Radio ID is skipped.
    let next = batch.assign(None, None).unwrap();
    assert_eq!((next.n, next.radio_id.clone(), next.name.clone()), (1, Some((own + 1).to_string()), Some("Unit 1".into())));

    let (e1, sim) = program(&fx, &mut batch, sim_radio(&a.saved), false);
    assert_eq!(e1.outcome, Outcome::Verified, "{}", e1.message);
    let id1 = (own + 1).to_string();
    assert_eq!(after(&e1), (Some(id1.as_str()), Some("Unit 1")));
    assert_eq!(radio_id_on(&fx, &sim, &a.saved), json!(own + 1));
    assert_eq!(e1.before.as_ref().unwrap().radio_id.as_deref(), Some("500"));
    assert_eq!(sim.written.len(), 55);

    // The planned radio gets its planned ID and name; the sequence carries on after it.
    let (e2, _) = program(&fx, &mut batch, sim_radio(&c.saved), false);
    assert_eq!(after(&e2), (Some((own + 50).to_string().as_str()), Some("Base")));
    assert_eq!(e2.n, Some(2));
    let (e3, _) = program(&fx, &mut batch, sim_radio(&b.saved), false);
    assert_eq!(after(&e3), (Some((own + 2).to_string().as_str()), Some("Unit 3")));

    // Radio A again: skipped without writing, unless asked; then it keeps its ID and name.
    let (e4, sim) = program(&fx, &mut batch, sim_radio(&a.saved), false);
    assert_eq!(e4.outcome, Outcome::Skipped);
    assert!(e4.message.contains("already programmed") && e4.after.is_none(), "{}", e4.message);
    assert!(sim.written.is_empty());
    let (e5, _) = program(&fx, &mut batch, sim_radio(&a.saved), true);
    assert_eq!((e5.outcome, e5.n), (Outcome::Verified, Some(1)));
    assert_eq!(after(&e5), (Some(id1.as_str()), Some("Unit 1")));

    // The log survives reopening; every attempt has its own folder.
    let reopened = Batch::open(&dir).unwrap();
    assert_eq!(reopened.file.radios.len(), 5);
    assert_eq!(reopened.assign(None, None).unwrap().radio_id, Some((own + 3).to_string()));
    let csv = std::fs::read_to_string(dir.join("log.csv")).unwrap();
    assert_eq!(csv.lines().count(), 6, "{csv}");
    assert!(csv.contains("R23N900003") && csv.contains("Base") && csv.contains("skipped"), "{csv}");
    assert!(hyt_core::is_config_folder(&dir.join("radios/001/backup")) && hyt_core::is_config_folder(&dir.join("radios/001/written")));
    assert!(dir.join("radios/004/backup").is_dir() && !dir.join("radios/004/written").exists());
    assert!(dir.join("configuration").join(hyt_core::CONFIGURATION_FILE).is_file());
    for d in [&dir, &a.dir, &b.dir, &c.dir] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn failed_and_unreachable_radios_are_logged_and_retried_with_the_same_id() {
    let Some(fx) = fixture() else { return };
    let own = own_id(&fx);
    let (a, b) = (radio(&fx, "R23N900011", "600"), radio(&fx, "R23N900012", "601"));
    let dir = temp("batch-faults");
    let mut batch = Batch::create(&dir, &fx.folder, rules(own + 10), &fx.rcdb, None).unwrap();

    // No radio answering: logged, nothing assigned.
    let mut off = sim_radio(&a.saved);
    off.faults.dead_after_frames = Some(0);
    let (e1, _) = program(&fx, &mut batch, off, false);
    assert_eq!((e1.outcome, e1.serial.as_deref(), e1.n), (Outcome::NotWritten, None, None));
    assert!(!e1.message.is_empty());

    // Cable pulled while writing: logged as failed, with the ID it was getting.
    let mut probe = Session::new(sim_radio(&a.saved));
    hyt_core::read_radio(&mut probe, &fx.rcdb, |_, _, _, _| {}).unwrap();
    let mut pulled = sim_radio(&a.saved);
    pulled.faults.dead_after_frames = Some(probe.into_transport().frames_received + 120);
    let (e2, sim) = program(&fx, &mut batch, pulled, false);
    assert_eq!(e2.outcome, Outcome::Failed, "{}", e2.message);
    assert!(!sim.written.is_empty() && sim.written.len() < 55, "{} files written", sim.written.len());
    let id = (own + 10).to_string();
    assert_eq!(after(&e2), (Some(id.as_str()), Some("Unit 1")));

    // Another radio does not get that ID; the failed radio does again, without being asked.
    let (e3, _) = program(&fx, &mut batch, sim_radio(&b.saved), false);
    assert_eq!(after(&e3), (Some((own + 11).to_string().as_str()), Some("Unit 2")));
    let (e4, _) = program(&fx, &mut batch, sim_radio(&a.saved), false);
    assert_eq!(e4.outcome, Outcome::Verified, "{}", e4.message);
    assert_eq!(after(&e4), (Some(id.as_str()), Some("Unit 1")));
    for d in [&dir, &a.dir, &b.dir] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn radios_keeping_their_own_id_may_not_share_one() {
    let Some(fx) = fixture() else { return };
    let (a, b) = (radio(&fx, "R23N900021", "700"), radio(&fx, "R23N900022", "700"));
    let dir = temp("batch-keep");
    let keep = Rules { radio_id: IdRule::Keep, name: None, plan: Vec::new(), allow_duplicate_id: false };
    let mut batch = Batch::create(&dir, &fx.folder, keep, &fx.rcdb, None).unwrap();
    let (e1, _) = program(&fx, &mut batch, sim_radio(&a.saved), false);
    assert_eq!(after(&e1), (Some("700"), Some("Old name")));
    let (e2, sim) = program(&fx, &mut batch, sim_radio(&b.saved), false);
    assert_eq!(e2.outcome, Outcome::NotWritten);
    assert!(e2.message.contains("Radio ID 700 was already given to radio R23N900021"), "{}", e2.message);
    assert!(sim.written.is_empty());
    for d in [&dir, &a.dir, &b.dir] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
fn rules_are_checked_when_the_batch_starts() {
    let Some(fx) = fixture() else { return };
    let own = own_id(&fx);
    let dir = temp("batch-rules");
    let refused = |r: Rules| {
        let err = Batch::create(&dir, &fx.folder, r, &fx.rcdb, None).expect_err("refused");
        assert!(!dir.exists(), "nothing created");
        format!("{err:#}")
    };
    let with_name = |n: &str| Rules { name: Some(n.into()), ..rules(100) };
    assert!(refused(with_name("Unit {x}")).contains("unknown {x}"));
    assert!(refused(Rules { radio_id: IdRule::Keep, ..with_name("R{id}") }).contains("{id}"));
    assert!(refused(with_name("A name that is far too long for the radio {n}")).contains("name pattern"));
    assert!(refused(rules(99_999_999)).contains("first Radio ID"));
    let plan = |rows: Vec<(&str, &str)>| Rules {
        plan: rows.into_iter().map(|(s, id)| Planned { serial: s.into(), radio_id: Some(id.into()), name: None }).collect(),
        ..rules(100)
    };
    assert!(refused(plan(vec![("A", "5"), ("B", "5")])).contains("both A and B"));
    assert!(refused(plan(vec![("A", "5"), ("a", "6")])).contains("twice"));
    assert!(refused(plan(vec![("A", &own.to_string())])).contains("configuration's own"));
    assert!(Batch::create(&dir, &fx.folder, plan(vec![("A", "5")]), &fx.rcdb, None).is_ok());
    assert!(Batch::create(&dir, &fx.folder, rules(100), &fx.rcdb, None).is_err(), "an existing batch is not overwritten");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_plan_is_read_from_csv() {
    let plan = parse_plan("\u{FEFF}Serial Number;x\n").err().map(|e| e.to_string());
    assert!(plan.is_some_and(|e| e.contains("serial")));
    let rows = parse_plan("Serial,Radio ID,Name\nR23N1, 101 ,Base\n,,\nR23N2,,Car 2\n").unwrap();
    assert_eq!(
        rows,
        vec![
            Planned { serial: "R23N1".into(), radio_id: Some("101".into()), name: Some("Base".into()) },
            Planned { serial: "R23N2".into(), radio_id: None, name: Some("Car 2".into()) },
        ]
    );
    assert!(parse_plan("serial\nR1\n").is_err(), "needs an ID or name column");
}
