//! CSV export/import of tables on the radio's configuration (captures/unattended_2026-11-19/start).
//! Skipped when the private captures, the CPS install or the option catalogue are missing.

use std::path::{Path, PathBuf};

use hyt_core::sheet::{export, import};
use hyt_core::tables::table;
use hyt_core::workspace::Workspace;
use hytera_codeplug::options::Options;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot};

fn open() -> Option<Workspace> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../captures/unattended_2026-11-19/start");
    let dir = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    if !dir.is_dir() || !folder.join("manifest.json").is_file() || !Options::default_path("V1.5.24.000")?.is_file() {
        eprintln!("skipping: captures, CPS RCDB data or option catalogue not present");
        return None;
    }
    Some(Workspace::open(&folder, &RcdbRoot::new(dir), None).unwrap())
}

fn rows(csv: &str) -> Vec<Vec<String>> {
    let mut r = csv::ReaderBuilder::new().has_headers(false).from_reader(csv.as_bytes());
    r.records().map(|x| x.unwrap().iter().map(str::to_string).collect()).collect()
}

fn to_csv(rows: &[Vec<String>]) -> String {
    let mut w = csv::Writer::from_writer(Vec::new());
    for r in rows {
        w.write_record(r).unwrap();
    }
    String::from_utf8(w.into_inner().unwrap()).unwrap()
}

#[test]
fn exported_tables_import_back_unchanged() {
    let Some(mut ws) = open() else { return };
    for id in ["digital", "analog", "zones", "contacts", "list:QEU_C_ScanConfig"] {
        let text = export(&ws, id).unwrap();
        let r = rows(&text);
        assert!(r[0].contains(&"RX Frequency [MHz]".to_string()) || id != "digital", "{:?}", r[0]);
        let report = import(&mut ws, id, &text).unwrap();
        assert_eq!((report.changed, report.added), (0, 0), "{id}: {:?}", report.errors);
        assert!(report.errors.is_empty(), "{id}: {:?}", report.errors);
    }
    assert!(!ws.has_changes());
}

#[test]
fn bulk_edits_and_new_channels_come_from_the_sheet() {
    let Some(mut ws) = open() else { return };
    let mut r = rows(&export(&ws, "digital").unwrap());
    let col = |name: &str| r[0].iter().position(|h| h == name).unwrap();
    let (alias, rx, cc) = (col("Channel Alias"), col("RX Frequency [MHz]"), col("Color Code"));
    r[2][alias] = "Sheet 2".into();
    r[2][cc] = "16".into(); // invalid: reported, the rest of the row still applies
    // A new channel: copy row 1, renumber, rename and retune.
    let mut new = r[1].clone();
    new[0] = String::new();
    new[alias] = "Sheet 3".into();
    new[rx] = "438.5".into();
    r.push(new);
    let report = import(&mut ws, "digital", &to_csv(&r)).unwrap();
    assert_eq!(report.added, 1);
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert!(report.errors[0].contains("Color Code"), "{:?}", report.errors);
    let t = table(&ws, "digital").unwrap();
    let names: Vec<&str> = t.rows.iter().map(|row| row.cells[0].text.as_str()).collect();
    assert_eq!(names, ["HYT Test 1", "Sheet 2", "Sheet 3"]);
    assert_eq!(t.rows[2].cells[1].text, "438.5");

    // Rows for records that cannot be added are refused.
    let mut c = rows(&export(&ws, "contacts").unwrap());
    let mut extra = c[1].clone();
    extra[0] = "2".into();
    c.push(extra);
    let err = import(&mut ws, "contacts", &to_csv(&c)).unwrap_err();
    assert!(format!("{err:#}").contains("needs a CPS recording"), "{err:#}");
}
