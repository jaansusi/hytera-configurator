//! Hytera licences: the radio's installed licences (identify item `0x0200`) and offline checks of a
//! licence package (`.zip`) as CPS's "Authorization" button does them.
//!
//! Source: CPS V1.5.24.000.0003, `plug\G2CPS.ParamTemplateBiz.dll` (`DeviceItemVM.Authorization`,
//! `MatchAuthorizationItems`, `syncLicenseFeature`) and `G2CPS.Protocol.dll`
//! (`BaseInforItem.LicenseFeature = 512`); see `docs/research-cps-visibility.md`. Nothing here talks
//! to the radio or looks inside the `.fcr` file (Hytera's per-radio licence); only its size and
//! MD5 are checked, like CPS does.

use std::io::{Cursor, Read};

use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use serde::Serialize;

use crate::ReadManifest;

/// Identify item with the radio's installed licences (`BaseInforItem.LicenseFeature`).
pub const LICENCE_INFO_ITEM: u16 = 0x0200;

/// CPS's licence codes (`syncLicenseFeature`).
pub const LICENCE_CODES: &[(&str, &str)] = &[
    ("1500", "Pure Digital"),
    ("1501", "Pure Analog"),
    ("1502", "Priority Interrupt"),
    ("1503", "Roam"),
    ("1504", "Remote Monitor"),
    ("1505", "Radio Disable Enable"),
    ("1506", "HDC1200"),
    ("1507", "Two Tone"),
    ("1508", "Five Tone"),
    ("1509", "Basic Encryption"),
];

pub fn licence_name(code: &str) -> Option<&'static str> {
    LICENCE_CODES.iter().find(|(c, _)| *c == code.trim()).map(|(_, n)| *n)
}

