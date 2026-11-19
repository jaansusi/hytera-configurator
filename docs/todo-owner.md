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
- [ ] **One CPS screenshot** of the digital channel table showing the "TX Contact" and "RX Group List" columns
  (open question 5b: confirms how the app names them).

## Needs your hardware or machines

- [ ] **Linux/macOS build**: WSL is not installed here (installing needs admin + reboot), and there is no Mac.
  Either install WSL (`wsl --install`) or try on a Mac; `hyt rcdb import <copy of CPS folder>` sets up the data.
