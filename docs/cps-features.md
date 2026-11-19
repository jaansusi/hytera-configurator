# CPS features vs `hyt` (BP515 U1, RCDB V1.5.24.000)

Inventory of what Hytera CPS V1.5.24.000.0003 offers for the BP515 U1, compared with `hyt` (CLI + GUI) on
2026-11-19, as a work list toward practical parity.

**Sources** (all read-only): CPS's screen tree for this model (`%LOCALAPPDATA%\hyt\rcdb\V1.5.24.000\ui.json`,
exported from `XMLToolCompactData.sdf`), the layout `RcdbJsons\V1.5.24.000.txt`, the radio's 58 files
(`captures/unattended_2026-11-19/start/`), CPS's config files (`DataValidationConfig.config`,
`ComplexRuleConfig.Config`, `TableRowAddHandleConfig.config`, `ExportConfig.config`, `DataIntegrityConfig.config`,
`DataCheckConfig.config`, `Config\ImportExport\`, `Update_CT\`), the English UI strings
(`LanguagePackage\en-US.xaml`), the manual `Help_EN.pdf` (19 pages: templates, program, license, reset,
upgrade, radio information), CPS's saved templates (`Templates\*.tpl`), more SDF tables queried from a temp
copy, and `hyt`'s code (`hyt-cli/src/main.rs`, `hyt-core/src/{tables,settings,refs,edit,workspace}.rs`,
`hyt-gui/ui/*`).

**Tags:** *observed* = seen in a CPS file or table; *inferred* = concluded from names or structure, not yet
checked on CPS's screens or on the wire. "Recording" = a CPS session recorded with the serial tap
([capture-howto.md](capture-howto.md)); that needs the owner.

## How CPS's screen tree maps to the radio (observed)

- Roots for this model, in order: Radio Information, Feature Control, Common, Narrowband Setting,
  Conventional, Developer. The SDF has only one UI model (`UiAppearanceModel` 1 "DMR"), so the export's
  `ModelOnlyId = 1` filter is right.
- 63 pages hold parameters the BP515 has. 2 pages (Key › Version, Location › BT Location) have none. 6
  categories have no English name and no pages (ids 44, 45, 50, 118, 121, 122).
- `UiPage.Type` is 0 for every page and `Visible` is 0 for nearly all, so neither tells list pages apart.
  The kind below comes from the layout's `Block_Type` (0 single record, 1 list, 2 list of member lists).
  `UiGroupBlockLink` names the group that shows each list block (e.g. group 205 "Digital Channel" → block 1,
  221 "Scan" → 619).
- **On the radio but with no page for this model** (observed): the 15 XPT/"PTE" files (157–168, 179, 180,
  183, 219, 230: personalities, contacts, RX groups, voice/data channels, site lists, roam, emergency, quick
  GPS, one-touch call), `Comm_Selfdefinetone` (94), call logs (74, 243, 244), the BT paired list (245) and the
  channel templates (251, 252). Inferred: CPS hides XPT for this model or licence. `hyt` reads, decodes and
  writes them unchanged.
- **CPS never writes** 242 (E2EE key list), 245 (BT paired list) and 253 (radio info) in an unchanged write
  (observed, `hytera-models` `write_excluded_files`).

## Table 1: settings and lists

Status key for single-record pages: **S** = every field can be edited with `hyt decode --labels` / `hyt apply`
(CLI). In the GUI it shows in the settings view (`hyt-core/src/settings.rs` plus GUI wiring, being built
2026-11-19). It is not "done" while CPS's link rules (greying out, dependent values) and model-number ranges
are missing (gaps 2–3). Files/blocks are `file/block`.

| CPS page (category path) | files/blocks | kind | hyt | needs | notes |
|---|---|---|---|---|---|
| **Radio Information** › Radio Information | 253/920 | settings | done (read-only) | – | `hyt info`, manifest, GUI identity; settings view read-only. CPS never writes 253 |
| **Feature Control** › Feature Control | 18/902, 253/920 | settings | done (read-only) | owner | "Feature in Subscriber" + "Custom Authorization" (Two Tone, Pure Analog/Digital, Basic Encryption, Roam, Priority Interrupt, Radio Disable/Enable, Remote Monitor). Licences come from a licence file, see Table 2 |
| Common › Basic Setting › Radio Alias | 18/612 | settings | partial (S) | code | |
| … › Language | 18/598 | settings | partial (S) | code | default depends on the model number (observed, `ModelNumberDefaultGroupLink`) |
| … › Date and Time | 18/592 | settings | partial (S) | code | |
| … › Power | 18/892 | settings | partial (S) | code | |
| … › Password | 18/896 | settings | read-only by design | owner | CPS Read/Write Lock + passwords; never shown by `hyt` (`is_secret`) |
| … › Keypad Lock | 19/591 | settings | partial (S) | code | |
| … › Device Info | 18/902 | settings | read-only | – | frequency range, programming time |
| … › Audio Feedback Suppression | 46/646 | settings | partial (S) | code | |
| Common › Audio Setting › Setting / Three Band Equalizer / Gain Setting | 34/661, 46/646, 46/783 | settings | partial (S) | code | |
| Common › Key › Key | 46/50 | settings | partial (S) | code | key function lists depend on the model number (observed: 380–584 range rows for TK/SK3/P1). CPS also has a key picture view |
| Common › Power and Performance | 46/643 | settings | partial (S) | code | frequency DC values depend on the model number (observed) |
| Common › UI Indication › Scene Mode, Tone, Vibration, LED, Screen Display, Convert Mode | 19/781, 633, 634, 570, 568, 586, 923 | settings | partial (S) | code | |
| Common › Menu › Menu | 18/904, 87/903 | settings | partial (S) | code | 69 menu on/off switches incl. front-panel programming |
| Common › Service Setting › Voice Service, Voice Notify, VoX, BT | 87/642, 660, 802, 797, 893; 19/634 | settings | partial (S) | code | |
| Common › Location Service › Location Service | 86/588 | settings | partial (S) | code | |
| Common › Security › Man Down, Lone Worker | 87/751, 87/752 | settings | partial (S) | code | |
| Common › Developer › Developer | 97/645 | settings | read-only by design | owner | 116 fields; CPS shows them (editable unknown) |
| **Narrowband Setting** › Work Mode, Basic Setting, Analog Setting | 104/838, 667, 822, 830, 848, 559; 5/5 | settings | partial (S) | code | |
| **Conventional** › Basic Setting › Setting | 5/606, 641, 895 | settings | partial (S) | code | current zone/channel, power-on zone |
| … › Network | 5/579 | settings | partial (S) | code | |
| … › One Touch Call | 200/807 | list (fixed 5 rows) | CLI only | code | references a contact by (type, ID) (observed, `DataValidationConfig` 807 → block 3) |
| … › Preset Channel | 241/879 | list (fixed 4 rows) | CLI only | code | zone/channel indexes; `hyt` refuses channel removal while presets are set |
| Conventional › Digital Setting › Setting, Dial Setting, Location Service, RRS, Voice, Data Bearer Service | 5/5, 739, 811, 757 | settings | partial (S) | code | Dial Setting (radio ID, dial rules) is checked by CPS against contacts (observed, `DataCheckConfig` 1001–1003) |
| … › Encrypt | 5/5 + 242/897 | settings + list (0/30 keys) | settings partial (S); key list missing | recording + owner | CPS did not write 242 in the unchanged write; `KeyEncryptCLR.dll` suggests keys are encrypted before writing (inferred) |
| … › Quick Message | 254/921 | list (1/10) | CLI only | code (values); recording (add/remove) | |
| Conventional › Analog Setting | 5/845 | settings | partial (S) | code | |
| Conventional › Zone and Channel › **Zone** | 6/7 (+ aliases 7/8) | two-level list (1/64 zones; aliases 1/32) | partial: alias edit, member add/remove/move | recording (add/remove zones) | members = (channel index, type) (observed, `ComplexRuleConfig` −5) |
| … › **Digital Channel** | 3/1 | list (2/256) | done: edit, add (CPS template), remove | recording (remove when referenced; reorder) | 42 fields; references partly by name (`refs.rs`) |
| … › **Analog Channel** | 9/4 | list (2/256) | done: same as digital | recording (same) | |
| Conventional › Contact › **Contact** | 8/3 | list (1/160) | partial: edit only (GUI); CLI append with RCDB defaults, not checked against CPS | recording (add, delete) | unique key (Call ID, Call Type) (observed, `TableRowAddHandleConfig`); has a `delete flag` field, so deletion may only mark the record (inferred) |
| … › **RX Group List** | 80/607 (+ aliases 81/608) | two-level list (1/16, 0 members; aliases 1/32) | CLI only | code (view); recording (add/remove lists, members) | members = group-call contacts by call ID, unique per list (observed, `DataValidationConfig` 607, `TableRowAddHandleConfig`) |
| … › Phone List | 212/823 | list (0/256) | CLI only | code (view); recording (add/remove) | |
| Conventional › Scan › Setting | 5/606 | settings | partial (S) | code | Scan On |
| … › **Scan** | 84/619 + 85/620 | two-level list (1/8; config 1/64) | CLI only; channel refs by name | code (view); recording (add/remove lists, members) | members (index, type) digital+analog (observed, `ComplexRuleConfig` −7); Priority Channel 1/2: 0 = None, 65535 = selected channel (observed) |
| Conventional › Roam › Setting | 5/606 | settings | partial (S) | code | Roam On |
| … › **Roam** | 38/575 + 39/576 | two-level list (1/8) | CLI only; refs by name | code (view); recording (add/remove) | members are digital channels only (observed, `ComplexRuleConfig` −6) |
| Conventional › Emergency › Digital Emergency | 95/636 | settings (one system on the BP515) | partial (S) | code | Revert Channel: 1-based digital channel, 0 = selected channel (observed) |
| … › Analog Emergency | 96/637 | settings | partial (S) | code | Revert Channel likewise for analog. Open question 2 (Emergency ID byte flip) |
| Conventional › Phone › Phone System | 213/824 | list (0/32) | CLI only | code (view); recording (add/remove) | referenced by digital channel `Phone System` (observed) |
| Conventional › Analog Service › HDC1200 System / HDC1200 Contact | 246/913, 247/914 | list (1/4, 1/32) | CLI only | code (view); recording (add/remove) | HDC1200 is a licence item (observed, en-US.xaml) |
| … › 2-Tone System / 2-Tone Contact / 2-Tone Personality | 249/915, 248/916, 250/917 | list (1/4, 1/32, 1/64) | CLI only | code (view); recording (add/remove) | Two Tone is a licence item |
| **Developer** › DMR/PDT Common, Conventional | 34/661, 34/571 | settings | read-only by design | owner | 59 fields |

## Table 2: application features

| CPS feature | evidence | hyt | needs | notes |
|---|---|---|---|---|
| Read from radio | Help 2.1.1; en-US `StandAlongFrequency` | **done** (`hyt read`, GUI) | – | byte-identical to CPS |
| Program (write) radio | Help 2.2 | **done** (`hyt write`, GUI) | – | `hyt` adds a fresh backup, a same-radio check and read-back verification, which CPS lacks (observed in recordings) |
| Template library: name, description, list, search, clone, delete, batch delete | Help 2.1; en-US `CloneTemplate`, `BatchDelete`, `OnlyDisplayUnused`; `Templates\TemplatesXML.xml` | partial: saved folders + manifest, Save as | code | a folder is the template. No list, description or search in the GUI |
| New template without a radio (pick model, defaults) | Help 2.1.1; en-US `AddTemplate` | missing | code (inferred: RCDB defaults + template files 251/252) | low value for one radio |
| **Import/export `.tpl` template files** | Help 2.1.1/2.1.4; `.tpl` format observed | missing | code (offline) | **observed:** `.tpl` = ZIP of `tpHeader.data` (XML: product, model number, RCDB version, serial, name) + one `<FileName>.hps` per radio file. Each `.hps` is itself a ZIP of `psHeader.data` (XML: file id, RCDB version) + the raw radio file in the same container format `hyt` reads (Comm_Basic: 408 bytes, same layout, header CRC field differs). The owner's 4 CPS templates are in `Templates\` |
| Export template parameters / parameter info (Excel) | en-US `ExportTemplateParams`, `ExportParamInfos`, `ParamName/ParamDesc/NormalValue/ParmaSize` | partial (`hyt decode --labels`, `hyt show --all` JSON/text) | code | the SDF `Param.description_en` is empty, so CPS has no help texts to export (observed) |
| Template upgrade to a newer RCDB / model conversion | en-US `NotExistsOldJson…`, `TemplateConversion`; `G2CPS.BusinessDataUpgradeService.dll`, `G2CPS.FileDataUpgradeManage.dll` | missing | other RCDB versions | only matters after a firmware/CPS update |
| Contact import/export (Excel): Contacts + RX group lists + dial rules | `ExportConfig.config` (Conventional: blocks 3, 607; params 1082, 116, 1444, 1445); `DataIntegrityConfig.config`; en-US `ImportTitle`/`ExportTitle`, `ModeTypeWrong`, `RcdbWrong`; `G2CPS.Office.exe` (NPOI) | missing | code (CSV/JSON offline); recording for adding contacts | import checks model, RCDB version, language |
| Import first-generation / older CPS data | `Config\ImportExport\DataUpgrade\*` (trunking, TMO/DMO, conventional V2009 maps) | missing | – | not needed |
| List editing: add, insert, delete, batch add, batch edit, copy/paste, up/down, clear | en-US `Add`, `InsertRow`, `BatchAdd`, `BatchEdit`, `Copy`, `Paste`, `UpRow`, `DownRow`, `EmptyData` | partial (add/remove channels, zone member add/remove/move) | recording (reorder, delete of others); code (batch/copy on top of add) | |
| Validation: ranges, steps, lengths | SDF `DefaultRangeValue` | **done** (`apply_edits`) | – | |
| Validation: **model-number ranges/defaults/visibility** | SDF `ModelNumberMainGroup` (model-number character → group), `ModelNumberRangeGroupLink` (4108 rows), `ModelNumberVisibleGroupLink` (379), `ModelNumberDefaultGroupLink` (346) | missing | code (offline) | e.g. RX/TX Frequency has 22 model-specific range rows. `hyt` accepts the generic 350–527 MHz (observed in options.json); the U1 band is 400–470 MHz (inferred from `Update_CT\ModelListG2PT3.txt`) |
| Validation: link rules (grey out / restrict dependent fields) | SDF `LinkMain` (165 rules, e.g. "RX Only on → grey TX options"), `LinkInitiative*`/`LinkPassivity*`; `ParamBaseTriggers.Config`; unprotected `G2CPS.Link.dll` | missing | code (offline research) | |
| Validation: uniqueness and cross-table checks | `TableRowAddHandleConfig.config`, `DataCheckConfig.config`, `DataValidationConfig.config` | partial (zone→channel refs, list lengths) | code | |
| References shown by name | `DataValidationConfig.config` `ParamDependencyItem`, `ComplexRuleConfig.Config` | partial (`refs.rs`: TX contact, RX group list, scan/roam, analog emergency) | code (offline) | **observed** for the BP515: RX Group List → 607 table index 1-based, 0 = None; digital Emergency System → 636 and Phone System → 824, 1-based, 0 = None; RRS Channel / Location Info Revert Channel → digital channel 1-based, 0 = selected channel, 65535 = None; analog Scan List → 619, analog Emergency System → 637. This confirms most of open question 5b. Digital `Scan List Roam List` is not in this file |
| Parameter search | en-US `CpsSearchParamKey`, `SearchMapTitle` | partial (`settings::search`, GUI being built) | code | |
| Programmable keys picture | en-US `ShowButtonImage` | missing | – | cosmetic |
| Radio information (versions, SN, band, last programmed) | Help 2.6; en-US `RadioDataVersion`, `BootloaderVersion`, … | **done** (`hyt info`, manifest; settings view) | – | |
| Licence / feature authorization (licence file → radio, then restart) | Help 2.3; en-US `Authorization`, `LicenseNotFound`, `AuthorizationItem` | missing | owner + recording | changes licensed features; high risk |
| Reset radio to defaults | Help 2.4; en-US `Confirm_Reset`, `Reseting` | missing | owner + recording | |
| Firmware upgrade | Help 2.5; `Update_CT\FlashBurn.exe`, `setAP_BP.xml` (L3boot, Tuning Data, DSP_main, FixedString, Font, Image, Voice, PowerOnOff UI, RCDB, SDF, BT, MenuXml) | missing | owner | separate tool; recommend leaving it to CPS |
| Reading a password-locked radio (password prompt) | en-US `NeedPassword`, `Password_Empty`; Password page | missing (protocol unknown) | owner + recording | the BP515 is not locked (inferred, reads work without a prompt) |
| Batch programming tasks (radio list, post-program radio ID auto-increment) | en-US `CreateTask`, `TaskMenuName`, `Terminal`, `Post-ProgramIDAutoINC` | missing | code | fleet feature, low priority |
| OTAR air keys, APK/APH, GCK, AD-domain import, NRM | en-US `ImportOTAR`, `ImportAPHAPK`, `ImportSGCK`; `ImportTrunkAirKeyCLR.dll`, `G2CPS.NRMService.dll` | n/a | – | trunking, Android and network-management features (inferred: not for the BP515) |
| Language switch (EN/ZH), model-number management | en-US `Chinese`, `ModelNumbersImport` | n/a | – | |
| Compare two configurations | not found in CPS (only a `Compare` symbol in `G2CPS.Special.dll`) | `hyt diff`, GUI change list | – | `hyt` goes beyond CPS here |
| Print | not found | – | – | |

## Gap list (top 15, by practical value)

| # | Gap | offline? | hint |
|---|---|---|---|
| 1 | Finish the GUI settings view (all S pages editable, with search) | offline | `hyt-core/src/settings.rs` + `hyt-gui` `get_settings_*`/`app.js` (in progress) |
| 2 | Model-number ranges/defaults/visibility (frequency band, power levels, key functions) | offline (owner check on one CPS screen is useful) | extend `tools/export-rcdb-options.ps1` with `ModelNumber*` tables; match the manifest model code in `hytera-codeplug/src/options.rs`; validate in `edit.rs` |
| 3 | Load references from `DataValidationConfig.config` / `ComplexRuleConfig.Config` at run time (adds Phone System, digital Emergency System, RRS/location revert channel, emergency revert, scan priority channels; 0/65535 specials) | offline | `hytera-codeplug` loader (like `TableLink`) → replace the hard-coded table in `hyt-core/src/refs.rs`; update open question 5b |
| 4 | Generic list views for the remaining lists (RX group lists + aliases, scan, roam, phone list/system, one-touch call, preset channel, quick message, HDC1200, 2-Tone), value edits and members shown by name | offline | `hyt-core/src/tables.rs`: table specs from `UiGroupBlockLink` + layout. Two-level lists like zones (alias file + member file) |
| 5 | Link rules (greying out, dependent values) | offline research | export `LinkMain`/`LinkInitiative*`/`LinkPassivity*`; read `G2CPS.Link.dll` (unprotected); apply in `tables.rs` editor state + `edit.rs` |
| 6 | Add/delete **contacts** like CPS (uniqueness, `delete flag`, RX group/one-touch references) | needs owner: recording of CPS adding and deleting a contact | `edit.rs` record rules; `workspace.rs` add/remove contact |
| 7 | Add/delete **zones** (alias file 7 + member file 6 together) | needs owner: recording | `workspace.rs` `add_zone`/`remove_zone`; `edit.rs` paired-file rule |
| 8 | Add/delete **scan / roam / RX group lists and their members** | needs owner: recordings | same pattern as zone members (`workspace.rs`), alias/config files 85, 39, 81 |
| 9 | Remove channels that are referenced (scan/roam members, emergency revert, presets, current channel; open questions 3/5c) | needs owner: recording of deleting a scan-listed + designated channel | `edit.rs` renumbering (`channel_renumbering`), drop the refusals |
| 10 | CPS `.tpl` import/export (open CPS backups in `hyt`; hand `hyt` configurations to CPS) | offline (import); owner check that CPS opens an exported file | new `hyt-core` module (zip of zips; container header as in `container.rs`); CLI `hyt import-tpl/export-tpl`; GUI Open |
| 11 | Reorder channels/contacts (up/down), copy/paste, batch add/edit | reorder needs owner: recording; batch/copy offline once add works | `workspace.rs` move + renumbering; GUI table actions |
| 12 | Contact CSV import/export (and optionally CPS's Excel layout) | offline (after 6) | `hyt-core` import module using `ExportConfig.config` block/param lists; CLI + GUI |
| 13 | Encryption key list (242) editing and writing | needs owner: recording of CPS adding a key (test key only) | check `KeyEncryptCLR.dll`; profile `write_excluded_files` |
| 14 | Template library in the GUI (recent folders, name/description, clone) | offline | `hyt-gui` + manifest fields |
| 15 | Password locks, licence authorization, reset, reading a locked radio | needs owner (recordings; risky) | `hytera-proto` session commands; keep firmware upgrade out of scope |

**Surprises worth noting:** CPS's `.tpl` templates contain the raw radio files, so CPS interoperability is cheap.
`DataValidationConfig.config` already states the reference encoding `refs.rs` had to infer. Frequency and
other ranges in CPS depend on the model number, while `hyt` uses the generic 350–527 MHz range. The radio
holds 15 XPT files that CPS shows no page for on this model.
