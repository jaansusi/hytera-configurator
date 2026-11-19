# Hytera radio programming without CPS

Goal: an independent app that reads and writes the settings of Hytera G2 DMR radios over the USB programming
cable, replacing Hytera CPS. Personal use; the owner has Hytera's permission to reverse engineer CPS for this.

First target radio: **BP515 U1**, firmware V1.5.36.001, RCDB (layout) version **V1.5.24.000**, serial R23N141382,
on **COM3** (Prolific PL2303GC cable). Reference software: Hytera CPS V1.5.24.000.0003.

## Status (update this when it changes)

- Done: read protocol fully decoded from a recording of CPS (all 648 frames verified); 58 files read and
  rebuilt; layout database located. See `docs/`.
- Decided (2026-11-18): Rust; start with the BP515 and design for more models; Windows, macOS and Linux
  targets; recording CPS writing an unchanged configuration is approved.
- Phase 1 done (2026-11-18): `hyt ports | info | read`. The replay test proves our bytes match CPS's exactly
  (324 writes). **Live read on the BP515 via COM3 works**: all 58 files byte-identical to CPS's read
  (`captures/hyt-read_2026-11-18/`).
- Phase 3 done (2026-11-18): CPS write of an unchanged configuration recorded
  (`captures/write-unchanged_2026-11-18.jsonl`) and decoded: commands, order, 55 files, CRC rule, resend timing.
