# Briefing for helper agents

Read this first, then `CLAUDE.md` (status, code map, working rules) and the doc for your task. Keep your
final report short: what you changed (files), what you verified (commands and results), what is open.

## The project in one paragraph

`hyt` is an independent replacement for Hytera's CPS programming software for Hytera G2 DMR radios, first for
a **BP515 U1** (RCDB layout version V1.5.24.000) on **COM3**. The owner has Hytera's permission to reverse
engineer CPS for personal use. Rust workspace: protocol (`hytera-proto`), file/layout codec (`hytera-codeplug`),
model profiles (`hytera-models`), shared operations (`hyt-core`), CLI `hyt` (`hyt-cli`) and a Tauri 2 desktop
app (`hyt-gui`, static `ui/` page, no npm). Reading, writing, editing values, adding/removing channels and zone
members all work and are verified against CPS recordings and on the real radio.

## Hard rules

- **Never talk to the radio** (no `hyt read/write/info --port`, no GUI read/write on COM3). Only the main session
  does live tests. Use saved folders, replays (`--replay captures/*.jsonl`) and the simulated radio
  (`hytera_proto::sim`, GUI env `HYT_GUI_SIM=<saved folder>`).
- **Never start or modify CPS** or anything under `C:\Program Files (x86)\Hytera`. Reading its files is fine.
- **Do not copy Hytera's files or code into the repository** (RCDB JSON, XML, DLL contents, the SDF database,
  exported `options.json`/`ui.json`). Load them at run time from the CPS install or `%LOCALAPPDATA%\hyt\rcdb\`.
  Short quotes of names/labels in docs are fine.
- `captures/` is private (the radio's full configuration, recordings). Never commit it; screenshots of the app
  showing the configuration go into `captures/`, not the repo.
- Keep `cargo test --workspace` passing and `cargo clippy --workspace --all-targets` at zero warnings. Add tests
  for new library code (tests may use files in `captures/` and skip themselves when they are missing; see
  `crates/hyt-core/tests/*.rs`).
- Mark new facts as **observed** (from a capture/file), **inferred**, or **unknown** in the docs.
- Do not commit unless your task says so; the main session reviews and commits (branch
  `unattended-2026-11-19`, commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`).

## Where things are

| What | Where |
|---|---|
| CPS install | `C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003` (`RcdbJsons\V1.5.24.000.txt` = field layouts; `RCDB\FileID`, `RCDB\TableLink`; `XMLToolCompactData.sdf` = parameter/UI database, SQL Server Compact 4.0) |
| Exported Hytera data | `%LOCALAPPDATA%\hyt\rcdb\V1.5.24.000\options.json` (labels, units, allowed values), `ui.json` (CPS screen structure) and `models.json` (model-number ranges/defaults/visibility, `hytera-codeplug/src/models.rs`), from `tools/export-rcdb-options.ps1` |
| Radio's current configuration | `captures/unattended_2026-11-19/start/` (a `hyt read` folder: `NNN_Name.bin` + `manifest.json`) |
| CPS recordings | `captures/*.jsonl` (+ `_files` folders with what CPS read/wrote); see `docs/capture-howto.md` |
| Knowledge base | `docs/protocol.md`, `docs/file-format.md`, `docs/cps-internals.md`, `docs/open-questions.md`, `docs/plan.md`, `docs/cps-features.md` (feature inventory) |

## Code map (beyond CLAUDE.md)

- `hytera-proto/src/sim.rs`: `SimRadio` / `SharedSim`, a simulated radio with fault injection.
- `hytera-codeplug/src/options.rs` (option catalogue), `ui.rs` (`UiTree`: CPS categories → pages → groups →
  layout field ids), `decode.rs` (`DecodedFile` / `Record` / `Fields`; `apply_with`).
- `hyt-core/src/edit.rs`: `apply_edits(saved, layout, options, links, edited files)`: the single validated path
  from edited decoded files to file bytes (values, appended records, removed channels with zone renumbering).
- `hyt-core/src/workspace.rs`: `Workspace` = opened folder + pending edits. Keeps an *edit form* (for
  `apply_edits`: records keep their original `index`, zone members use the opened folder's channel numbering)
  and a decoded *view* of the result. `set_value(FieldRef, text)`, `revert`, `add_channel`, `remove_channel`,
  zone member add/remove/move, `save_as`. Positions in the view address the edit form.
- `hyt-core/src/tables.rs`: GUI tables (`Table`, `Column`, `Cell` with `Editor`), `record_detail`,
  `change_list`, `channel_options`. `refs.rs`: reference fields shown by name.
- `hyt-core/src/write_flow.rs`: guarded write (backup → same-radio check → write → restart wait → verify).
- `hyt-gui/src/commands.rs`: Tauri commands (thin wrappers on `hyt-core`); `ui/app.js` renders, using
  `window.__TAURI__.core.invoke`; text from the radio only via `textContent`.

## How to work

- Build/test: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`
  (`%USERPROFILE%\.cargo\bin` may need to be on PATH in Git Bash: `export PATH="$PATH:$USERPROFILE/.cargo/bin"`).
- Editing files: prefer the Edit/Write tools. In Git Bash, heredocs containing backticks or `${...}` have broken
  before; for multi-line patches write a Python script to the scratchpad and run it with `.venv/Scripts/python.exe -I`.
- Run the GUI for screenshots: `HYT_GUI_SIM=<folder> WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333
  ./target/debug/hyt-gui.exe "<folder>"` (background), then `node tools/gui-drive/cdp.mjs steps.json` (see the
  header of `cdp.mjs`; `tools/gui-drive/helpers.js` has `cell`, `col`, `edit`, `nav`, `btn`). Stop it with
  `taskkill //IM hyt-gui.exe //F`. Only one instance with the DevTools port at a time.
- Query CPS's database read-only: copy `XMLToolCompactData.sdf` to a temp file and open it with
  `System.Data.SqlServerCe.dll` from `%ProgramFiles%\Microsoft SQL Server Compact Edition\v4.0` in PowerShell
  (see `tools/export-rcdb-options.ps1`).