fn describe(code: &str) -> String {
    match licence_name(code) {
        Some(n) => format!("{n} ({code})"),
        None => format!("unknown code {code}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Installed licences (identify item 0x0200)

#[derive(Debug, Clone, Serialize)]
pub struct InstalledLicences {
    /// The radio's raw answer, hex.
    pub raw_hex: String,
    /// Licence codes found. Empty when the answer is all zero (no licence installed).
    pub codes: Vec<String>,
    /// Set when the answer is neither all zero nor `;`-separated codes as text.
    pub unrecognised: bool,
}

/// Decode the answer to item `0x0200`. All zero = no licence (observed on the BP515). With
/// licences, the format is *inferred*: CPS splits `LicenseFeature` on `;` and matches codes, so
/// the text is taken as UTF-16BE (like the other items) or, failing that, ASCII.
pub fn installed_licences(value: &[u8]) -> InstalledLicences {
    let raw_hex = crate::hex(value);
    if value.iter().all(|&b| b == 0) {
        return InstalledLicences { raw_hex, codes: Vec::new(), unrecognised: false };
    }
    let utf16 = value.len().is_multiple_of(2)
        .then(|| {
            let units: Vec<u16> = value.chunks(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
            String::from_utf16(&units).ok()
        })
        .flatten();
    let ascii = value.is_ascii().then(|| String::from_utf8_lossy(value).into_owned());
    for text in [utf16, ascii].into_iter().flatten() {
        let text = text.trim_matches(char::from(0)).trim();
        let codes: Vec<String> =
            text.split(';').map(|s| s.trim_matches(char::from(0)).trim().to_string()).filter(|s| !s.is_empty()).collect();
        if !codes.is_empty() && codes.iter().all(|c| c.chars().all(|ch| ch.is_ascii_digit())) {
            return InstalledLicences { raw_hex, codes, unrecognised: false };
        }
    }
    InstalledLicences { raw_hex, codes: Vec::new(), unrecognised: true }
}

/// [`installed_licences`] from a saved read's identify answers, if item `0x0200` was answered.
pub fn installed_from_manifest(manifest: &ReadManifest) -> Option<InstalledLicences> {
    let entry = manifest.info.iter().find(|i| i.item == format!("0x{LICENCE_INFO_ITEM:04X}") && i.status == 0)?;
    Some(installed_licences(&unhex(&entry.hex)?))
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

// ---------------------------------------------------------------------------------------------
// Licence package check

/// What CPS knows about the connected radio when it imports a package.
#[derive(Debug, Clone, Serialize)]
pub struct RadioFacts {
    /// Item `0x000B`, compared with the package's `objdevice`.
    pub serial: String,
    /// Item `0x0011`, e.g. `BP515 U1`.
    pub model_name: String,
    /// Item `0x0000`, e.g. `55P10B00-1L0BAPU1U1…`.
    pub model_number: String,
}

impl RadioFacts {
    pub fn from_manifest(manifest: &ReadManifest) -> Result<Self> {
        Ok(Self {
            serial: manifest.serial_number.clone().context("the read has no serial number")?,
            model_name: manifest.model_name.clone().context("the read has no model name")?,
            model_number: crate::model_number(manifest).context("the read has no model number")?.to_string(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Level {
    Ok,
    Note,
    /// CPS would stop here.
    Refused,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ZipEntry {
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageFeature {
    pub code: String,
    pub name: Option<&'static str>,
    pub value: String,
    /// `value` is not "0" (CPS: `IsChecked`).
    pub licensed: bool,
}

/// One `license/filelist/package` element of the readme XML.
#[derive(Debug, Clone, Serialize)]
pub struct Package {
    pub objdevice: String,
    pub file: String,
    pub md5: String,
    pub features: Vec<PackageFeature>,
    /// Size of the named `.fcr` file, if the zip has it at the top level.
    pub fcr_size: Option<u64>,
    /// MD5 of that file equals `md5` (exact text, as CPS compares).
    pub fcr_md5_ok: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageReport {
    pub entries: Vec<ZipEntry>,
    pub readme: Option<String>,
    pub md5_file: Option<String>,
    pub packages: Vec<Package>,
    pub checks: Vec<Check>,
    /// The radio checked against, if any.
    pub radio: Option<RadioFacts>,
}

impl PackageReport {
    /// Whether CPS V1.5.24 would get as far as sending the licence to the radio.
    pub fn accepted(&self) -> bool {
        self.radio.is_some() && !self.checks.iter().any(|c| c.level == Level::Refused)
    }
}

fn md5_hex(bytes: &[u8]) -> String {
    crate::hex(&Md5::digest(bytes))
}

/// Codes CPS V1.5.24 refuses for a radio (`<Authorization>b__e`, IL 0x047A–0x0803). Basic Encryption
/// (1509) is refused in every branch. Always `Some`; the `Option` is kept for later CPS versions.
pub fn refused_codes(model_name: &str, model_number: &str) -> Option<Vec<&'static str>> {
    let lower = model_name.to_lowercase();
    if model_number.get(14..15) == Some("0") {
        let mut codes = vec!["1500", "1501", "1502", "1503", "1504", "1505", "1506", "1508", "1509"];
        if model_name.starts_with("ap32") {
            codes.push("1507");
        }
        return Some(codes);
    }
    let k_model = ["bp568k", "bp578k", "bp578 dig"].iter().any(|p| lower.starts_with(p))
        || model_number.get(38..41) == Some("051");
    if model_number != "N/A" && k_model {
        let all = ["1500", "1501", "1502", "1503", "1504", "1505", "1506", "1507", "1508", "1509"];
        if lower.starts_with("bp578k") || lower.starts_with("bp578 dig") {
            return Some(all.to_vec());
        }
        return Some(all.into_iter().filter(|c| *c != "1503").collect());
    }
    let mut codes = vec!["1502", "1504", "1505", "1506", "1508", "1509"];
    if lower.starts_with("bp36") {
        codes.extend(["1503", "1507"]);
    } else if lower.starts_with("bp515 lf") {
        codes.push("1503");
    }
    Some(codes)
}

/// Check a licence package the way CPS's Authorization does, without a radio. With `radio`, the
/// serial and model rules are checked too; without it, every package in the readme is listed.
pub fn check_package(zip_bytes: &[u8], radio: Option<&RadioFacts>) -> Result<PackageReport> {
    let mut zip = zip::ZipArchive::new(Cursor::new(zip_bytes)).context("not a readable zip file")?;
    let mut entries = Vec::new();
    let mut top: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().context("reading a file name in the zip")?.to_string();
        entries.push(ZipEntry { name: name.clone(), size: f.size() });
        let mut data = Vec::new();
        f.read_to_end(&mut data).with_context(|| format!("unpacking {name}"))?;
        // CPS unzips to a temp folder and lists only its top level (Directory.GetFiles).
        if !name.contains('/') && !name.contains('\\') {
            top.push((name, data));
        }
    }
    top.sort_by_key(|(n, _)| n.to_lowercase());

    let mut checks = Vec::new();
    let mut report = PackageReport {
        entries,
        readme: None,
        md5_file: None,
        packages: Vec::new(),
        checks: Vec::new(),
        radio: radio.cloned(),
    };
    let ends = |n: &str, ext: &str| n.to_lowercase().ends_with(ext);
    let readme = top.iter().find(|(n, _)| ends(n, ".xml"));
    let fcrs: Vec<&(String, Vec<u8>)> = top.iter().filter(|(n, _)| ends(n, ".fcr")).collect();
    let md5_file = top.iter().find(|(n, _)| ends(n, ".md5"));
    for (what, ext) in [("readme", ".xml"), ("checksum", ".md5")] {
        let n = top.iter().filter(|(n, _)| ends(n, ext)).count();
        if n > 1 {
            checks.push(Check {
                level: Level::Note,
                text: format!("{n} {ext} files at the top level; CPS takes the first one it lists as the {what}"),
            });
        }
    }
    let nested = report.entries.iter().any(|e| e.name.contains('/') || e.name.contains('\\'));
    let missing = |what: &str| Check {
        level: Level::Refused,
        text: format!(
            "no {what} file at the top level of the zip (CPS: \"FileNotExit\"){}",
            if nested { "; the zip has subfolders, which CPS does not look into" } else { "" }
        ),
    };
    let Some((readme_name, readme_bytes)) = readme else {
        checks.push(missing(".xml readme"));
        report.checks = checks;
        return Ok(report);
    };
    report.readme = Some(readme_name.clone());
    if fcrs.is_empty() {
        checks.push(missing(".fcr licence"));
    }
    match md5_file {
        None => checks.push(missing(".md5 checksum")),
        Some((name, bytes)) => {
            report.md5_file = Some(name.clone());
            let expected = String::from_utf8_lossy(bytes).trim().trim_start_matches('\u{feff}').to_string();
            let actual = md5_hex(readme_bytes);
            checks.push(if expected == actual {
                Check { level: Level::Ok, text: format!("{name} matches the MD5 of {readme_name}") }
            } else if expected.eq_ignore_ascii_case(&actual) {
                Check {
                    level: Level::Refused,
                    text: format!("{name} has the right MD5 of {readme_name} but in upper case; CPS compares lower-case text exactly"),
                }
            } else {
                Check {
                    level: Level::Refused,
                    text: format!("MD5 of {readme_name} is {actual}, {name} says {expected} (CPS: \"readme.xml MD5 verification failed\")"),
                }
            });
        }
    }

    let text = String::from_utf8_lossy(readme_bytes);
    let text = text.trim_start_matches('\u{feff}');
    let doc = match roxmltree::Document::parse(text) {
        Ok(d) => d,
        Err(e) => {
            checks.push(Check { level: Level::Refused, text: format!("{readme_name} is not valid XML: {e}") });
            report.checks = checks;
            return Ok(report);
        }
    };
    let root = doc.root_element();
    if !root.has_tag_name("license") {
        checks.push(Check {
            level: Level::Refused,
            text: format!("{readme_name}: root element is <{}>, CPS looks for license/filelist/package", root.tag_name().name()),
        });
    }
    let packages: Vec<roxmltree::Node> = if root.has_tag_name("license") {
        root.children()
            .filter(|c| c.has_tag_name("filelist"))
            .flat_map(|f| f.children().filter(|c| c.has_tag_name("package")))
            .collect()
    } else {
        Vec::new()
    };
    for p in &packages {
        let file = inner(child(*p, "file"));
        let md5 = inner(child(*p, "md5"));
        let features = child(*p, "detail")
            .map(|d| {
                d.children()
                    .filter(|c| c.is_element())
                    .map(|item| {
                        let code = inner(child(item, "code"));
                        let value = inner(child(item, "value"));
                        PackageFeature { name: licence_name(&code), licensed: value != "0", code, value }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let fcr = fcrs.iter().find(|(n, _)| n.trim() == file);
        report.packages.push(Package {
            objdevice: inner(child(*p, "objdevice")),
            fcr_size: fcr.map(|(_, b)| b.len() as u64),
            fcr_md5_ok: fcr.map(|(_, b)| md5_hex(b) == md5),
            file,
            md5,
            features,
        });
    }
    if packages.is_empty() && root.has_tag_name("license") {
        checks.push(Check { level: Level::Refused, text: format!("{readme_name} has no license/filelist/package entry") });
    }

    let Some(radio) = radio else {
        for p in &report.packages {
            checks.extend(package_checks(p));
        }
        checks.push(Check {
            level: Level::Note,
            text: "no radio given (--radio <read folder>): serial and model rules not checked".into(),
        });
        report.checks = checks;
        return Ok(report);
    };

    // CPS keeps the last package whose objdevice equals the radio's serial, and collects the
    // features of every matching package.
    let matching: Vec<&Package> = report.packages.iter().filter(|p| p.objdevice == radio.serial.trim()).collect();
    let Some(chosen) = matching.last() else {
        let serials: Vec<&str> = report.packages.iter().map(|p| p.objdevice.as_str()).collect();
        checks.push(Check {
            level: Level::Refused,
            text: format!(
                "no package for serial {} (the readme has: {}) (CPS: \"DeviceFileNotExists\")",
                radio.serial,
                if serials.is_empty() { "none".into() } else { serials.join(", ") }
            ),
        });
        report.checks = checks;
        return Ok(report);
    };
    checks.push(Check { level: Level::Ok, text: format!("package for serial {} found", radio.serial) });
    checks.extend(package_checks(chosen));

    let licensed: Vec<&PackageFeature> = matching.iter().flat_map(|p| &p.features).filter(|f| f.licensed).collect();
    if licensed.is_empty() {
        checks.push(Check {
            level: Level::Note,
            text: "no feature in the package is licensed (all values are 0); CPS's \"NoAvailableLicense\" check never fires, so it would still send it".into(),
        });
    }
    let has = |code: &str| licensed.iter().any(|f| f.code == code);
    for (a, b) in [("1500", "1507"), ("1501", "1503")] {
        if has(a) && has(b) {
            checks.push(Check {
                level: Level::Refused,
                text: format!("{} and {} together are refused (CPS: \"ErrorLicense\")", describe(a), describe(b)),
            });
        }
    }
    match refused_codes(&radio.model_name, &radio.model_number) {
        None => checks.push(Check {
            level: Level::Note,
            text: format!("CPS's model rule for {} was not decoded reliably; not checked", radio.model_name),
        }),
        Some(refused) => {
            let bad: Vec<String> =
                licensed.iter().filter(|f| refused.contains(&f.code.as_str())).map(|f| describe(&f.code)).collect();
            checks.push(if bad.is_empty() {
                Check {
                    level: Level::Ok,
                    text: format!("CPS V1.5.24 allows every licensed feature for {}", radio.model_name),
                }
            } else {
                Check {
                    level: Level::Refused,
                    text: format!(
                        "CPS V1.5.24 refuses for {}: {} (\"Mismatched license features exist in the file\"); \
                         a newer CPS may accept it",
                        radio.model_name,
                        bad.join(", ")
                    ),
                }
            });
        }
    }
    report.checks = checks;
    Ok(report)
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(name))
}

/// Trimmed text of an element (XmlNode.InnerText.Trim()), empty if missing.
fn inner(n: Option<roxmltree::Node>) -> String {
    n.map(|n| n.descendants().filter(|d| d.is_text()).filter_map(|d| d.text()).collect::<String>())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn package_checks(p: &Package) -> Vec<Check> {
    let mut out = Vec::new();
    let who = format!("package for {}", if p.objdevice.is_empty() { "(no serial)" } else { &p.objdevice });
    if p.objdevice.is_empty() || p.file.is_empty() {
        out.push(Check { level: Level::Refused, text: format!("{who}: objdevice or file is empty (CPS: \"DeviceFileNotExists\")") });
        return out;
    }
    match (p.fcr_size, p.fcr_md5_ok) {
        (None, _) => out.push(Check {
            level: Level::Refused,
            text: format!("{who}: {} is not at the top level of the zip (CPS: \"FcrNameVerificationFailed\")", p.file),
        }),
        (Some(size), Some(true)) => {
            out.push(Check { level: Level::Ok, text: format!("{who}: {} ({size} bytes) matches its MD5", p.file) })
        }
        (Some(_), _) => out.push(Check {
            level: Level::Refused,
            text: format!("{who}: MD5 of {} does not equal {} (CPS: \"fcr MD5 verification failed\")", p.file, p.md5),
        }),
    }
    for f in p.features.iter().filter(|f| f.name.is_none()) {
        out.push(Check { level: Level::Note, text: format!("{who}: feature code {} is not one CPS knows", f.code) });
    }
    out
}

/// Read a package from disk and [`check_package`] it.
pub fn check_package_file(path: &std::path::Path, radio: Option<&RadioFacts>) -> Result<PackageReport> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if bytes.is_empty() {
        bail!("{} is empty", path.display());
    }
    check_package(&bytes, radio)
}
