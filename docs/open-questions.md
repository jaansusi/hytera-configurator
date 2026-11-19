# Open questions

Ordered by how much they block a working read/write app. Update this file when something is answered: move the
answer into the right doc and note it under "Resolved" with the date and the evidence.

| # | Question | Blocks | How to answer |
|---|---|---|---|
| 1 | How long the radio is unavailable after a write, and whether it is a restart | Low (`hyt write`'s wait-and-retry read-back worked live on 2026-11-18) | Watch the radio during a write; time it |
| 2 | Why CPS flips `Emergency ID` in `QEU_C_AEmergency` (96) between 1 and 16777216 (`00 00 00 01`): unchanged on its first write, 1 → 16777216 on the second, back to 1 on the third (2026-11-18) | Low (looks like a CPS byte-order bug) | Check CPS's analog emergency screen |
| 3 | **Removing other records and references**: how CPS updates scan/roam lists, scan priority/designated channels, emergency revert channels, power-on/stun channel and presets when a channel is deleted; and what deleting a contact, zone or scan list changes | Deleting channels used in those places; deleting contacts, zones, … | Record CPS deleting a channel used in a scan list (and as designated channel), and deleting a contact; diff |
| 4 | Why the radio's read CRC for 5, 18, 46 matches no rule (structure itself is decoded) | Low (CPS-style writes recompute it) | Compare more reads |
| 5 | Which `different_version` variant applies to the BP515 (only matters for a few rare options) | Low | Compare with CPS screens where variants differ |
| 5b | **Confirm the reference mapping the GUI uses** (inferred 2026-11-19 from RCDB ranges, `hyt-core/src/refs.rs`): RX group list, scan list, roam list, analog emergency system = 1-based position in the named list, 0 = None; digital `Scan List Roam List` follows `Use Scan or Roam List`; `TX Contacts ID` = contact with that call ID (channels hold 10, no contact has it: shown as `10`). Other references (phone system, personality list, encryption keys, emergency on digital) still show catalogue text or numbers | Correct names in the GUI | CPS's channel screen: RX group list / TX contact columns for the BP515's channels; a second RX group list or scan list to tell 0- from 1-based |
| 5d | **Model-number rules** ([file-format.md §2b](file-format.md#2b-model-number-rules-ranges-defaults-visibility-by-model)): confirm on CPS's screens that the BP515 U1 refuses channel frequencies outside 400–470 MHz (and accepts 400.000 / 470.000) and that In Call TX Admit offers no "Always Allow"; which group wins when several match; what the `ModelNumberUILink` link types mean (screen parts per model) | Low (`hyt` already applies the ranges; a wrong rule would refuse or allow too much) | In CPS: type 399.995 and 470.005 MHz into a channel's RX frequency; open In Call TX Admit's list |
| 5c | How CPS handles removing a channel that is the radio's **current channel** (`QEU_C_Config` `Current CH Address`, = 1 on the BP515) or other references | Removing channel 1 of either list (refused by `hyt`/GUI) | Part of question 3's recordings |
| 6 | Header fields 4..7 (version/count) and 16..19 of the file container | Writing new files | Compare across files and recordings |
| 8 | Link header bytes 8–10, 11, 13–15 (constant so far); info items 3, 10, 0x45, 0x65, 0x100, 0x200, 0xFFF4; sub-command `0x001B` | Low | Other recordings, CPS "radio information" screen |
| 9 | Is the extra `00` byte rule "frame length multiple of 64" or something else? (3 of 3 consistent) | Low | More recordings with 64-byte-multiple frames |
| 10 | Other models and firmware: same framing? which RCDB versions? `TableLink.xml` only ships for V1.5.24.000 | Supporting other radios | Recordings from other radios |
| 11 | Error handling on the real radio: status codes other than 0, behaviour on a pulled cable mid-write. `hyt` now reports how far a failed write got and points to the backup; tested on the simulated radio (`hytera-proto/src/sim.rs`, `hyt-core/tests/robustness.rs`), not on the BP515 | Robust app | Controlled tests (pull the cable during a read; never during a write) |
| 12 | Does `hyt-gui` build and run on Linux (WebKitGTK 4.1, `pl2303` driver) and macOS (WKWebView, Prolific driver for the PL2303GC)? Only Windows 11 (WebView2 157) has been tried (2026-11-19) | Cross-platform release | Build in WSL / on a Mac or in CI; read the radio there |
| 13 | **Does CPS open a `.tpl` written by `hyt tpl export`** (import into its library, open it)? Does it need `Seqo`, a filled `Comm_RadioInfo` (253, `isReadTemplate` = 1) or a `TemplatesXML.xml` entry? Which data fill 253's `Radio ID`, licence flags and `N/A` fields? ([file-format.md §6](file-format.md#6-cps-template-files-tpl)) | Handing `hyt` configurations to CPS | Owner: import an exported file in CPS (no radio needed) and look at its Radio Information page; also try an export made without any `Seqo` |

## Resolved

- 2026-11-19: **How CPS picks model-specific ranges**: by the model number (info item 0x0000; 1-based positions
  without the `-`), conditions in `ModelNumberMainGroup`, overrides in `ModelNumberRange/Default/VisibleGroupLink`.
  The BP515 U1 (band `U1` at positions 17–18) gets 400–470 MHz for channel frequencies instead of the generic
  350–527 MHz; `hyt` now applies it. See [file-format.md §2b](file-format.md#2b-model-number-rules-ranges-defaults-visibility-by-model).
  (Observed in CPS's data files; CPS's on-screen behaviour is inferred, see 5d.)

- 2026-11-19: **GUI live write works.** In `hyt-gui`, digital channel 2 renamed `APTLK_CH2` → `HYT Test 2`, written to the BP515 on COM3 (owner's OK) via the Write dialog: the app saved the edit (`captures/gui-write_2026-11-19/edited`), took a fresh backup (`backup-before-write`, 0 differences from the GUI read), wrote 55 files (1 differed from the radio), waited for the restart and verified all 55 by read-back. A separate `hyt read` (`after`) differs from the backup in exactly that one setting; its `003` file is the written file plus the usual 152-byte stale tail. Screenshots `captures/gui-screens_2026-11-19/live4..6*.png`. (Observed.)

- 2026-11-19: **GUI live read works.** `hyt-gui` read the BP515 on COM3 (Read dialog, owner's OK) into `captures/gui-read_2026-11-19`: all 58 files byte-identical to the `hyt read` of the same day (`captures/remove-hyt-test-4_2026-11-19/after`); `hyt diff` shows 0 differences. Screenshots `captures/gui-screens_2026-11-19/live*.png`. (Observed.)

- 2026-11-18: transport, link framing, checksum, read commands and full read sequence
  ([protocol.md](protocol.md)), from `captures/read_2026-11-18.jsonl`.
- 2026-11-18: the read protocol works without CPS: `hyt read --port COM3` read all 58 files byte-identical to
  CPS's read (`captures/hyt-read_2026-11-18/`).
- 2026-11-18, from `captures/write-unchanged_2026-11-18.jsonl`: the **write path** (commands, write mode byte,
  order, no commit step), **which files are written** (55; not 242, 245, 253), the **CRC rule for written files**
  (`crc32(file[20:])`, header bytes 16–19 = 0, list files trimmed), CPS's resend timing and the extra `00` byte.
  See [protocol.md §5–6](protocol.md#5-full-write-sequence-used-by-cps) and
  [file-format.md §3–4a](file-format.md#4a-preparing-files-for-writing). Verified by `tests/replay_write.rs`:
  `hyt` reproduces all 312 writes of the recording byte for byte.
- 2026-11-18: **first live write by `hyt`** (unchanged configuration from `captures/hyt-read_2026-11-18`, without
  CPS's date stamp and file-80 change): all 55 files written; the radio needed one resend (seq 13, first chunk of
  file 87); afterwards it was briefly unavailable; a read-back (`captures/hyt-read-after-write_2026-11-18`) matched
  all 55 written files. The radio accepts files without CPS's two edits. Log: `captures/hyt-write_2026-11-18.log`.
- 2026-11-18: **file structure decoded** (block directory, block headers, slot index, records, two-level member
  lists) and **field decoding**: all 58 files decode into named fields and re-encode byte for byte, with the fields
  covering every record byte (`tests/decode_roundtrip.rs`). Frequencies are in Hz. See
  [file-format.md §2, §4](file-format.md#4-blocks-and-records).
- 2026-11-18: **option names, units and scaling** found in CPS's database `XMLToolCompactData.sdf`
  ([file-format.md §2a](file-format.md#2a-option-names-units-and-scaling-xmltoolcompactdatasdf)): tone `1072` =
  CTCSS 107.2 Hz, zone member type 0/1 = Digital/Analog, `TX Contacts ID` is a call ID, etc. Exported by
  `tools/export-rcdb-options.ps1`; `hyt show` labels 957 of the BP515's values.
- 2026-11-18: **`hyt` matches CPS's screens.** Owner's screenshots of CPS's Digital Channel and Analog Channel
  tables (`captures/cps-screens_2026-11-18/`) agree with `hyt show --all` on every visible column for both rows:
  colour code 9/14, Slot 1/2, Pseudo Trunk TX None, RX Only Off, scan settings, Talk Around Off, 444.725 /
  446.13125 / 446.18125 MHz, power High, TX Admit Color Code Free, TX timeout 60 s, spacing 12.5, CTCSS tail
  revert On / 180, scrambler Off, pre-emphasis On, squelch modes. This also **confirms LSB-first bit numbering**
  (several non-zero bit fields: slot, colour code, tail revert, squelch mode). Further screenshots confirmed the
  analog channels' carrier squelch Normal, RX/TX CTCSS 107.2, power Low, TX Admit Always Allow, TX timeout 60 s
  (stored 12), TOT pre-alert/rekey/reset 0, RX Only Off; Zone 1 = APTLK_CH1, 446.13125, 446.18125 (i.e. member
  `Digital 1, Analog 1, Analog 2` references channel numbers per type); contact Call 1, Group Call, ID 1.
- 2026-11-18: **first edit written by `hyt`**: digital channel 1 renamed `APTLK_CH1` → `HYT Test 1` via
  `hyt decode --labels` / `hyt apply` / `hyt write` (`captures/rename_2026-11-18/`: before, edits.json, edited,
  backup-before-write, write.log, after). One resend (seq 13, file 87, as always); automatic read-back after the
  restart verified all 55 files; `hyt diff before after` shows exactly the rename.
- 2026-11-18: **adding records and members** (`captures/add-channel-read/write_2026-11-18*`, CPS adding a digital
  channel and putting it in Zone 1): count fields, body length, template-based new record, member slots
  ([file-format.md §4](file-format.md#adding-records-and-members-observed-2026-11-18-captures-add-channel)).
  `hyt apply` reproduces CPS's files byte for byte. Also explained: the file-80 bytes CPS changes on every write
  are junk in an unused member slot.
- 2026-11-18: **first record added by `hyt`**: digital channel 4 `HYT Test 4` plus Zone 1 member Digital 4
  (`captures/add-hyt-test-4_2026-11-18/`); one resend (seq 13, as always); read-back verified all 55 files;
  `hyt diff before after` shows exactly the two additions.
- 2026-11-18: **removing a channel** (`captures/delete-channel-*_2026-11-18`, CPS deleting digital channel 3 which
  was in Zone 1): records shift down, counts and body length shrink, zone members of the deleted channel are
  removed and later ones renumbered ([file-format.md §4](file-format.md#removing-a-channel-observed-2026-11-18-captures-delete-channel)).
  `hyt apply` reproduces CPS's files byte for byte. Also seen: CPS flips `Emergency ID` in file 96 back to 1.
- 2026-11-19: **first channel removed by `hyt`**: `HYT Test 4` (digital 3), Zone 1 member removed automatically
  (`captures/remove-hyt-test-4_2026-11-19/`); read-back verified all 55 files; `hyt diff before after` shows exactly
  the two removals.
