//! Quick setup (the beginner questions) and CPS's help texts, on the radio's configuration after
//! 2026-11-19 (captures/remove-hyt-test-4_2026-11-19/after). Skipped when the private captures, the
//! CPS install or the option catalogue are missing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hyt_core::quick::{NewChannel, Question, QuestionKind, Reply, apply, questions};
use hyt_core::tables::{change_list, table};
use hyt_core::workspace::{ChannelKind, Workspace};
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};

fn open() -> Option<Workspace> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../captures/remove-hyt-test-4_2026-11-19/after");
    let options = Options::default_path("V1.5.24.000")?;
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !folder.is_dir() || !options.is_file() || !dir.is_dir() {
        eprintln!("skipping: captures, CPS RCDB data or option catalogue not present");
        return None;
    }
    Some(Workspace::open(&folder, &RcdbRoot::new(dir), None).unwrap())
}

fn choice(id: &str, answer: &str) -> Reply {
    Reply { question: id.into(), answer: Some(answer.into()), ..Default::default() }
}

fn find<'a>(qs: &'a [Question], id: &str) -> &'a Question {
    qs.iter().find(|q| q.id == id).unwrap_or_else(|| panic!("question {id} not offered"))
}

#[test]
fn every_question_is_offered_for_the_bp515() {
    let Some(ws) = open() else { return };
    let qs = questions(&ws);
    let ids: Vec<&str> = qs.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(ids, ["identity", "clock", "sounds", "screen", "keylock", "vox", "battery", "power", "timeout", "channel"]);
    // What the radio has now (captures/remove-hyt-test-4_2026-11-19/after).
    let current = |id: &str| match &find(&qs, id).kind {
        QuestionKind::Choice { current, .. } => current.clone(),
        _ => panic!("{id} is not a choice"),
    };
    assert_eq!(current("sounds").as_deref(), Some("all"));
    assert_eq!(current("screen").as_deref(), Some("5"));
    assert_eq!(current("keylock").as_deref(), Some("off"));
    assert_eq!(current("vox").as_deref(), Some("off"));
    assert_eq!(current("battery").as_deref(), Some("on"));
    assert_eq!(current("timeout").as_deref(), Some("60"));
    assert_eq!(current("power"), None, "digital channels are High, analog ones Low");
    let QuestionKind::Channel(form) = &find(&qs, "channel").kind else { panic!() };
    assert_eq!(form.frequency_hint, "400..470 MHz");
    assert_eq!(form.slots, ["Slot 1", "Slot 2"]);
    assert_eq!(form.spacings, ["12.5", "20", "25"]);
    assert_eq!(form.zone.as_deref(), Some("Zone 1"));
    assert!(form.tones.contains(&"107.2".to_string()));
}

#[test]
fn every_answer_applies_and_then_is_current() {
    let Some(mut ws) = open() else { return };
    let qs = questions(&ws);
    for q in &qs {
        let QuestionKind::Choice { answers, .. } = &q.kind else { continue };
        for a in answers {
            let outcome = apply(&mut ws, &[choice(&q.id, &a.id)], false).unwrap_or_else(|e| panic!("{} / {}: {e:#}", q.id, a.id));
            let now = questions(&ws);
            let QuestionKind::Choice { current, .. } = &find(&now, &q.id).kind else { panic!() };
            assert_eq!(current.as_deref(), Some(a.id.as_str()), "{} / {}: {:?}", q.id, a.id, outcome.changes);
            ws.discard();
        }
    }
}

#[test]
fn dry_run_previews_without_keeping_anything() {
    let Some(mut ws) = open() else { return };
    let replies = [choice("sounds", "quiet"), choice("power", "high"), choice("battery", "most")];
    let preview = apply(&mut ws, &replies, true).unwrap();
    assert!(!ws.has_changes());
    let settings: Vec<String> = preview.changes.iter().map(|c| format!("{} | {} | {} -> {}", c.place, c.setting, c.old, c.new)).collect();
    // Keypad and power on/off tones off; both analog channels to High; power save 1-4.
    assert_eq!(settings.len(), 5, "{settings:#?}");
    assert!(settings.iter().any(|s| s.contains("Keypad Tone") && s.ends_with("On -> Off")), "{settings:#?}");
    assert_eq!(settings.iter().filter(|s| s.starts_with("Analog channel") && s.ends_with("Low -> High")).count(), 2, "{settings:#?}");
    assert!(settings.iter().any(|s| s.contains("Power Save Mode") && s.ends_with("1-2 -> 1-4")), "{settings:#?}");
    // Applying for real gives the same list.
    let applied = apply(&mut ws, &replies, false).unwrap();
    assert_eq!(applied.changes.len(), 5);
    assert_eq!(change_list(&ws).len(), 5);
}

