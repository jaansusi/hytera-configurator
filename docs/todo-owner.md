# Things that need the owner

Collected during unattended work (2026-11-19/20). Each item says why it needs you and what it unblocks.
Tick items off here (or delete them) as we go through them.

## Review

- [ ] **Try the app** (`cargo run -p hyt-gui -- captures/unattended_2026-11-19/start`, or the release build
  `target/release/hyt-gui.exe`). Screenshots of everything new are in `captures/gui-screens_2026-11-19/` and
  `captures/gui-screens_2026-11-20/`. Say what is confusing or missing.
- [ ] **Safe trial mode**: `HYT_GUI_SIM=<any saved folder>` adds "Simulated radio" as a port, so reads and
  writes can be tried without the radio (it even restarts after a write like the BP515).
- [ ] **Radio log of the night**: `captures/unattended_2026-11-19/log.md` lists every read/write I did and what
  the radio holds now.

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
