# Radio settings files ("codeplug") and the RCDB layout database

The radio stores its settings as numbered **files**. CPS reads and writes them whole over the serial protocol
([protocol.md](protocol.md)). Their layout is described by Hytera's **RCDB** (radio configuration database), which
ships as plain files inside CPS. Everything below was checked against the 58 files read from the BP515 on
2026-11-18 (`captures/read_2026-11-18_files/`).

## 1. Choosing the right RCDB version

The radio reports its layout version as info item 2 (`V1.5.24.000` for our BP515). CPS ships one layout per
version, so pick files by that string:

| File in `C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003\` | Contents |
|---|---|
| `RCDB\FileID\<ver>.xml` | Files CPS reads/writes for this version (ID + name), 58 for V1.5.24.000. Also in `RcdbXmls\<ver>.xml` (89 versions) |
| `RcdbJsons\<ver>.txt` | Field-level layout, JSON (104 versions; V1.5.24.000 has 1,572 fields over 65 files) |
| `RCDB\TableLink\TableLink.xml` | Pairs each two-level list file with its alias file (e.g. zones 6 ↔ zone aliases 7) |
| `RCDB\DMR\<FileName>` | One JSON layout per file name, for the CPS install's own version |
| `RCDB\Revison.txt` | `RCDB_Revision:V1.5.24.000` |

## 2. Field layout JSON (`RcdbJsons\<ver>.txt`)

Top level: `{"detail": [...], "radio_info": {...}, "createtime": ...}`. Each `detail` entry:

| Key | Meaning |
|---|---|
| `Id`, `Name` | Parameter ID and English name (e.g. `RX Frequency`) |
| `File_Id`, `File_Name` | Which radio file holds it |
| `Block_Id`, `Block_Type` | Record structure the field belongs to (see below) |
| `Offset` | Byte offset inside the record |
| `Type` | `Byte`, `Char`, `Short`, `Ushort`, `Int`, `Uint`, `Float`, `Bit`, `AsciiString`, `UnicodeString` |
| `Length` | Element count. For strings it is characters: a `UnicodeString` of `Length` 16 takes 32 bytes |
| `Bit_Start_Pos`, `Bit_Field_Len` | For `Bit` fields: `Bit_Field_Len` = size in bytes (1 or 2) of the little-endian word at `Offset`, `Bit_Start_Pos` = first bit (0 = least significant, confirmed), `Length` = number of bits. `-1` for other types |
| `DefaultValueEn/Cn` | Default value (strings as hex of their UTF-16LE bytes) |
| `RangeEns/Cns`, `ChildMember` | Always empty in V1.5.24.000; allowed values / option names are not in this file |

`Block_Type` values seen: **0** = a single fixed record (930 fields), **1** = a list of identical records such as
channels and contacts (621), **2** = a two-level list such as zones and roam lists (21).

Example, file 3 `QEU_C_DChannel` (digital channels, block type 1): `Channel Alias` UnicodeString at offset 0
(16 chars), `Channel Type` Byte at 32, `Power Level` Bit at 33, `RX Frequency` Int at 36, `TX Frequency` Int at 40, …

Strings inside files are **UTF-16LE**, and numbers are **little-endian** (unlike the big-endian protocol).
`Char` is a one-byte number (levels, timeouts, flags), `Short`/`Int` are signed, `Byte`/`Ushort`/`Uint` unsigned.
Fields with `Length` > 1 (other than strings) are arrays.

Checked against real values (BP515, 2026-11-18): frequencies are `Int` in **Hz** (analog channels named
`446.13125` store 446131250). For every record of all 58 files the fields cover every byte exactly (decode
round-trip test). Bit numbering is LSB first (confirmed against CPS's channel screens); option values: see 2a.

## 2a. Option names, units and scaling (`XMLToolCompactData.sdf`)

The RCDB JSON has no option names. They live in CPS's parameter database `XMLToolCompactData.sdf` (SQL Server
Compact 4.0; the runtime is installed with CPS), from which the RCDB files are generated (`DBVersion`: current
RCDB V1.5.24.000, tool V3.0.07.011). Key tables:

| Table | Content |
|---|---|
| `Param` (1,624) | `id` = the RCDB field `Id`; `alias_en`, `param_unit` (e.g. `MHz`, `Hz`, `ms`), `default_value_en`, `ui_type`. `delete_flag = 1` does **not** mean absent (163 such parameters are in the V1.5.24.000 layout) |
| `DefaultRangeValue` (7,905) | Options per `param_id` (the row key of `Param`, not `Param.id`); `language_type` 1 = English, 0 = Chinese; `range_value` text; `Index` = display order; `different_version` = model/firmware variant (0 = always) |
| `UiCategory`, `UiPage`, `UiGroup`, `Ui*Link` | How CPS arranges parameters into screens and groups (useful for a GUI) |
| `Block`, `File`, `FileBlockLink`, `BlockParamLink`, `TableTwoDLinkData` | The same structure as the RCDB files |
| `Link*` | Rules between parameters (also in `ParamTemplate.config`) |

`range_value` has two shapes (all 3,552 English options parse):

- `RAM:<raw>,UI:<label>`: a named choice, e.g. `RAM:0,UI:High`, `RAM:1072,UI:107.2` (CTCSS 107.2 Hz).
- `RAM:[a,b],UI:[c,d],Step:s`: raw a..b shown **linearly** as c..d, e.g. frequency `RAM:[350000000,527000000],
  UI:[350,527]` MHz, TX timeout `RAM:[3,99],UI:[15,495]` s (×5), RSSI `RAM:[75,115],UI:[-75,-115]` dBm.

Some `different_version` variants add choices (zone member type 255 = None) or give alternative ranges (some
gains). `hyt` prefers variant 0 and falls back to others.

Confirmed meanings for the BP515's settings: digital power 0 = High, 2 = Low; slot 0/1/2 = Slot 1 / Slot 2 /
Pseudo Trunk; call type 0/1/2/255 = Private / Group / All Call / None; zone member type 0/1/2/255 = Digital /
Analog / XPT / None and channel number 1-based (1–1,024); analog spacing 0/1/2 = 12.5/20/25 kHz; tone type
0–3 = None / CTCSS / CDCSS / CDCSS Invert; **`TX Contacts ID` is a call ID** (1–16,776,415), not a contact-list
position.

`tools/export-rcdb-options.ps1` exports names, units and English options to
`%LOCALAPPDATA%\hyt\rcdb\<version>\options.json` (portable JSON, Hytera data: never in the repository). `hyt show`
loads it (`hytera-codeplug/src/options.rs`).

## 2b. Model-number rules (ranges, defaults, visibility by model)

CPS narrows the generic options of 2a by the radio's **model number**: info item `0x0000` ("model code"), e.g.
the BP515's `55P10B00-1L0BAPU1U100000000CB000000000000-00000NN0000000000000000000000000-00040000` (observed in
every `manifest.json`; CPS's own `ModelNumber.csv` lists 49 sample models in the same format, e.g. `BP510 U1` =
`50P10B00-1L0BAPU1U1…`). Model name (0x0011) and product type (0xFFF0) play no part in these rules (inferred: the
conditions only index model-number characters).

**Positions** (observed, CPS's `XML Model Number Rule.xml`): 1-based character positions in the model number
**without the `-` separators** (8 + 32 + 32 + 8 = 80 characters). Relevant ones: 1 level/series (`5`), 2 sales
region (`5` = EMEA), 3 type (`P` = portable), 4 "ID" (display/keypad), 14 NB protocol standard (`P` = "QEU",
`D` = "ENS", `T` = "TFTSB"), 15–16 hardware band, **17–18 "flash burned" band** (`U1` = 400–470 MHz, `U3`/`U9` =
350–400, `V1` = 136–174, `Ux` = 350–527, `B1` = 446 MHz, …), 19 display, 20 GPS module, 21 Bluetooth, 70
customization. That the SDF's `start_index` uses the same numbering is inferred from the data: positions 1/2/4/6
hold values that only fit that way, 17 (length 2) holds band codes, 21 holds the Bluetooth `A` of the `… BT`
models in `ModelNumber.csv`.

**Tables** (observed, `XMLToolCompactData.sdf`; `param_id` = `Param.param_id`, not the layout id):

| Table | Rows | Content |
|---|---|---|
| `ModelNumberMainGroup` | 1,070 | Conditions: `model_number_group_id`, `start_index`, `length`, `value` (comma-separated, trailing comma, e.g. `U1,` or `P,D,`), `param_id` (the same for all rows of a group); `link_type` always 0 |
| `ModelNumberRangeGroupLink` | 2,054 per language | `default_group_id` → allowed values for `param_id`: same `range_value` text and `Index` order as `DefaultRangeValue`; `prefix` always empty |
| `ModelNumberDefaultGroupLink` | 346 | Default per group and parameter (`default_en`), e.g. `429075000` for RX/TX frequency on U1, `400-470` for Comm_Basic "Frequency Range" |
| `ModelNumberVisibleGroupLink` | 379 | `is_visible` 0/1 per group and parameter |
| `ModelNumberEnableGroupLink`, `ModelNumberDescription`, `ProductModelNumberList`, `LinkModelNumber` | 0 | empty in this CPS version |
| `ModelNumberTypeRule` | 1 | `RuleType 0`, `ModelRuleType 0` (inferred: `G2Terminal` in `Config\ModelNumberRuleTypeInfo.xml`, i.e. the "XML" rule file, not the RVM one) |
| `ModelNumberUIMainGroup` / `ModelNumberUILink` | 125 / 76 | The same kind of conditions for screen parts (`related_linkType`/`link_type` 1 or 2, `relatedUI_id`, `property_value` 0/1). Meaning of the link types **unknown** (the ids fit `UiPage`/`UiGroup`/`UiCategory` inconsistently); exported, not used |

**Matching** (inferred): a group applies when all its conditions hold; its range rows then **replace** the
parameter's generic options (they repeat the generic entries minus the excluded ones, or give a narrower scale).
For the BP515 U1, 68 of 586 groups match and no parameter gets more than one range/default/visibility group;
across the 49 models of `ModelNumber.csv` only the AP320 has an overlap (two visibility rows). `hyt` takes the
lowest group id then; CPS's rule is unknown.

**What applies to the BP515 U1** (observed in the data; that CPS's screens behave so is inferred, not yet compared):

- RX/TX Frequency of digital (layout ids 7, 8) and analog (84, 85) channels, and four other RX/TX pairs
  (3299/3300, 3305/3306, 6512/6513, 6568/6569): **400–470 MHz**, step 5 Hz (generic: 350–527 MHz). Default
  429.075 MHz (the frequency of CPS's channel template). Comm_Basic "Frequency Range" default `400-470`.
- In Call TX Admit (38): Follow TX Admit / Color Code Free / Priority Interrupt (no "Always Allow").
- Emergency Type/Mode (1607, 1608, 1623), Software Encrypt Type (1726: Hytera Encrypt only), Contact Switch Type
  (1727), SK3 Short/Long key functions (6105/6106: 21 of 27).
- Defaults for 30 parameters (e.g. Language 1033, several tones on) and visibility for 47 (e.g. VOX hidden).
- Every value in the radio's current configuration lies inside these ranges (`hyt-core/tests/model_ranges.rs`).

`tools/export-rcdb-options.ps1` writes these tables (with layout ids) to `models.json` next to `options.json`
(Hytera data: never in the repository). `hytera-codeplug/src/models.rs` evaluates them; `Options::for_model` /
`with_model` narrows the catalogue (a value outside the model's range keeps its label for display but is refused
by `allows`/`parse_display`); `hyt_core::load_options_for_read` takes the model number from a saved read's
manifest (used by `Workspace::open` and `hyt apply/show/decode/diff`; without `models.json` or a model number it
notes this and uses the generic ranges). Defaults and visibility are loaded (`Options::model_default`,
`model_visible`) but not yet used by the GUI or when adding records.

## 3. File container (little-endian)

Every file starts with a 20-byte header:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | File ID (`Comm_RadioInfo` has an extra high bit: `0x008000FD`) |
| 4 | 4 | Version / record count (meaning unconfirmed; 1 for most list files) |
| 8 | 4 | Logical size. Equals the transferred size for flat files; larger for list files (channels: 1,822 bytes sent, 21,050 logical) |
| 12 | 4 | CRC-32 (zlib/IEEE) |
| 16 | 4 | Unknown. The radio returns non-zero values for 18, 46 and 5; CPS always writes 0 |

**CRC (zlib/IEEE CRC-32 of the bytes after the header):**

- **Every file CPS writes** has `crc32(file[20:]) == header CRC` (55 of 55, observed).
- **As read from the radio**, 48 of 58 files satisfy the same rule. Of the other ten:
  - List files 3, 6, 7, 9 come back with **stale bytes after their real end**: exactly one leftover record
    (76 bytes for a digital channel, 2,570 for a zone). The real end is the end of the block area (section 4),
    and `crc32(file[20:real_end])` matches. CPS writes them trimmed (e.g. channels 1,822 → 1,746 bytes).
  - 18, 46, 5 (multi-block files with non-zero header bytes 16–19): the read CRC matches no rule found. CPS
    writes them with bytes 16–19 = 0 and a CRC over the body. For 46 and 5 that is the only change.
  - 242, 245, 253 match no rule (242 and 245 even share CRC `0x76344191`), but CPS never writes them.

## 4. Blocks and records

Decoded 2026-11-18 and verified on all 58 files (directory, block IDs = the RCDB's blocks for that file, sizes,
two-level flag = RCDB block type 2). Implemented in `crates/hytera-codeplug/src/structure.rs`.

**After the file header** (little-endian):

| Offset | Size | Field |
|---|---|---|
| 20 | 8 | Zero |
| 28 | 4 | `n`: total length of the block bodies |
| 32 | 6 × k | **Block directory**: `(u16 blockId, u32 offset)`; offsets count from the end of the directory. `k` is not stored: the directory ends where the first block body starts (it begins with the first entry's block ID). `k` = 1 for most files, up to 11 (`QEU_C_Config`) |
| 32 + 6k | n | Block bodies. The real content ends here; anything after it is a stale tail |

**Each block body:**

| Offset | Size | Field |
|---|---|---|
| +0 | 2 | Block ID |
| +2 | 2 | Capacity (maximum records). Bit 15 set for two-level lists (block type 2) |
| +4 | 2 | Unknown; equals the record count in every file seen, except empty lists (0) |
| +6 | 2 | Record count |
| +8 | 8 | ASCII short name (`Digital `, `Zone`, `Conv Con`, …) |
| +16 | 4 | Record size |
| +20 | 6 × capacity | Slot index `(u16 slot, u32 slot × recordSize)`, always full and in order |
| … | count × size | The records of slots 0 … count−1 |

So `n` of a single-block file = 20 + 6 × capacity + count × recordSize (e.g. 256 digital channels capacity,
2 used, 76 bytes each: 1,708). The header's logical size is the size with every slot used.

**Records of two-level blocks** (zones, RX group lists, scan lists, roam lists, site lists) each hold a member list:

| Offset | Size | Field |
|---|---|---|
| +0 | 2 | Unknown (0) |
| +2 | 2 | Member capacity (256 for zones) |
| +4 | 2 | Member count |
| +6 | 2 | Member size (4 for zones) |
| +8 | 2 | Unknown (0) |
| +10 | 6 × capacity | Member slot index, as above |
| … | capacity × size | Member slots; the first `count` are used |

The RCDB fields of a type-2 block describe one **member** (zone member: `Channel List` u16 at 0 = channel number,
`Channel Type` u8 at 2). Each record pairs with the record of the same index in its one-level partner file
(`TableLink.xml`: zone 1's name is record 0 of `QEU_C_ZoneAlias`).

Zone member values seen: `1/0, 1/1, 2/1` = Digital 1, Analog 1, Analog 2 (meanings from CPS's database, §2a).
CPS shows them as the channels' names (APTLK_CH1, 446.13125, 446.18125): `Channel List` is the 1-based number
within the digital or analog channel list selected by `Channel Type` (confirmed on CPS's Zone screen).

### Adding records and members (observed 2026-11-18, `captures/add-channel-*`)

CPS added a digital channel and put it into Zone 1. What changed:

- **Digital channel file (3):** the new record is appended after the last one (slot 3); the block header's count (+6)
  **and** the field at +4 go from 2 to 3; the body length at offset 28 grows by one record (76); the slot index, the
  file header's logical size and version stay unchanged; there is no stale tail; CRC recomputed.
- **The new record is a copy of the channel template** in `QEU_C_Template_DChannel` (252): name `CH D1`,
  429.075 MHz, etc. (`QEU_C_Template_AChannel` 251 is the analog template). The template differs from the RCDB
  defaults (default frequency 400.25 MHz), so CPS uses the template file, which the user can edit in CPS.
- **Zone file (6):** only the zone record's member count (3 → 4) and the next member slot (`03 00 00 00` = Digital
  3) change. Member slots are pre-allocated, so the file size stays the same.
- No other file changed for the channel (18: date stamp; 80 and 96: see below).

`hyt` does the same (`structure::rebuild`, `rebuild_sublist`; new channels from the template, other records from
the RCDB defaults) and reproduces CPS's files 3 and 6 byte for byte (`tests/add_records.rs`).

### Removing a channel (observed 2026-11-18, `captures/delete-channel-*`)

CPS deleted digital channel 3 (`CH D1`, a Zone 1 member) while `HYT Test 4` was channel 4:

- **Channel file:** the record is removed and later records shift down (new record 3 = old record 4, byte for
  byte); count and +4 field 4 → 3; body length −76; slot index and logical size unchanged.
- **Zone:** the member pointing to the deleted channel is removed and members pointing to later channels of the
  same type are renumbered (Zone 1: D1, A1, A2, D3, D4 → D1, A1, A2, D3); member count 5 → 4; the freed slot is
  zeroed.
- Nothing else changed (apart from CPS's own noise in files 80 and 96).

Other fields that can point to conventional channels (none were affected in this recording, so CPS's handling is
unknown): scan list members (`Channel List` + type; 65535 = "Selected"), roam list members, scan config
`Designated Channel` / `Priority Channel 1/2` (+ type fields), digital/analog emergency `Revert Channel`
(0 = Selected), `QEU_C_Config` `Power On Channel` / `Current CH Address`, `QEU_DeveloperConfig`
`Stun Channel List`, digital channels' `RRS Channel` / `Location Info Revert Channel`, and preset channels
(`Zone Index`, `Channel Index`; 65535 = none). Values 0 and 65535 are placeholders, not channel numbers.

`hyt` removes channels like CPS (zones updated automatically) and reproduces CPS's files 3 and 6 byte for byte
(`tests/delete_channel.rs`). It refuses a removal when any of the other references above points to a channel whose
number would change or disappear, or when presets are set. Removing other kinds of records (contacts, zones, …) is
not supported yet.

**References to other lists (inferred 2026-11-19, not yet confirmed on CPS's screens; open question 5b):** the
RCDB ranges of list references run from 0 to the list's capacity (digital `RX Group List` 0–32 with 32 RX group
lists, `Scan List Roam List` 0–64 with 64 scan lists, analog `Scan List` 0 = None, 1–64), so they are read as
1-based positions in the named list (`QEU_C_RxGrpAlias`, `QEU_C_ScanConfig`, `QEU_C_RoamConfig`,
`QEU_C_AEmergency`) with 0 = None. The BP515's digital channels have `RX Group List` 1 with one RX group list
(`Rx Group List 1`), consistent with this. A digital channel's `Scan List Roam List` points into the scan or the
roam lists according to its `Use Scan or Roam List` (0 None, 1 scan, 2 roam). `TX Contacts ID` is a call ID; the
GUI names it after the contact with that call ID (both channels hold 10, which no contact has).

**List pairs and their members (2026-11-20):**

- *Observed* (layout and the BP515's files): an RX group list member (`QEU_C_RxGrpList` 80, block 607) is
  `Call ID` (Uint, offset 0) + `Reserved 01` (Uint); scan list members (84) are `Channel List` (Ushort) +
  `Channel Type` (Byte, 0 Digital / 1 Analog) like zone members, with 65535 = "Selected"; roam list members (38)
  are `Channel List` only (65535 = "Selected"). Each list record has 16 member slots (scan, roam and RX group).
  The paired files have **different capacities**: RX group lists 16 (members, 80) vs 32 (aliases, 81), scan
  lists 8 (members, 84) vs 64 (settings, 85), roam lists 8 vs 8. The GUI shows the smaller as the capacity.
  The BP515 has one of each: `Rx Group List 1` (0 members), `Scan List 1` and `Roam List 1` (one member each,
  "Selected").
- *Inferred*: an RX group list member's `Call ID` refers to the contact with that call ID (like a digital
  channel's `TX Contacts ID`); the GUI names it only when that is unambiguous (one contact with the ID, or one
  group-call contact among several). Not checked on CPS's screen (this radio's list is empty).
- *Unknown*: what the scan config's `Designated Channel` (RCDB range 1–1,024 + `DesignatedCH ChType`) and
  `Priority Channel 1/2` (0 None, 65535 resp. 1 "Selected", 1–31 resp. 1–30 + type fields) count: channel
  numbers or positions in the scan list. They are shown as numbers. Likewise HDC1200 contacts' `HDC System`,
  2-Tone contacts' `TwoTone System` and 2-Tone personalities' `Call List n` (position vs. the systems' own
  `System ID`/`SystemID` fields) are shown as numbers.
- *Observed* (`ui.json`): CPS shows these lists on Conventional › Basic Setting (One Touch Call, Preset
  Channel), Digital Setting (Quick Message; Encrypt = E2EE key list 242, never written), Contact (RX Group
  List, Phone List), Scan, Roam, Phone (Phone System) and Analog Service (HDC1200, 2-Tone). Call logs (74, 243,
  244), Bluetooth pairings (245) and the XPT (`PTE`) files are on no BP515 screen.

**Unused slots hold junk:** `QEU_C_RxGrpList` (80) has 16 member slots and 0 members; CPS writes leftover memory
into the unused slot 0, which is why bytes 260–267 differ on every CPS write. It has no meaning.

**CPS quirk in `QEU_C_AEmergency` (96):** on this write CPS stored `Emergency ID` 1 as `00 00 00 01`
(= 16777216 little-endian) in all three records, which it did not do on its earlier unchanged write. Meaning unknown.

## 4a. Preparing files for writing

What CPS sends when writing back an unchanged read (implemented as `prepare_for_write` in
`crates/hytera-codeplug/src/container.rs`; reproduces 53 of CPS's 55 files byte for byte from our own read):

1. Files with a stale tail are cut to the end of their block area (section 4).
2. Header bytes 16–19 are set to 0.
3. The header CRC is set to `crc32(file[20:])`.

CPS also changes content on its own, even when nothing was edited:

- **`Comm_Basic` (18)**: field `Programming Time` (block 902 `Device Info`, ASCII `YYYY-MM-DD`, file offset 306;
  next to `Frequency Range` = `400-470`) is set to today. The radio reports this date as info item `0xFFF6`.
- **`QEU_C_RxGrpList` (80)**: bytes at file offset 260–267 change. They are the unused member slot 0 of the only
  RX group list (0 members), which CPS fills with leftover memory; meaningless (section 4).

Our tool does not make these two changes yet: it writes back exactly what was read.

**Sensitive content:** `Comm_Basic` block 896 holds the radio's `Write Password` and `Read Possword` (sic) as
text (`89500000` on this radio). No password is exchanged over the cable, so CPS presumably checks them itself
after reading (inferred). File 242 is the E2EE key list (empty on this radio). `hyt show` never prints these;
decoded JSON contains them.

## 5. The 58 files read from the BP515 (RCDB V1.5.24.000)

Comm_Basic 18, Comm_UI 19, Comm_GPS 86, Comm_Hardware 46, Comm_Selfdefinetone 94, Comm_Service_Setting 87,
Common_DeveloperConfig 97, QEU_C_DChannel 3, QEU_C_Config 5, QEU_C_Zone 6, QEU_C_MissCallLog 74,
QEU_C_RxGrpList 80, QEU_C_RxGrpAlias 81, QEU_C_ZoneAlias 7, QEU_C_DContact 8, QEU_C_AChannel 9,
QEU_DeveloperConfig 34, QEU_C_RoamList 38, QEU_C_RoamConfig 39, QEU_C_ScanList 84, QEU_C_ScanConfig 85,
QEU_C_DEmergency 95, QEU_C_AEmergency 96, QEU_Public 104, QEU_C_PTEPersonality 157, QEU_C_PTEInBox 158,
QEU_C_PTECallLog 159, QEU_C_PTEContact 160, QEU_C_PTERXGroupList 161, QEU_C_PTERXGroupAlias 162,
QEU_C_PTEVoiceChannel 163, QEU_C_PTEDataChannel 164, QEU_C_PTESiteVoiceListInfo 165, QEU_C_PTESiteVoiceList 166,
QEU_C_PTESiteDataListInfo 167, QEU_C_PTESiteDataList 168, QEU_C_PTERoamList 179, QEU_C_PTERoamlistconfig 180,
QEU_C_PTEEmergency 183, QEU_C_OneTouchCall 200, QEU_C_PhoneList 212, QEU_C_PhoneSystem 213, QEU_C_PTEQGPSSystem 219,
QEU_C_PTEOneTouchCall 230, QEU_C_PresetChannel 241, QEU_C_E2EEKEYLIST 242, QEU_C_IncomingCallLog 243,
QEU_C_OutgoingCallLog 244, QEU_C_BTMatchedList 245, QEU_C_HDC1200_System 246, QEU_C_HDC1200_Contact 247,
QEU_C_2Tone_Contact 248, QEU_C_2Tone_System 249, QEU_C_2Tone_Personality 250, QEU_C_Template_AChannel 251,
QEU_C_Template_DChannel 252, Comm_RadioInfo 253, QEU_C_Quick_Message 254.

CPS writes back 55 of them, including the call logs and inbox; it skips 242 `QEU_C_E2EEKEYLIST`,
245 `QEU_C_BTMatchedList` and 253 `Comm_RadioInfo` (see [protocol.md §5](protocol.md#5-full-write-sequence-used-by-cps)).

## 6. CPS template files (`.tpl`)

CPS keeps its template library in `Templates\` of its install folder: one `<Guid>.tpl` per template plus an index
`TemplatesXML.xml` (an `ArrayOfTemplateBase` holding a copy of each template's header). Everything below is
**observed** in the owner's four templates (saved 2026-11-18 from the BP515: "Vana", "Backup", "2026-11-18-1",
"2026-11-18-2") unless marked otherwise. Code: `crates/hyt-core/src/tpl.rs`; CLI `hyt tpl info|import|export`.
CPS's archive code is `G2CPS.Archives.dll` (not protected; uses DotNetZip, `Ionic.Zip.dll`). Its strings name
`tpHeader.data`, `psHeader.data`, `.hps`, `.tpl`, and also `tsHeader.data` / `.tsk` / `Tasks` (not seen yet).

**Outer ZIP** (`.tpl`): 59 entries, all deflated, no comment, no data descriptors, no UTF-8 flag. Entry 1 is
`tpHeader.data`; then one `<FileName>.hps` per radio file (58, including 242, 245 and 253, which CPS never
writes to the radio). Order: the RCDB file list order (= `hyt read` order), except that the eight one-level files
of `TableLink.xml` (7, 81, 39, 85, 162, 165, 167, 180) come last, in the order of their two-level partners
(6 → 7, 80 → 81, 38 → 39, 84 → 85, 161 → 162, 166 → 165, 168 → 167, 179 → 180). Every entry (outer and inner):
version made by 4.5, needed 2.0, MS-DOS attributes `0x20` (archive), DOS time in local time, and a 36-byte NTFS
extra field (`0x000A`; mtime/atime/ctime in UTC, the moment of saving) in both the local and the central
header: DotNetZip's defaults.

**`tpHeader.data`** (XML `TemplateBase` in .NET `XmlSerializer` style: `<?xml version="1.0"?>`, no BOM, CRLF line
ends, two-space indent, trailing CRLF, empty strings as `<X />`). Elements in this order, with the BP515's values:

| Element | Value | Notes |
|---|---|---|
| `OperatePermission` | `All` | |
| `OperateType` | `Professional` | |
| `IsCreate` | `false` | inferred: `true` for a template created in CPS without a radio |
| `Guid` | e.g. `53994d0e-f71c-480c-ab36-396cf586b9fa` | = the `.tpl` file name |
| `Description` | empty | |
| `Product` | `BP515 U1` | = info item `0x0011` (model name) |
| `ModelNumber` | `55P10B00-1L0BAPU1U1…-00040000` | = info item `0x0000` (model code) |
| `Name` | e.g. `Vana` | the name the owner typed |
| `RcdbVersion` | `V1.5.24.000` | = info item `0x0002` |
| `Frequence` | `U1` | the band suffix of the model name (inferred) |
| `CreatedTime` | e.g. `2026-11-18T19:17:15.0656269+02:00` | local time with offset; up to 7 fraction digits, trailing zeros dropped |
| `ProductId` | `0` | |
| `SerialNumber` | `R23N141382` | = info item `0x000B` |

**Each `.hps`** is itself a ZIP: `psHeader.data`, then, for a two-level list file (6, 80, 38, 84, 161, 166,
168, 179), a copy of its one-level companion (e.g. `QEU_C_Zone.hps` holds `psHeader.data`, `QEU_C_ZoneAlias`,
`QEU_C_Zone`), then the radio file, named exactly like the `.hps` without extension. The embedded companion
copies are byte-identical to the companion's own `.hps` in all four templates. **`psHeader.data`** (XML
`ParamStore`, same style):

| Element | Value |
|---|---|
| `Seqo` | CPS's Chinese description of the file plus `1`, e.g. `整机公共1` (Comm_Basic), `常规数字信道1` (DChannel): `File.description` in `XMLToolCompactData.sdf` (observed by query). **Absent** for the eight one-level files |
| `RcdbVersion` | `V1.5.24.000` |
| `Key` | the file ID (decimal); always equal to the ID in the radio file's own header |
| `Type` | `0` |
| `ModelNumber` | empty |
| `CreateTime`, `ModifiedTime` | local times, equal in all but one store (1 ms apart). Main files carry about the time the template's data was taken, the one-level files a time 20–80 s later, at saving (inferred) |

**The radio files inside** are in the form CPS writes to the radio (section 4a): every file satisfies
`crc32(file[20:]) == header CRC` with header bytes 16–19 = 0, and list files have no stale tail. Compared with
`hyt`'s saved `NNN_Name.bin` of the same day, most files are byte-identical to a file `hyt` or CPS read or wrote;
Comm_Basic 18, 242 and 245 are the read with CRC and bytes 16–19 recomputed (`prepare_for_write` of the read);
QEU_C_Config 5 (1 byte) and QEU_C_RxGrpList 80 (bytes 260–267, the leftover member slot of section 4a) differ
slightly from every capture. **Comm_RadioInfo (253) differs**: the radio holds it empty (zeros after the block
header), but CPS fills it in templates with the radio's identity. Decoded: `isReadTemplate` = 1, Serial Number,
`Radio ID` = `1`, Model Name, Radio Data Version (RCDB), Bootloader L2 Version `V1.6.02.001` (= info items
`0x0045`/`0x0065`, inferred from the value), Bootloader L3 `V1.6.03.001` (= `0x0003`), Frequency Range
`400-470MHz`, Last Programmed At (= Comm_Basic's Programming Time), Firmware Version, Voice Broadcast Version
`V1.00.05.000` (= `0xFFF4`), Tuning Data Version (= `0x0006`, hardware version), Model Number, `N/A` for the
others, licence flags Off.

So a template carries the radio's identity twice: in `tpHeader.data` (product, model number, RCDB version,
serial) and in file 253.

**How `hyt` uses it:**

- `hyt tpl import <t.tpl> --like <read> --out <folder>` takes every radio file from the template (including
  CPS's filled 253, which `hyt write` never writes) and the radio identity (manifest: profile, model, serial,
  firmware, RCDB version, info items) from `<read>`, so `hyt write` accepts the folder for that radio. It refuses
  a template whose `RcdbVersion` (header or any `psHeader`) or `Product` differs from the read; a different
  model number or serial number is only a warning. Files the template lacks are kept from the read and listed.
  The template's headers and a CRC of each imported file go into `template.json` in the folder.
- `hyt tpl export <folder> --out <t.tpl>` writes every file through `prepare_for_write`, in CPS's entry order,
  two-level files carrying their companion, with headers from the folder's identity (new `Guid`, `CreatedTime`
  now, `Frequence` from the model name). With `template.json` from an import, unchanged files keep their
  original `psHeader` and the template its name and description. `Seqo` comes from `template.json`, else from a
  CPS template of the same RCDB version (`--seqo-from`, default: CPS's `Templates\` folder); `hyt` keeps no copy
  of CPS's file descriptions. File 253 is exported as the folder has it (empty for a folder read from the radio:
  `hyt` does not fill it in like CPS; inferred harmless, unverified).
- Round trip (`crates/hyt-core/tests/tpl.rs`, all four templates): `export(import(t))` has the same entries in
  the same order, the same inner entries, byte-identical radio files and `psHeader.data`, and a `tpHeader.data`
  that differs only in `Guid` and `CreatedTime`. Header XML is rewritten byte for byte. At ZIP level only the
  timestamps, "version made by" (zip crate 2.0, DotNetZip 4.5) and the deflate output differ (exports are about
  1.9 KB larger).
- **Not verified:** that CPS opens an exported `.tpl`, and whether it needs `Seqo`, a filled 253 or an entry in
  `TemplatesXML.xml` (CPS's import presumably adds that itself). See open question 13.