#[test]
fn values_are_validated_and_nothing_is_kept_on_error() {
    let Some(mut ws) = open() else { return };
    let mut values = BTreeMap::new();
    values.insert("Radio Alias".to_string(), "My BP515".to_string());
    values.insert("Radio ID".to_string(), "2470001".to_string());
    let identity = Reply { question: "identity".into(), values, ..Default::default() };
    let out = apply(&mut ws, std::slice::from_ref(&identity), false).unwrap();
    // Radio IP follows Radio ID like in CPS (10.37.176.113).
    assert_eq!(out.changes.len(), 3, "{:?}", out.changes);
    assert!(out.changes.iter().any(|c| c.setting == "Radio IP" && c.new == (0x71B0_250A_i64).to_string()), "{:?}", out.changes);
    ws.discard();

    // An ID out of range fails the whole request, including the earlier answer.
    let mut bad = identity.clone();
    bad.values.insert("Radio ID".into(), "99999999".into());
    let err = apply(&mut ws, &[choice("sounds", "silent"), bad], false).unwrap_err();
    assert!(format!("{err:#}").contains("Radio ID"), "{err:#}");
    assert!(!ws.has_changes());

    // Values equal to the current ones change nothing.
    let mut same = BTreeMap::new();
    // (Time zone and formats are internal-build-only in CPS, so Language is this radio's only clock field.)
    same.insert("Language".to_string(), "English".to_string());
    let out = apply(&mut ws, &[Reply { question: "clock".into(), values: same, ..Default::default() }], false).unwrap();
    assert!(out.changes.is_empty());
}

fn new_channel(kind: ChannelKind) -> NewChannel {
    NewChannel {
        kind,
        name: "Quick 1".into(),
        rx: "439.9875".into(),
        tx: String::new(),
        color_code: Some("1".into()),
        slot: Some("Slot 2".into()),
        spacing: Some("12.5".into()),
        tone: Some("107.2".into()),
        tone_rx: true,
        add_to_zone: true,
    }
}

#[test]
fn adds_a_channel_and_puts_it_in_the_first_zone() {
    let Some(mut ws) = open() else { return };
    for kind in [ChannelKind::Digital, ChannelKind::Analog] {
        let reply = Reply { question: "channel".into(), channel: Some(new_channel(kind)), ..Default::default() };
        let out = apply(&mut ws, &[reply], false).unwrap_or_else(|e| panic!("{kind:?}: {e:#}"));
        let id = if kind == ChannelKind::Digital { "digital" } else { "analog" };
        let t = table(&ws, id).unwrap();
        let row = t.rows.last().unwrap();
        let text = |key: &str| row.cells[t.columns.iter().position(|c| c.key == key).unwrap()].text.clone();
        assert_eq!(text("Channel Alias"), "Quick 1");
        assert_eq!(text("RX Frequency"), "439.9875");
        assert_eq!(text("TX Frequency"), "439.9875", "TX defaults to RX");
        let summary = out.notes.iter().find(|n| n.starts_with("New ")).expect("channel summary");
        assert!(summary.contains("\"Quick 1\": receives on 439.9875 MHz, transmits on 439.9875 MHz"), "{summary}");
        if kind == ChannelKind::Digital {
            assert_eq!(text("Slot Operation"), "Slot 2");
            assert!(summary.ends_with("colour code 1, Slot 2."), "{summary}");
        } else {
            assert!(summary.ends_with("sends tone 107.2 Hz, listens for tone 107.2 Hz."), "{summary}");
            assert_eq!(text("TX CTCorCDC Value"), "107.2");
            assert_eq!(text("RX CTCorCDC Value"), "107.2");
            assert_eq!(text("RX Squelch Mode"), "CTCSS/CDCSS");
        }
        let zones = table(&ws, "zones").unwrap();
        assert!(zones.rows[0].members.as_ref().unwrap().iter().any(|m| m.alias == "Quick 1"), "{kind:?}");
        assert!(out.changes.iter().any(|c| c.setting == "Channel added"), "{:?}", out.changes);
        ws.discard();
    }

    // A frequency outside the model's band is refused and the channel is not added.
    let mut c = new_channel(ChannelKind::Digital);
    c.rx = "145.500".into();
    let err = apply(&mut ws, &[Reply { question: "channel".into(), channel: Some(c), ..Default::default() }], false).unwrap_err();
    assert!(format!("{err:#}").contains("RX Frequency"), "{err:#}");
    assert!(!ws.has_changes());
}

#[test]
fn fields_carry_cps_help_and_search_finds_it() {
    let Some(ws) = open() else { return };
    assert!(ws.help.is_some(), "CPSParams.xlsx not loaded");
    let digital = table(&ws, "digital").unwrap();
    let alias = digital.columns.iter().find(|c| c.key == "Channel Alias").unwrap();
    assert_eq!(alias.help.as_ref().and_then(|h| h.text.as_deref()), Some("Set the unique name of the channel."));
    let admit = digital.columns.iter().find(|c| c.key == "TX Admit").unwrap();
    assert!(admit.help.as_ref().and_then(|h| h.options.as_deref()).is_some_and(|o| o.contains("Channel Free")));
    // "prolong" only appears in CPS's description of Power Save.
    let found = hyt_core::settings::search(&ws, "prolong");
    let labels: Vec<&str> =
        found.iter().flat_map(|t| &t.sections).flat_map(|s| &s.groups).flat_map(|g| &g.settings).map(|s| s.label.as_str()).collect();
    assert!(labels.contains(&"Power Save"), "{labels:?}");
}

