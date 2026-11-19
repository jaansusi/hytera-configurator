//! CPS template (`.tpl`) import and export, on the owner's CPS templates and the radio's read.
//! Skipped when CPS's `Templates` folder, the CPS RCDB data or the private captures are missing.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use hyt_core::tpl::{self, ExportOptions, ParamStore, Template, TemplateHeader};
use hyt_core::{SavedRead, decode_read, load_read};
use hytera_codeplug::container::prepare_for_write;
use hytera_codeplug::layout::Layout;
use hytera_codeplug::rcdb::{DEFAULT_CPS_DATA_DIR, RcdbRoot, TableLink};

struct Fx {
    templates: Vec<PathBuf>,
    like: SavedRead,
    like_dir: PathBuf,
    layout: Layout,
    links: Vec<TableLink>,
}

fn fixture() -> Option<Fx> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cps = std::env::var_os("HYT_RCDB").map(PathBuf::from).unwrap_or(DEFAULT_CPS_DATA_DIR.into());
    let tpl_dir = std::env::var_os("HYT_TPL_DIR").map(PathBuf::from).unwrap_or(Path::new(DEFAULT_CPS_DATA_DIR).join("Templates"));
    let like_dir = root.join("captures/unattended_2026-11-19/start");
    let mut templates: Vec<PathBuf> = std::fs::read_dir(&tpl_dir)
        .ok()
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "tpl")).collect())
        .unwrap_or_default();
    templates.sort();
    if templates.is_empty() || !like_dir.is_dir() || !cps.is_dir() {
        eprintln!("skipping: CPS templates, CPS RCDB data or captures not present");
        return None;
    }
    let rcdb = RcdbRoot::new(cps);
    let like = load_read(&like_dir).ok()?;
    let layout = rcdb.layout(&like.manifest.rcdb_version).ok()?;
    let links = rcdb.table_links().ok()?;
    Some(Fx { templates, like, like_dir, layout, links })
}

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("hyt-tpl-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    let _ = std::fs::remove_file(&p);
    p
}

/// All entries of a ZIP archive with their DOS time, in order.
fn raw_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>, String)> {
    let mut a = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..a.len())
        .map(|i| {
            let mut f = a.by_index(i).unwrap();
            let name = f.name().unwrap().to_string();
            let time = f.last_modified().map(|t| t.to_string()).unwrap_or_default();
            let mut d = Vec::new();
            f.read_to_end(&mut d).unwrap();
            (name, d, time)
        })
        .collect()
}

#[test]
fn headers_rewrite_byte_for_byte() {
    let Some(fx) = fixture() else { return };
    for path in &fx.templates {
        let bytes = std::fs::read(path).unwrap();
        for (name, data, _) in raw_entries(&bytes) {
            if name == tpl::TEMPLATE_HEADER {
                assert_eq!(TemplateHeader::parse(&data).unwrap().to_xml(), data, "{}: tpHeader", path.display());
            } else {
                for (inner, d, _) in raw_entries(&data) {
                    if inner == tpl::STORE_HEADER {
                        assert_eq!(ParamStore::parse(&d).unwrap().to_xml(), d, "{}: {name}", path.display());
                    }
                }
            }
        }
    }
}

#[test]
fn import_each_template_and_decode() {
    let Some(fx) = fixture() else { return };
    for (i, path) in fx.templates.iter().enumerate() {
        let t = Template::read(path).unwrap();
        assert!(t.warnings().is_empty(), "{}: {:?}", path.display(), t.warnings());
        let out = temp(&format!("import-{i}"));
        let report = tpl::import(&t, &fx.like, &out, "test").unwrap();
        assert_eq!(report.from_template, fx.like.manifest.files.len(), "{}", path.display());
        assert!(report.kept_from_reference.is_empty() && report.ignored.is_empty());
        let saved = load_read(&out).unwrap();
        // The reference radio's identity is kept, so `hyt write` accepts the folder for it.
        let (a, b) = (&saved.manifest, &fx.like.manifest);
        assert_eq!(
            (&a.profile, &a.model_name, &a.serial_number, &a.firmware_version, &a.rcdb_version),
            (&b.profile, &b.model_name, &b.serial_number, &b.firmware_version, &b.rcdb_version)
        );
        assert_eq!(serde_json::to_value(&a.info).unwrap(), serde_json::to_value(&b.info).unwrap());
        for f in t.files().unwrap() {
            assert_eq!(saved.files[&f.id], f.bytes, "file {}", f.name);
            // Template files are already in the form CPS writes.
            assert_eq!(prepare_for_write(&f.bytes).unwrap(), f.bytes, "file {} is not in written form", f.name);
        }
        let decoded = decode_read(&saved, &fx.layout).unwrap();
        assert_eq!(decoded.len(), 58);
        std::fs::remove_dir_all(&out).unwrap();
    }
}