- Phase 4 done (2026-11-18): `hyt write` (fresh backup → identity check → prepared files in CPS order → wait for
  the radio's restart → read-back verify). Replay test reproduces all 312 CPS writes byte for byte. **First live
  write succeeded**: unchanged configuration, all 55 files read back as written. BP515 is now `write_supported`.
  The automatic wait-before-verify was added after that run and has not yet run live.
- Phase 2 done (2026-11-18): file structure decoded; `hyt decode` (all fields → JSON) and `hyt show`
  (overview, or `--all` for every field) with CPS's option names/units/scaling from `XMLToolCompactData.sdf`
  (exported by `tools/export-rcdb-options.ps1` to `%LOCALAPPDATA%\hyt\rcdb\<ver>\options.json`). Decode→encode is
  exact for all 58 files. Open: adding or removing records.
- `hyt show` confirmed against 5 CPS screenshots (channels, zone, contacts) on 2026-11-18.
- Phase 5 step 1 done (2026-11-18): editing values in place: `hyt decode --labels` → edit JSON → `hyt apply`
  (validates changed values against CPS's options/ranges, saves a new folder) → `hyt diff` / `hyt show` →
  `hyt write --from`. **First live edit written 2026-11-18**: digital channel 1 renamed `APTLK_CH1` →
  `HYT Test 1` (`captures/rename_2026-11-18/`); automatic wait-for-restart + read-back verified all 55 files; a
  separate read confirms exactly that one change. The radio's channel 1 is now called `HYT Test 1`.
- Phase 5 step 2 mostly done (2026-11-18): from CPS's recording of adding a channel + zone member, `hyt apply`
  now appends records (channels from CPS's template file, others from RCDB defaults) and adds/removes zone
  members, checks linked list lengths and zone→channel references, and reproduces CPS's files byte for byte.
  **Written live 2026-11-18**: digital channel 4 `HYT Test 4` (template settings, 429.075 MHz) added and put in
  Zone 1 (`captures/add-hyt-test-4_2026-11-18/`), verified by read-back.
- Removing channels (2026-11-18): from CPS's recording of deleting channel 3, `hyt apply` removes channel
  records, drops/renumbers zone members like CPS and reproduces CPS's files byte for byte. It refuses when other
  channel references (scan/roam lists, designated/priority/revert/power-on/stun channel, presets) would be
  affected, and refuses removing other record kinds. **Done live 2026-11-19**: `HYT Test 4` removed, Zone 1
  member dropped automatically, verified by read-back (`captures/remove-hyt-test-4_2026-11-19/`).
- Radio state: see the end of `captures/unattended_2026-11-19/log.md` (every read/write of the unattended
  night 2026-11-19, done with the owner's permission; the starting configuration is written back at the end).
- Phase 5 GUI milestone 1 built (2026-11-19, awaiting the owner's review): code shared by CLI and GUI moved into
  `crates/hyt-core` (CLI unchanged in use); `crates/hyt-gui` (Tauri 2) opens a saved folder or reads the radio,
  shows digital/analog channels, zones (members by channel alias) and contacts with CPS's labels/units, edits
  values in tables and a per-record detail panel (validated by `apply_edits` on every edit; passwords never
  shown), lists/reverts changes, saves as a new folder and writes through `hyt_core::write_flow` (same as
  `hyt write`) after a confirmation dialog. **Live read from the GUI on COM3 works** (2026-11-19, owner's OK):
  58 files byte-identical to that day's `hyt read` (`captures/gui-read_2026-11-19/`). **Live write from the GUI
  works** (2026-11-19): digital channel 2 renamed `APTLK_CH2` → `HYT Test 2`, verified by the app's read-back and
  a separate `hyt read` (`captures/gui-write_2026-11-19/`).
- GUI milestone 2 built (2026-11-19, tested offline only): add digital/analog channels (CPS's template, as CPS
  does), remove channels (zones updated like CPS; refused when another reference would change, e.g. channel 1 of
  either list is the radio's current channel), add/remove/move zone members, and reference fields shown and
  chosen by name (TX contact by call ID, RX group list, scan/roam list, analog scan list and emergency system;
  mapping inferred, `hyt-core/src/refs.rs`). The GUI's add-channel + zone-member and
  remove-channel reproduce CPS's recorded files byte for byte (`tests/workspace_structure.rs`). `Workspace`
  keeps an edit form (for `apply_edits`) and a decoded view of the result. Not offered: adding/removing zones and
  contacts (not recorded from CPS). **Written live 2026-11-19 night** (GUI on COM3): channel `HYT Test 3` added +
  put in Zone 1, TX contact by name, radio alias and keypad tone from the settings view; verified by read-back
  and a separate read.
- Unattended night 2026-11-19 (branch `unattended-2026-11-19`, see `docs/cps-features.md` for the CPS feature
  inventory and `docs/todo-owner.md` for what needs the owner): simulated radio (`hytera-proto/src/sim.rs`,
  GUI `HYT_GUI_SIM`) and clearer failure messages; atomic folder saves; settings view (every single-record
  setting CPS shows, 17 topics, search; `ui.json` from the export script); list editors (RX group/scan/roam and 10
  other lists, values only); model-number ranges (U1: 400–470 MHz) and visibility; references and dependency
  rules from CPS's own config/database; programming date stamped on write like CPS (byte-identical);
  `hyt rcdb status|import` (work from a copy of CPS's data); `hyt csv` + GUI CSV import/export; CPS `.tpl`
  import/export (`hyt tpl`, GUI); duplicate channel; duplicate contacts refused like CPS. Live: CLI write of a
  scan-list alias and a quick message made via `hyt csv`, verified.
- GUI "More ▾" menu (2026-11-19 night, offline only): CSV export/import of the table shown (pending changes + summary
  of refused cells) and CPS template export (what is shown, unsaved changes included) / import (for the open
  folder's radio, into a new folder that is then opened; warnings shown). Banner shows `model_note`.
- Field help + Quick setup (2026-11-19, offline only): CPS's own help texts from `CPSParams.xlsx` (install
  root; `hytera-codeplug/src/help.rs`, by field `Id`) on every column, shown behind a "?" in the GUI and
  searched by the settings search (name matches first). GUI "Quick setup": 10 plain questions (name/ID,
  language/clock, sounds, screen, key lock, VOX, battery saving, power and TX time limit on all channels, add a
  channel), each answer = preset values (`hyt-core/src/quick.rs`); dry-run review, then applied all-or-nothing
  as ordinary pending changes. Questions appear only where CPS shows and allows every field for the model.
  A Start step offers "adjust what the radio has" or "start fresh" (each question preselects its recommended
  answer, and all channels are replaced by a starter set: DMR 1–4 at 446.10625–446.14375 MHz + PMR 1–8,
  low power; `STARTER` in `quick.rs`). Replacing reuses existing channels in place after resetting them to
  CPS's channel template (`Workspace::reset_channel`), removes extras from the end and rebuilds zone 1, so
  references to channel 1 (current channel, scan list) stay valid.
- Encryption checked with the owner in CPS (2026-11-19, `captures/cps-encryption_2026-11-19/`): CPS shows no
  encryption at all on this radio (Basic Encryption licence Off). `hyt` now: channel `Encrypt Key` names keys
  of the key list (242) and `apply_edits` refuses a missing key; key values are secret like passwords; fields
  CPS hides are classified (`hyt-core/src/visibility.rs`) as *licence* (feature not licensed) or *internal*
  (CPSParams.xlsx visibility, or on no CPS screen), read-only, and hidden in the GUI unless switched on (More
  menu, two separate switches). A research agent then read CPS's code (`docs/research-cps-visibility.md`):
  *internal* now follows CPS's database (`Param.visible`/`delete_flag`, exported to `visibility.json`; xlsx only as
  fallback) and *licence* the real rule (`ParamTemplate.config` LinkType 20125: Roam, Basic Encryption, Two Tone;
  incl. whole lists). Digital channel 1 shows exactly CPS's 22 fields, Feature Control its 4 flags. Fixed: model
  rules' `is_visible` was read inverted (`models.rs`). Analysis scripts in `tools/cps-analysis/`.
- Licences (2026-11-19, offline only): CPS installs licences with its Authorization button from a Hytera `.zip`
  (readme XML + MD5 + per-serial `.fcr`, pushed with `E100` secure-file commands); the radio reports installed
  licences in identify item `0x0200` (BP515: none). CPS V1.5.24 refuses Basic Encryption (1509) for the BP515 U1.
  `hyt licence check <zip> --radio <read>` repeats CPS's checks; `hyt licence status` shows item `0x0200` and the
  flags (`hyt-core/src/licence.rs`). The owner has ordered licences from Hytera (due about 2026-11-26).
- Next: owner review (`docs/todo-owner.md`); CPS recordings for zone/contact/list-member add/remove and
  referenced-channel removal; merge branch `unattended-2026-11-19` after review.

## Code

Rust workspace (Rust 1.100 via rustup, MSVC linker from VS Build Tools 2026). New shells have `cargo` on PATH;
older ones need `%USERPROFILE%\.cargo\bin` added.

| Crate | Role |
|---|---|
| `crates/hytera-proto` | Transports (serial, replay of `captures/*.jsonl`), link frames (resends, extra `00` byte), `0x0A0B` packets, `Session` (identify, read files, write files) |
| `crates/hytera-codeplug` | RCDB file lists, `TableLink.xml`, CPS write order, file header + CRC rules + `prepare_for_write` (`container`), block structure (`structure`), field layouts from `RcdbJsons` (`layout`), field codec (`value`), whole-file decode/apply (`decode`), CPS's reference config `DataValidationConfig.config`/`ComplexRuleConfig.Config` (`validation`), CPS's help
texts `CPSParams.xlsx` (`help`), which parameters CPS shows (`param_visibility`, from `visibility.json`), link rules from `links.json` (`link_rules`) |
| `crates/hytera-models` | Model profiles (`BP515` incl. files never written; read-only `GENERIC_G2` fallback) |
| `crates/hyt-core` | Shared by CLI and GUI: `src/lib.rs` (read/save/load folders, plan/verify writes, `load_options`), `view.rs` (labelled output, `is_secret`), `edit.rs` (labels ↔ raw, `apply_edits`, diff), `write_flow.rs` (backup → same-radio check → write → wait for restart → verify), `workspace.rs` (open folder + validated pending edits incl. channel add/remove and zone member actions, save-as), `tables.rs` (GUI tables, record detail, change list), `lists.rs` (list editors: RX group/scan/roam lists and other one-level lists as `list:<file>` tables, members read-only by name), `refs.rs` (reference fields by name, from CPS's `DataValidationConfig.config`; built-in fallback), `link_rules.rs` (CPS's link rules: disabled fields with reason, forced values), `sheet.rs` (tables as CSV, files + suggested names), `tpl.rs` (CPS `.tpl` import/export incl. `export_workspace`, `import_file`), `licence.rs` (licence package checks like CPS's Authorization, installed licences from item `0x0200`), `quick.rs` (beginner quick
setup: questions → preset values, dry run / all-or-nothing apply), `visibility.rs` (fields CPS hides: licence /
internal); tests: `visibility.rs` (against CPS screenshots); tests: `quick_setup.rs` (questions, presets,
channel add, help texts), `replay_read.rs`, `replay_write.rs` (CPS recordings), `decode_roundtrip.rs`, `edit_values.rs`, `add_records.rs`, `delete_channel.rs`, `workspace_tables.rs` (GUI model + write-flow refusals), `workspace_structure.rs` (GUI add/remove vs CPS recordings, renumbering, references), `list_editors.rs` (list tables, member names, value edits, change list), `references.rs` (CPS references, special values, link rules) |
| `crates/hyt-cli` | `hyt` binary (`ports`, `info`, `read`, `write`, `decode [--labels]`, `show [--all]`, `apply`, `diff`, `csv`, `tpl`, `licence check|status`); only argument parsing and printing (`src/main.rs`) |
| `crates/hyt-gui` | Tauri 2 desktop app `hyt-gui [folder]`: `src/commands.rs` (commands on `hyt-core`, radio jobs on a blocking thread with `progress` events), `ui/` (static `index.html`, `app.js`, `style.css`; no npm build), `tauri.conf.json`, `capabilities/`, `icons/` (from `tools/make_icon.py`) |

- Test: `cargo test --workspace` (replay test skips itself if `captures/` or the CPS install is missing).
- Lint: `cargo clippy --workspace --all-targets` (keep it at zero warnings).
- Try offline: `cargo run -p hyt-cli -- read --replay captures/read_2026-11-18.jsonl --out <new folder>`.
- GUI: `cargo run -p hyt-gui -- <read folder>` (or open/read from the toolbar). `HYT_GUI_DEV=1` adds "Replay a
  recording" as a port, for testing without the radio. For screenshots/automation, start it with
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and drive it with `tools/gui-drive/cdp.mjs`.
  Linux builds need WebKitGTK 4.1 dev packages (not tried yet); macOS untested.
- Live: `cargo run -p hyt-cli -- read --port COM3 --out <new folder>` (CPS must not be using the port).
- Edit: `hyt decode <read> --labels --out edits.json`, edit (whole file or only some files; raw numbers or CPS
  text), `hyt apply <read> edits.json --out <new folder>`, check with `hyt diff <read> <new folder>`. To add a
  record, append `{"fields": {...}}` (no `index`) to a block's `records` (omitted fields come from the
  template/defaults); to remove a channel, delete its entry (keep the others' `index`; zones are updated); to add
  a zone member, append `{"Channel List": 3, "Channel Type": "Digital"}` to the zone's `members` (channel numbers
  as in the read; channels added in the same edit continue that numbering).
- Write (changes the radio; only with the owner's OK): `hyt write --port COM3 --from <read or edited folder>
  --backup <new folder>`. `--allow-unverified` is only needed for models without `write_supported`.
- RCDB location: `--rcdb <folder>` or env `HYT_RCDB`; default is the CPS install path.
- Option labels: run `powershell -ExecutionPolicy Bypass -File tools\export-rcdb-options.ps1` once per CPS
  version; `hyt show --options <file>` or env `HYT_OPTIONS` overrides the default location.
- Link rules (greyed-out/forced fields): run `powershell -ExecutionPolicy Bypass -File tools\export-rcdb-links.ps1`
  once per CPS version (writes `links.json` next to `options.json`); without it nothing is greyed out.

## Knowledge base

| Doc | What it holds |
|---|---|
| `docs/protocol.md` | Serial settings, link frames and checksum, application commands, read sequence, real byte examples |
| `docs/file-format.md` | Radio file container, CRC, RCDB layout JSON, list files, the 58 file IDs |
| `docs/cps-internals.md` | How CPS is built: which DLLs are obfuscated, where the data files are, analysis tool notes |
| `docs/capture-howto.md` | How to record CPS talking to the radio and decode the recording |
| `docs/open-questions.md` | Unanswered questions, what each blocks, how to answer it, resolved log |
| `docs/plan.md` | Phased plan for the app, risks, decisions pending |
| `docs/research-cps-visibility.md` | How CPS decides which settings to show (database flags, model rules, licence rule), from its code |

Files: `captures/` (recordings + files read from the radio; **private**, includes the radio's whole
configuration), `tools/` (recorder `serial_tap.js` + `tap_runner.py` + `start_tap.cmd`, decoder
`parse_capture.py`, GUI driver `gui-drive/`, icon generator `make_icon.py`, read-only CPS code/database analysis
`cps-analysis/`), `.venv` (Python with `frida` 17.27.0 and `pyserial`; see `requirements.txt`).

## Key facts at a glance

- Serial 115200 8N1; CPS clears RTS and DTR. Frames: `5E 6E 01 <type> <src> <dst> <seq16> 00 00 00 01 <flag>
  00 00 00 <len16> <cksum16>` + payload; checksum is the RFC 1071 Internet checksum over the whole frame.
- Payload: `0A 0B <opcode16> <sub16> <txn32> <len16> <data>`, big-endian; responses set opcode bit `0x1000`.
- Read flow: CONNECT → `01F0/0000` enter → per file `01F0/0006` open (fileId, 0x400) + `41F8/0008` chunks →
  `01F0/0001` leave → DISCONNECT. Match responses to requests by transaction ID.
- File contents are little-endian with UTF-16LE strings; layouts are in CPS's `RcdbJsons\<rcdb version>.txt`.

## Working rules

- **Never send anything that writes to the radio** without the owner's explicit OK for that specific action.
  Reads are low risk but still ask before talking to the radio from a new tool for the first time.
- CPS runs elevated (its launcher requires admin), so attaching to it needs an elevated recorder run by the
  owner. CPS only opens COM3 during a read/write, so our own tools can use the port while CPS is idle.
- Mark every new fact as observed (from a capture or file), inferred, or unknown, and record it in the right
  doc. Move answered questions from `docs/open-questions.md` into its "Resolved" list.
- Do not copy Hytera's RCDB files or code into the app; load them from the CPS install.
