# Plan: an independent read/write app for Hytera G2 radios

**Goal:** read a radio's settings, show and edit them, and write them back over the USB programming cable
without Hytera CPS. Personal use, with Hytera's permission. First target: the BP515 U1 on RCDB V1.5.24.000.

**Starting point (2026-11-18):** the read protocol is fully known ([protocol.md](protocol.md)), and the field
layouts ship with CPS ([file-format.md](file-format.md)). The write path, 10 file CRCs and the list-file layout
are still unknown ([open-questions.md](open-questions.md)).

## Approach

- **Rust** (decided): a core library shared by a command-line tool first and a GUI later (Tauri recommended for
  the table-heavy editor; egui as a pure-Rust alternative). One executable, no runtime. Installed via `rustup`
  (Rust 1.100); the MSVC linker comes from Visual Studio Build Tools 2026.
- **Python stays for research only**: the recorder and capture decoder in `tools/`. It is not part of the app.
- **Load layouts from CPS's own RCDB files** (path configurable) instead of copying them into the app. This keeps
  Hytera's data out of the code and lets new firmware versions work when CPS is updated.
- **Test without the radio** by replaying recorded sessions. Live radio tests are opt-in.

### Proposed layout (Cargo workspace)

```
crates/
  hytera-proto/      Transport trait (serial + replay), link frames + Internet checksum + acks + retries,
                     0x0A0B application commands, session flows: identify / read_all / write_files
  hytera-codeplug/   file container + CRC rules, RCDB loader (FileID XML, RcdbJsons, TableLink),
                     codec for fixed records, bit fields, strings and list files
  hytera-models/     model registry and per-model profiles (see "Supporting more radios")
  hyt-core/          operations shared by CLI and GUI: read/save/load folders, decode, edit (apply_edits),
                     guarded write flow (write_flow), GUI model (workspace, tables)
  hyt-cli/           hyt ports | info | read | decode | show | apply | diff | write (thin, on hyt-core)
  hyt-gui/           Tauri 2 desktop app: src/ (commands calling hyt-core) + ui/ (static HTML/JS/CSS)
tests/fixtures/      replay sessions converted from captures/
tools/               Python research tooling (existing)
```

Likely crates: `serialport`, `serde`/`serde_json`, `quick-xml`, `crc32fast`, `clap`, `thiserror`, `tracing`.

### Platforms (Windows, macOS, Linux)

All planned crates and both GUI options are cross-platform, so one codebase targets all three.

