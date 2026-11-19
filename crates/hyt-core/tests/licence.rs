//! Licence package checks (CPS's Authorization rules) and the radio's licence answer (item 0x0200).
//! Packages here are synthetic: a made-up serial and `.fcr` bytes.

use std::io::Write;

use hyt_core::licence::{Level, RadioFacts, check_package, installed_licences, refused_codes};
use md5::{Digest, Md5};

const BP515_U1: &str = "55P10B00-1L0BAPU1U100000000CB000000000000-00000NN0000000000000000000000000-00040000";
const BP515_LF: &str = "55P10B00-1L0BAPB1B100000000CB000000000000-00000NN0000000000000000000000000-00040000";

fn md5_hex(b: &[u8]) -> String {
    Md5::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn readme(serial: &str, fcr_name: &str, fcr: &[u8], features: &[(&str, &str)]) -> String {
    let items: String = features
        .iter()
        .map(|(c, v)| format!("<item><code>{c}</code><value>{v}</value></item>"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<license><filelist><package>\
         <objdevice>{serial}</objdevice><file>{fcr_name}</file><md5>{}</md5><detail>{items}</detail>\
         </package></filelist></license>",
        md5_hex(fcr)
    )
}

fn zip_of(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut out);
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }
    out.into_inner()
}

/// A well-formed package: readme.xml, readme.md5 and the .fcr.
fn package(serial: &str, features: &[(&str, &str)]) -> Vec<u8> {
    let fcr = b"not a real licence".to_vec();
    let xml = readme(serial, "TEST0001.fcr", &fcr, features);
    zip_of(&[
        ("readme.xml", xml.clone().into_bytes()),
        ("readme.md5", md5_hex(xml.as_bytes()).into_bytes()),
        ("TEST0001.fcr", fcr),
    ])
}

fn radio(model_name: &str, model_number: &str) -> RadioFacts {
    RadioFacts { serial: "TEST0001".into(), model_name: model_name.into(), model_number: model_number.into() }
}

fn refused(r: &hyt_core::licence::PackageReport) -> Vec<&str> {
    r.checks.iter().filter(|c| c.level == Level::Refused).map(|c| c.text.as_str()).collect()
}

#[test]
fn roam_and_two_tone_are_accepted_for_bp515_u1() {
    let zip = package("TEST0001", &[("1503", "1"), ("1507", "1"), ("1509", "0")]);
    let r = check_package(&zip, Some(&radio("BP515 U1", BP515_U1))).unwrap();
    assert!(r.accepted(), "{:?}", r.checks);
    assert_eq!(r.packages.len(), 1);
    assert_eq!(r.packages[0].fcr_md5_ok, Some(true));
    assert_eq!(r.packages[0].features.iter().filter(|f| f.licensed).count(), 2);
}

#[test]
fn basic_encryption_is_refused_for_bp515_u1_by_cps_1_5_24() {
    let zip = package("TEST0001", &[("1509", "1")]);
    let r = check_package(&zip, Some(&radio("BP515 U1", BP515_U1))).unwrap();
    assert!(!r.accepted());
    let refused = refused(&r);
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].contains("Basic Encryption (1509)"), "{}", refused[0]);
}

#[test]
fn model_rules_match_cps() {
    let bp515 = refused_codes("BP515 U1", BP515_U1).unwrap();
    assert_eq!(bp515, ["1502", "1504", "1505", "1506", "1508", "1509"]);
    assert!(refused_codes("BP515 LF", BP515_LF).unwrap().contains(&"1503"));
    let bp36 = refused_codes("BP365", "36P10B00-1L0BAPU1U1").unwrap();
    assert!(bp36.contains(&"1503") && bp36.contains(&"1507"));
    // 15th character '0': only Two Tone is allowed.
    assert_eq!(refused_codes("XX", "00000000-000000000000").unwrap().len(), 9);
    // K models: only Roam, BP578K nothing; model number characters 39-41 "051" count as K.
    assert_eq!(refused_codes("BP568K U1", BP515_U1).unwrap().len(), 9);
    assert_eq!(refused_codes("BP578K U1", BP515_U1).unwrap().len(), 10);
    let k = "55P10B00-1L0BAPU1U100000000CB0000000000051-00000NN0000000000000000000000000-00040000";
    assert!(!refused_codes("BP515 U1", k).unwrap().contains(&"1503"));
    // Basic Encryption is refused for every model in CPS V1.5.24.
    for (name, number) in [("BP515 U1", BP515_U1), ("BP568K U1", BP515_U1), ("AP510 U1", "50P10B00-1L0BA0U1U1"), ("BP578K V1", k)] {
        assert!(refused_codes(name, number).unwrap().contains(&"1509"), "{name}");
    }
}