fn column_texts(ws: &Workspace, id: &str, key: &str) -> Vec<String> {
    let t = table(ws, id).unwrap();
    let i = t.columns.iter().position(|c| c.key == key).unwrap();
    t.rows.iter().map(|r| r.cells[i].text.clone()).collect()
}

fn replace() -> Reply {
    Reply { question: "channel".into(), replace_channels: true, ..Default::default() }
}

#[test]
fn replaces_all_channels_with_the_starter_set() {
    let Some(mut ws) = open() else { return };
    let qs = questions(&ws);
    let QuestionKind::Channel(form) = &find(&qs, "channel").kind else { panic!() };
    assert_eq!(form.starter.len(), 12);
    assert_eq!((form.existing_digital, form.existing_analog, form.other_zones), (2, 2, 0));
    // Digital channel 1 has settings the template does not (colour code 9, TX admit Color Code Free).
    assert_eq!(column_texts(&ws, "digital", "TX Admit")[0], "Color Code Free");

    let out = apply(&mut ws, &[replace()], false).unwrap();
    assert_eq!(column_texts(&ws, "digital", "Channel Alias"), ["DMR 1", "DMR 2", "DMR 3", "DMR 4"]);
    assert_eq!(column_texts(&ws, "digital", "RX Frequency"), ["446.10625", "446.11875", "446.13125", "446.14375"]);
    assert_eq!(column_texts(&ws, "digital", "TX Frequency"), column_texts(&ws, "digital", "RX Frequency"));
    assert!(column_texts(&ws, "digital", "Color Code").iter().all(|c| c == "1"));
    assert!(column_texts(&ws, "digital", "Slot Operation").iter().all(|c| c == "Slot 1"));
    assert_eq!(column_texts(&ws, "digital", "TX Admit")[0], "Channel Free", "reset to CPS's template");
    let analog = column_texts(&ws, "analog", "Channel Alias");
    assert_eq!(analog, (1..=8).map(|n| format!("PMR {n}")).collect::<Vec<_>>());
    assert_eq!(column_texts(&ws, "analog", "RX Frequency")[7], "446.09375");
    assert!(column_texts(&ws, "analog", "RX CTCorCDC Type").iter().all(|c| c == "None"), "old tones are gone");
    for id in ["digital", "analog"] {
        assert!(column_texts(&ws, id, "Power Level").iter().all(|c| c == "Low"), "{id}");
    }
    let zones = table(&ws, "zones").unwrap();
    let members: Vec<String> = zones.rows[0].members.as_ref().unwrap().iter().map(|m| m.alias.clone()).collect();
    assert_eq!(members, ["DMR 1", "DMR 2", "DMR 3", "DMR 4", "PMR 1", "PMR 2", "PMR 3", "PMR 4", "PMR 5", "PMR 6", "PMR 7", "PMR 8"]);
    assert!(out.notes.iter().any(|n| n.starts_with("All channels replaced by 4 digital (DMR 1–DMR 4")), "{:?}", out.notes);
}

#[test]
fn replacing_removes_extra_channels() {
    let Some(mut ws) = open() else { return };
    for _ in 0..5 {
        ws.add_channel(ChannelKind::Digital).unwrap();
    }
    assert_eq!(ws.channel_count(ChannelKind::Digital), 7);
    apply(&mut ws, &[replace()], false).unwrap();
    assert_eq!(column_texts(&ws, "digital", "Channel Alias"), ["DMR 1", "DMR 2", "DMR 3", "DMR 4"]);
}

#[test]
fn starting_fresh_with_every_recommended_answer() {
    let Some(mut ws) = open() else { return };
    let qs = questions(&ws);
    let mut replies: Vec<Reply> = qs
        .iter()
        .filter_map(|q| match &q.kind {
            QuestionKind::Choice { recommended, .. } => Some(choice(&q.id, recommended.as_deref().expect("every choice has a recommendation"))),
            _ => None,
        })
        .collect();
    // The channel step comes last in the replies; it is still applied first.
    replies.push(replace());
    let out = apply(&mut ws, &replies, false).unwrap();
    assert!(!out.changes.is_empty());
    for id in ["digital", "analog"] {
        assert!(column_texts(&ws, id, "Power Level").iter().all(|c| c == "Low"), "{id}");
    }
    let now = questions(&ws);
    for q in &now {
        if let QuestionKind::Choice { current, recommended, .. } = &q.kind {
            assert_eq!(current, recommended, "{}", q.id);
        }
    }
}