- **Serial ports:** Windows `COMn` (Prolific driver); Linux `/dev/ttyUSBn` (kernel `pl2303` driver, user needs the
  `dialout` group or a udev rule; `serialport` uses `libudev` for port listing by default); macOS
  `/dev/cu.usbserial-…` (Prolific's macOS driver may be needed for the PL2303GC; not verified). Port names are
  never hard-coded; the CLI lists ports and takes `--port`.
- **Layout data:** CPS (and so the RCDB files) exists only on Windows, and the app must not bundle Hytera's files.
  Add `hyt rcdb import <CPS folder>`, which copies `RcdbJsons\`, `RCDB\FileID\` and `RCDB\TableLink\` into the
  app's per-user data folder. Users on macOS/Linux import from a copy taken from a Windows CPS install.
- **Builds:** each platform builds natively. Use GitHub Actions (Windows, macOS, Linux runners) for release
  binaries and Tauri installers; locally, Windows natively and Linux via WSL. macOS builds need a Mac or CI
  (Apple SDK and code signing).
- **Testing:** replay tests run on every platform in CI; live radio tests so far only on Windows.

### Supporting more radios

The BP515 is the only target for now, but the design keeps model specifics in one place:

- **Identify first.** Every session starts by reading info items: model name (17), model code (0) and RCDB
  version (2). A registry maps these to a **model profile**.
- **Profiles are mostly data.** A profile names the model, the RCDB versions known to work, which files are
  read and which are written, CRC rules per file, and any protocol quirks (chunk size, baud rate, timing).
  Everything shared by the G2/Auctus platform (framing, commands, container, RCDB codec) lives in the common crates.
- **Unknown model or RCDB version:** reading is allowed (to collect data), writing is refused.
- Adding a model = a recording from that radio + a new profile + replay fixtures. No changes to the core
  unless the recording shows a protocol difference.

## Phases

Each phase ends with a check that can be shown to pass.

### Phase 1: read-only core (no new research needed)

**Status 2026-11-18: done.** `hyt ports | info | read` implemented; the offline replay check passes; a live read
of the BP515 on COM3 produced 58 files byte-identical to CPS's read.

Build `transport`, `link`, `app`, `session` and `hyt info` / `hyt read --port COM3 --out <dir>`.
- Offline: a replay test feeds `captures/read_2026-11-18.jsonl` to the library through a fake serial port and
  gets the same 58 files.
- Live: `hyt read` on the radio produces files byte-identical to the CPS read, apart from logs that may have
  changed since.

### Phase 2: decoding

**Status 2026-11-18: done.** Structure decoded; `hyt decode` and `hyt show`; exact round trip for all 58 files;
option names, units and scaling from CPS's parameter database. The same database describes CPS's screens
(pages and groups), which phase 5's GUI can reuse.

Build `rcdb`, `container` and `codec`; add `hyt decode <dir> --format json`.
- Fixed-record files decode into named fields.
- Resolve list-file layout (open question 4), then decode channels, zones and contacts.
- Round trip: decode → encode gives byte-identical files for all 58 captured files.
- Spot-check decoded values (channel names, frequencies, zone membership) against what CPS shows (question 5).

### Phase 3: write-path research (needs recordings in CPS)

**Status 2026-11-18: done** for the unchanged write (steps 1, 2, 4). Step 3 (one small change) is still useful for
list encoding and will be needed in phase 2/5.

1. Save a backup of the radio from CPS itself (File → Save) before anything else. CPS can always restore it.
2. Record CPS writing the configuration back **unchanged** → write commands, order, which files, commit step.
3. Record CPS writing with **one small change** (e.g. rename one channel) → confirms list encoding and CRC rules.
4. Update the docs and `parse_capture.py`; close questions 1–3.

### Phase 4: writing, with safety rails

**Status 2026-11-18: done** (except `hyt diff`). The replay test reproduces CPS's write byte for byte, and the
first live write (unchanged configuration) read back identically for all 55 files. The radio is briefly unavailable
after a write, so `hyt write` now waits and retries before verifying.

Add `hyt write` and `hyt diff`.
- Refuse to write unless model, serial number and RCDB version match the backup being written.
- Always take a fresh full read (backup) right before writing.
- Write only the files CPS writes; recompute CRCs with the confirmed rule; refuse any file whose CRC rule is unknown.
- Read back after writing and compare.
- First live test: write back an unchanged backup (exactly what CPS did in step 2). Then a single-field change.

### Phase 5: editing

**Status 2026-11-18:** step 1 (values in place) implemented: `hyt decode --labels`, `hyt apply` (label → raw,
validation of changed values against CPS's choices, ranges and step grid; text length; structure unchanged),
`hyt diff`. Tested on the real configuration (`tests/edit_values.rs`): unchanged labelled JSON applies with zero
changes; an edit changes exactly the requested fields; invalid values are refused. **Written to the radio**
2026-11-18 (channel 1 renamed, verified by read-back). Step 2: adding records and adding/removing zone members
implemented from CPS's recording and reproduces CPS byte for byte (`tests/add_records.rs`); written live
2026-11-18 (channel `HYT Test 4` + zone member). Removing channels implemented from CPS's recording (zones
updated like CPS; byte-for-byte, `tests/delete_channel.rs`) and done live 2026-11-19; other references and other
record kinds are refused until recorded. Remaining for phase 5: GUI (below).

- Export settings to an editable text format (JSON or YAML, plus CSV for channels/contacts), validate against
  RCDB ranges, show a diff, write.
- Then a GUI, either a local web page served by the app or a desktop window (decision below).

#### GUI (`hyt-gui`, Tauri 2; started 2026-11-19)

Logic stays in `hyt-core` (tested there); the app is a thin Tauri shell plus a static page in
`crates/hyt-gui/ui/` (plain HTML/JS/CSS, no npm build: `cargo build -p hyt-gui` is the whole build, and the
Tauri CLI is not needed). Hytera's data is loaded like the CLI (`HYT_RCDB` / CPS install, option catalogue in
the per-user data folder); nothing of Hytera's is bundled.

Screen: toolbar (Open folder, Read radio, Save as, Write to radio; radio identity and folder on the right),
navigation (Digital channels, Analog channels, Zones, Contacts with `count/capacity`, Changes with a badge),
a table of the selected list (CPS's labels and units; double-click a cell to edit; changed cells highlighted),
a detail panel with every field of the selected record (passwords and reserved bytes left out), and a status
line for validation errors and progress.

- **Milestone 1** (values in place; built 2026-11-19, live read and a live write (channel rename) verified on the BP515 the same day): open a saved folder or read the radio; tables for channels, zones (members
  listed by channel alias, like CPS) and contacts; edit values with `hyt apply`'s validation (every edit runs
  `apply_edits` on the whole working copy; a rejected edit is not kept); change list with revert / discard;
  Save as (new folder, same layout as `hyt read`); Write to radio through `write_flow` (the same code as
  `hyt write`: fresh backup → same-radio check → write → wait for restart → read-back verify) after a
  confirmation dialog that lists the changes, the port and the backup folder, and needs a ticked checkbox.
- **Milestone 2** (built 2026-11-19, offline only so far): Add channel / Remove channel on the channel tables
  (new channels from CPS's template; removal updates zones like CPS and is refused when another reference would
  change); zone panel with ↑ ↓ ✕ per member and "Add to zone"; reference fields (TX contact, RX group list,
  scan/roam list, analog scan list and emergency system) shown and chosen by name; added rows marked, change list
  names added/removed records and zone members. `Workspace` now keeps the edit form `apply_edits` takes (original
  indexes, opened folder's channel numbering) and the decoded result it shows; positions match between them.
  Adding/removing zones and contacts waits for CPS recordings (open question 3).
- **List editors** (built 2026-11-20, offline only): a "Lists" section in the navigation with RX group, scan
  and roam lists, one-touch calls, preset channels, quick messages, phone contacts/systems, HDC1200
  systems/contacts and 2-Tone systems/contacts/personalities (only lists the radio has; files the profile never
  writes, e.g. the E2EE key list, are left out). Values are edited through `Workspace::set_value` like the other
  tables; members of scan/roam/RX group lists are shown read-only by name in the detail panel. Adding/removing
  records or members is not offered (no CPS recording yet). Library: `hyt-core/src/lists.rs`; test
  `tests/list_editors.rs`.
- **CPS's references and link rules** (built 2026-11-20, offline only): reference fields come from CPS's
  `DataValidationConfig.config` / `ComplexRuleConfig.Config` at run time (adds phone system, digital emergency
  system, RRS/location revert channel, emergency revert channels, scan priority channels, emergency group call
  ID). CPS's link rules (`tools/export-rcdb-links.ps1` → `links.json`) grey out dependent fields in tables, detail
  and settings (reason as tooltip); `set_value` refuses them and applies values CPS forces. Library:
  `hytera-codeplug/src/{validation,link_rules}.rs`, `hyt-core/src/{refs,link_rules}.rs`; test
  `tests/references.rs`. Owner check: compare a few greyed fields with CPS's screens.
- Later: CPS's page/group structure from `XMLToolCompactData.sdf` for the other settings; `hyt rcdb import`
  for macOS/Linux; installers via `cargo tauri build` in CI.

### Phase 6: other radios
More RCDB versions and models, guided by recordings from those radios.

## Risks and how they are handled

| Risk | Mitigation |
|---|---|
| A bad write leaves the radio with broken settings | CPS backup before the first write; fresh backup before every write; refuse unknown CRC rules; read-back verify |
| Write path needs something not visible in the serial traffic | Recordings show everything on the wire; if needed, read the unprotected `Rcdb2CLR.dll` / `G2CPS.FileData*.dll` |
| Other firmware versions differ | Select layouts by the RCDB version the radio reports; refuse unknown versions |
| Sensitive data (encryption keys in file 242, contacts) leaks via logs or exports | Never log file contents by default; keep `captures/` private |

## Decisions

| # | Decision | Status |
|---|---|---|
| 1 | Language: Rust core + CLI, GUI later (Tauri or egui); Windows, macOS and Linux | **Decided** 2026-11-18. Alternatives weighed: C#/.NET, Go, C++/Qt, Python |
| 2 | Scope: start with the BP515, design so more models can be added | **Decided** 2026-11-18 |
| 3 | Record CPS writing an unchanged configuration back to the radio (phase 3) | **Approved** 2026-11-18 |
| 4 | GUI toolkit (Tauri vs egui) | **Decided** 2026-11-19: Tauri 2 (table-heavy editor; WebView2 ships with Windows 11; WebKitGTK on Linux, WKWebView on macOS). No blocker found |

Because decision 3 is approved and recordings need no app code, the phase 3 recordings can be made early, in
parallel with phase 1. Knowing the write commands before designing the session API avoids rework.
