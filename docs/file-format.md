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