#[test]
fn wrong_serial_is_refused() {
    let zip = package("OTHER123", &[("1503", "1")]);
    let r = check_package(&zip, Some(&radio("BP515 U1", BP515_U1))).unwrap();
    assert!(refused(&r)[0].contains("no package for serial TEST0001"), "{:?}", r.checks);
}

#[test]
fn checksums_are_compared_like_cps() {
    let fcr = b"abc".to_vec();
    let xml = readme("TEST0001", "TEST0001.fcr", &fcr, &[("1503", "1")]);
    // Upper-case MD5 text: CPS compares strings exactly.
    let zip = zip_of(&[
        ("readme.xml", xml.clone().into_bytes()),
        ("readme.md5", md5_hex(xml.as_bytes()).to_uppercase().into_bytes()),
        ("TEST0001.fcr", fcr.clone()),
    ]);
    let r = check_package(&zip, Some(&radio("BP515 U1", BP515_U1))).unwrap();
    assert!(refused(&r).iter().any(|t| t.contains("upper case")), "{:?}", r.checks);
    // A changed .fcr no longer matches the readme.
    let zip = zip_of(&[
        ("readme.xml", xml.clone().into_bytes()),
        ("readme.md5", format!("{}\r\n", md5_hex(xml.as_bytes())).into_bytes()),
        ("TEST0001.fcr", b"abd".to_vec()),
    ]);
    let r = check_package(&zip, Some(&radio("BP515 U1", BP515_U1))).unwrap();
    let refused = refused(&r);
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].contains("fcr MD5"));
}

#[test]
fn files_in_subfolders_are_not_found() {
    let fcr = b"abc".to_vec();
    let xml = readme("TEST0001", "TEST0001.fcr", &fcr, &[("1503", "1")]);
    let zip = zip_of(&[
        ("lic/readme.xml", xml.clone().into_bytes()),
        ("lic/readme.md5", md5_hex(xml.as_bytes()).into_bytes()),
        ("lic/TEST0001.fcr", fcr),
    ]);
    let r = check_package(&zip, None).unwrap();
    assert!(refused(&r)[0].contains("subfolders"), "{:?}", r.checks);
}

#[test]
fn conflicting_features_are_refused() {
    let zip = package("TEST0001", &[("1500", "1"), ("1507", "1")]);
    let r = check_package(&zip, Some(&radio("BP515 U1", BP515_U1))).unwrap();
    assert!(refused(&r).iter().any(|t| t.contains("together")), "{:?}", r.checks);
}

#[test]
fn without_a_radio_every_package_is_listed() {
    let zip = package("TEST0001", &[("1509", "1")]);
    let r = check_package(&zip, None).unwrap();
    assert!(!r.accepted());
    assert!(refused(&r).is_empty(), "{:?}", r.checks);
    assert_eq!(r.packages[0].objdevice, "TEST0001");
    assert_eq!(r.packages[0].features[0].name, Some("Basic Encryption"));
}

#[test]
fn installed_licence_answer() {
    assert!(installed_licences(&[0u8; 64]).codes.is_empty());
    let mut v: Vec<u8> = "1503;1509".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    v.resize(64, 0);
    let l = installed_licences(&v);
    assert_eq!(l.codes, ["1503", "1509"]);
    assert!(!l.unrecognised);
    assert!(installed_licences(&[0xff, 0x01, 0x80]).unrecognised);
}
