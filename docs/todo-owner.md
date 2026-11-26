# Things that need the owner

Collected during unattended work (2026-11-19/20). Each item says why it needs you and what it unblocks.
Tick items off here (or delete them) as we go through them.

## Review

- [x] **Older BP515 (R225105080, firmware V1.5.13, RCDB V1.5.08.000)** (2026-11-26): read, show, edit, CSV and
  the GUI work with it (`captures/old-bp515-read_2026-11-26/`; it holds factory-like settings: `CH D1`, `CH A1`,
  Zone 1, contact `Call 1`). Labels come from CPS's V1.5.24 data, as noted in the app. Writing needs the model *and*
  its RCDB version verified (this closed a gap: before, `hyt write` only checked the model).
  Done 2026-11-26: you recorded CPS reading and writing its unchanged configuration; `hyt` reproduces both byte for
  byte, and a live `hyt write` of its unchanged configuration verified (`captures/old-bp515-write_2026-11-26/`).
  It is now managed like the other BP515s (CLI and GUI). Left for you: try it in the GUI.

- [ ] **Programming several radios** (2026-11-20; first live run done with the CLI on the second radio, see below): GUI **Program radios…**
  (toolbar; works without a configuration open) or `hyt batch new <batch> --from <config> --first-id N --name
  "Unit {n}" [--plan plan.csv]` then `hyt batch program <batch> --port COM3`. Each radio: read (backup) → its Radio
  ID/name by plan, earlier attempt or rule → written and verified → logged in `<batch>/log.csv`. The radio keeps
  its identity, never-written files, `Comm_Basic` radio data (serial, frequency range, licences, BT). A radio
  already done is skipped unless you press "Program it again". Live 2026-11-20: second BP515 (R23N030201) became
  Radio ID 2 "Radio 2" with radio 1's configuration, verified by a separate read
  (`captures/second-radio_2026-11-20/`; its earlier configuration is in `read-before/` if you want it back). Still
  to try: the GUI screen on a real radio. Open question 14 says what is inferred.
  The single-radio `hyt copy` stays on the CLI; the GUI's "Copy to radio…" dialog was replaced by this screen.
- [ ] **Radio IP now follows Radio ID** (all edits, incl. Quick setup's Radio ID), as CPS does. Before this,
  changing Radio ID in `hyt` left Radio IP at 10.0.0.1.

- [ ] **Try the app** (`cargo run -p hyt-gui -- captures/unattended_2026-11-19/start`, or the release build
  `target/release/hyt-gui.exe`). Screenshots of everything new are in `captures/gui-screens_2026-11-19/` and
  `captures/gui-screens_2026-11-19-night/`. Say what is confusing or missing.
- [ ] **Safe trial mode**: `HYT_GUI_SIM=<any saved folder>` adds "Simulated radio" as a port, so reads and
  writes can be tried without the radio (it even restarts after a write like the BP515).
- [ ] **Radio log of the night**: `captures/unattended_2026-11-19/log.md` lists every read/write I did and what
  the radio holds now.

- [ ] **Field help and Quick setup** (2026-11-19, tested offline only): click "?" next to a field name
  (detail panel, Settings, table headers) for CPS's own description. "Quick setup" (top of the left list)
  asks 10 plain questions and shows exactly what would change before applying. Check the questions and
  preset values (`crates/hyt-core/src/quick.rs`, `QUESTIONS`): are they the ones a beginner needs, and are
  the defaults right for how you use the radio?
- [ ] **Quick setup "Start fresh" starter channels**: DMR 1–4 (446.10625–446.14375 MHz, colour code 1,
  slot 1) and PMR 1–8 (446.00625–446.09375 MHz, 12.5 kHz, no tone), all low power, are my placeholder
  choice (`STARTER` in `crates/hyt-core/src/quick.rs`). Replace them with the channels you actually use. Note
  that the starter channels talk to contact "Call 1" (CPS's template), not to your old TX contact 10.

- [ ] **Which settings CPS shows** (2026-11-19): the app now hides what CPS hides (More ▾ has two switches to
  show licence-limited and internal fields). Please check these CPS screens against the app
  (details in `docs/research-cps-visibility.md`, "Owner checks"): the programmable keys page (are SK1/SK2
  settings shown?), the Conventional tree below Emergency (no Roam, no Analog Service), the analog channel
  dialog (no Two Tone fields, RX Squelch Mode only Carrier and CTCSS/CDCSS), the Analog Emergency page (only
  Emergency Type), and a key-function drop-down (option "Scramble").

## Needs CPS (recordings or screens; CPS runs elevated, so only you can record it)

How to record: `docs/capture-howto.md`. Each recording is a CPS read + one change + CPS write.

- [ ] **Add a zone, then delete a zone** → lets the app add/remove zones (structure inferred today, not checked
  against CPS).
- [ ] **Add a contact, then delete a contact** (one used as a channel's TX contact, if possible) → contact
  add/remove, and how CPS updates references to a deleted contact.
- [ ] **Delete a channel that is in a scan list and is the scan list's designated channel; delete channel 1**
  (the radio's current channel) → open questions 3 and 5c; today such removals are refused.
- [ ] **Add and remove a scan list member, an RX group list member, a roam list member** → member editing in
  those lists (today read-only).
- [ ] **Reorder** a channel and a contact in CPS (Up/Down) → reordering in the app (renumbering rules).
- [ ] **Encryption keys** (file 242, today never written): record CPS adding one *test* key → key list editing.
  Optional; only if you use encryption.
- [ ] **When the Hytera licences arrive** (Basic Encryption ordered 2026-11-19): copy the `.zip` (keep the original
  untouched) and run `hyt licence check <zip> --radio captures/gui-read_2026-11-19`. It does CPS's checks offline and
  says whether CPS V1.5.24 would send it. Expected: refused for Basic Encryption on the BP515 U1 (CPS V1.5.24's
  model rule); then ask Hytera which CPS version the licence is for. Before installing with any CPS, start the
  recorder (`tools/start_tap.cmd`, elevated) so the install is recorded. Afterwards: `hyt licence status --port COM3`
  and a `hyt read` (Basic Encryption should be 1 in file 18).
- [ ] **Password/lock, licence activation, radio reset, reading a locked radio**: out of scope unless you
  want them; each needs a recording and carries risk (see `docs/cps-features.md`, gap 15).
- [ ] **Frequency limits by model (open question 5d)**: in CPS, type 399.995 and 470.005 MHz into a channel's RX
  frequency (both should be refused; 400.000 and 470.000 accepted), and check that "In Call TX Admit" has no
  "Always Allow". The app now enforces these limits from CPS's model-number rules.
- [ ] **CPS templates (open question 13)**: `hyt tpl export <folder> --out x.tpl --name "hyt test"`, then
  import `x.tpl` into CPS's template library and open it (also try an export of an imported template). Check
  CPS's Radio Information page for it (hyt leaves that file empty, as on the radio).
- [ ] **Greyed-out fields (CPS's dependency rules)**: compare a few with CPS's screens, e.g. a digital channel
  with RX Only on (TX settings greyed), digital emergency with type Disable, analog TX tone value with tone type
  None. Also check that the app's "Selected" matches CPS's "Selected Channel" in revert-channel fields.
- [ ] **One CPS screenshot** of the digital channel table showing the "TX Contact" and "RX Group List" columns
  (open question 5b: confirms how the app names them).

## Needs your hardware or machines

- [ ] **Linux/macOS build**: WSL is not installed here (installing needs admin + reboot), and there is no Mac.
  Either install WSL (`wsl --install`) or try on a Mac; `hyt rcdb import <copy of CPS folder>` sets up the data.