#[test]
fn export_of_import_reproduces_the_template() {
    let Some(fx) = fixture() else { return };
    for (i, path) in fx.templates.iter().enumerate() {
        let original_bytes = std::fs::read(path).unwrap();
        let original = Template::parse(&original_bytes).unwrap();
        let folder = temp(&format!("rt-{i}"));
        tpl::import(&original, &fx.like, &folder, "test").unwrap();
        let out = temp(&format!("rt-{i}.tpl"));
        let opts = ExportOptions {
            name: None,
            default_name: "unused".into(),
            description: None,
            sidecar: None,
            seqo_from: None,
            when: tpl::now(),
        };
        let (_, report) = tpl::export(&folder, &out, &fx.links, opts).unwrap();
        assert_eq!(report.headers_kept, 58);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let exported_bytes = std::fs::read(&out).unwrap();
        let exported = Template::parse(&exported_bytes).unwrap();

        // Same entries in the same order, same inner entries, same radio files, same stores.
        let names = |t: &Template| t.entries.iter().map(|e| (e.name.clone(), e.files.iter().map(|f| f.0.clone()).collect::<Vec<_>>())).collect::<Vec<_>>();
        assert_eq!(names(&exported), names(&original));
        for (a, b) in exported.entries.iter().zip(&original.entries) {
            assert_eq!(a.files, b.files, "{}", a.name);
            assert_eq!(a.store, b.store, "{}", a.name);
        }
        // The template header differs only in Guid and CreatedTime (the radio identity in this
        // template is the reference radio's, which is the same radio here).
        let (mut a, b) = (exported.header.clone(), original.header.clone());
        assert_ne!(a.guid, b.guid);
        assert_ne!(a.created_time, b.created_time);
        a.guid.clone_from(&b.guid);
        a.created_time.clone_from(&b.created_time);
        assert_eq!(a, b);

        // At ZIP level only the timestamps and the compressed bytes differ.
        let (ro, re) = (raw_entries(&original_bytes), raw_entries(&exported_bytes));
        assert_eq!(ro.len(), re.len());
        for ((n1, _, t1), (n2, _, t2)) in ro.iter().zip(&re) {
            assert_eq!(n1, n2);
            let _ = (t1, t2);
        }
        eprintln!(
            "{}: {} bytes, exported {} bytes; zip times {} -> {}",
            path.file_name().unwrap().to_string_lossy(),
            original_bytes.len(),
            exported_bytes.len(),
            ro[0].2,
            re[0].2
        );
        std::fs::remove_dir_all(&folder).unwrap();
        std::fs::remove_file(&out).unwrap();
    }
}

#[test]
fn export_of_a_radio_read_has_cps_structure() {
    let Some(fx) = fixture() else { return };
    let reference = Template::read(&fx.templates[0]).unwrap();
    let out = temp("read.tpl");
    let opts = ExportOptions {
        name: Some("hyt test".into()),
        default_name: "unused".into(),
        description: None,
        sidecar: None,
        seqo_from: Some(&reference),
        when: tpl::now(),
    };
    let (_, report) = tpl::export(&fx.like_dir, &out, &fx.links, opts).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let t = Template::read(&out).unwrap();
    std::fs::remove_file(&out).unwrap();
    let names = |t: &Template| t.entries.iter().map(|e| (e.name.clone(), e.files.iter().map(|f| f.0.clone()).collect::<Vec<_>>())).collect::<Vec<_>>();
    assert_eq!(names(&t), names(&reference));
    for (a, b) in t.entries.iter().zip(&reference.entries) {
        assert_eq!((&a.store.seqo, &a.store.key, &a.store.rcdb_version, &a.store.type_, &a.store.model_number),
                   (&b.store.seqo, &b.store.key, &b.store.rcdb_version, &b.store.type_, &b.store.model_number));
        let id: u16 = a.store.key.as_deref().unwrap().parse().unwrap();
        assert_eq!(a.main_file().unwrap(), prepare_for_write(&fx.like.files[&id]).unwrap().as_slice());
    }
    let h = &t.header;
    let r = &reference.header;
    assert_eq!(
        (&h.operate_permission, &h.operate_type, &h.is_create, &h.product, &h.model_number, &h.rcdb_version, &h.frequence, &h.product_id, &h.serial_number),
        (&r.operate_permission, &r.operate_type, &r.is_create, &r.product, &r.model_number, &r.rcdb_version, &r.frequence, &r.product_id, &r.serial_number)
    );
    assert_eq!(h.name.as_deref(), Some("hyt test"));
    assert_eq!(h.description.as_deref(), Some(""));
}

#[test]
fn a_template_for_another_rcdb_version_or_model_is_refused() {
    let Some(fx) = fixture() else { return };
    let base = Template::read(&fx.templates[0]).unwrap();

    let mut other_version = base.clone();
    other_version.header.rcdb_version = Some("V1.5.99.000".into());
    let path = temp("other-version.tpl");
    std::fs::write(&path, other_version.to_bytes(tpl::now()).unwrap()).unwrap();
    let t = Template::read(&path).unwrap();
    let out = temp("other-version");
    let err = tpl::import(&t, &fx.like, &out, "test").unwrap_err().to_string();
    assert!(err.contains("RCDB V1.5.99.000"), "{err}");
    assert!(!out.exists());
    std::fs::remove_file(&path).unwrap();

    let mut store_version = base.clone();
    store_version.entries[3].store.rcdb_version = Some("V1.5.99.000".into());
    let err = tpl::import(&store_version, &fx.like, &out, "test").unwrap_err().to_string();
    assert!(err.contains("V1.5.99.000"), "{err}");
    assert!(!out.exists());

    let mut other_model = base.clone();
    other_model.header.product = Some("PD985 U1".into());
    let err = tpl::import(&other_model, &fx.like, &out, "test").unwrap_err().to_string();
    assert!(err.contains("PD985 U1"), "{err}");
    assert!(!out.exists());
}

#[test]
fn missing_files_are_kept_from_the_reference() {
    let Some(fx) = fixture() else { return };
    let mut t = Template::read(&fx.templates[0]).unwrap();
    t.entries.retain(|e| e.name != "QEU_C_DContact");
    let out = temp("missing");
    let report = tpl::import(&t, &fx.like, &out, "test").unwrap();
    assert_eq!(report.kept_from_reference, vec![(8, "QEU_C_DContact".to_string())]);
    let saved = load_read(&out).unwrap();
    assert_eq!(saved.files[&8], fx.like.files[&8]);
    std::fs::remove_dir_all(&out).unwrap();
}
