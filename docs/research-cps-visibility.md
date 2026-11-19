# How CPS decides which settings to hide (research, 2026-11-19)

Read-only research for the BP515 U1 (RCDB V1.5.24.000, CPS V1.5.24.000.0003, model number
`55P10B00-1L0BAPU1U100000000CB000000000000-00000NN0000000000000000000000000-00040000`). Nothing in the CPS install
was changed, CPS was not started and the radio was not contacted.

Tags as in the other docs: **observed** = seen in a file, table or CPS's IL code (cited); **inferred** = conclusion
from observations; **unknown** = not found.

## Summary

CPS hides a setting in four layers. The first is new to `hyt`. It explains all 7 channel fields and most of
what `hyt` currently takes from `CPSParams.xlsx` or infers from screenshots:

1. **Parameter database flags** (`XMLToolCompactData.sdf`, table `Param`, columns `visible` and `delete_flag`).
   In CPS as installed (external build), a parameter is shown only if `visible` is 0, 3 or 4 **and**
   `delete_flag` = 0. *Observed* in code (below). The `xlsx` visibility column is a separate document
   and does not always match this.
2. **Model-number visibility rules** (`ModelNumberVisibleGroupLink.is_visible`, already exported to `models.json`
   as `visible`). For a matching model number they **replace** `Param.visible` before rule 1 runs.
   *Observed*. They are why Feature Control shows exactly Two Tone, Pure Analog, Pure Digital and Roam.
3. **Licence rule** `ParamTemplate.config` LinkType 20125 = class `AuthorizationRelationRule`. When the radio
   is opened it hides channel fields, menu items, key-function options and whole pages or categories for
   licence flags that are Off. Only Roam, Basic Encryption and Two Tone hide anything when Off. *Observed*.
4. The SDF link rules (`IsVisible` results) and the model-number UI rules, which `hyt` already has.

Answers:
- (a) The 7 digital-channel fields are hidden by layer 1. Special Frame, Quick GPS, Enhanced Channel Access and
  Location Info Revert Channel have `delete_flag` = 1. In Call TX Admit has `visible` = 2 (DevelopmentVisible,
  internal build only). Emergency System and RSSI Threshold (id **6499**) have `visible` = 1 (UnVisible).
  Confidence: **high**. Layers 1 and 3 together match all 42 fields of CPS's digital-channel dialog in
  `digital-channel-1.png`: every shown field passes and every hidden field fails one of them.
- (b) Table below. `hyt`'s `LICENCES` is partly wrong. Priority Interrupt Encode/Decode, Reliable PI Transmit and
  Auto Start Roam are hidden by layer 1, not by a licence. Several items are missing: the Two Tone effects,
  Beacon Interval, menu items, key options and whole categories.
- (c) LinkType numbers select an implementing class in `plug\ComplexRule\G2CPS.ComplexRule.G2LinkRule.dll`
  (method `GetLinkType()`). Among the conventional DMR rules, only 20125 writes `ParamBase.Visiable`.
  2010 and 2078 only enable or disable fields and force values.

## How this was found (tools)

- `plug\ComplexRule\G2CPS.ComplexRule.G2LinkRule.dll`, `G2CPS.Data.dll`, `G2CPS.ParamControls.dll`,
  `G2CPS.BaseMain.dll`, `DBService.dll` and `G2CPS.ComplexRule.dll`'s `AuthorizationRelationEventArgs` are **not
  obfuscated** (readable type, method and string names). *Observed*. `cps-internals.md` did not list them.
- No .NET decompiler is installed. A small IL dumper was used instead. It runs in Windows PowerShell 5.1, loads
  the DLL with `Assembly.ReflectionOnlyLoadFrom` (no CPS code runs), reads `MethodBody.GetILAsByteArray()` and
  resolves tokens, and it also turns stack IL into pseudo-C#. The scripts are now in `tools/cps-analysis/` (originally in the session scratchpad
  (`C:\Users\Jaan\AppData\Local\Temp\claude\7472904c488daa3d\e524b0d9-8bc7-49a3-a9ff-74f87e248641\scratchpad\`:
  `ildump.ps1` raw IL, `pdc.ps1` + `Pdc.cs` pseudo-C#, `sdfq.ps1` SDF queries on a copy of the SDF). They are
  copied 2026-11-19; `lt.txt` is `linktypes.txt` there).
- REA's managed tools list types but cannot decompile C#, and member inspection of `G2LinkRule.dll` ran over its
  size limit.

## Layer 1: `Param.visible` and `delete_flag`

*Observed*:
- `DBService.Common.EnumsDefine+ParamVisible` (`DBService.dll`): IsVisible 0, UnVisible 1, DevelopmentVisible 2,
  RVM_Visible_Store 3, RVM_Visible_UnStore 4, RVM_UnVisible_Store 5, RVM_UnVisible_UnStore 6.
- `G2CPS.Data.BusinessData.ParamBase.InitVisualState()` (`G2CPS.Data.dll`, IL 0x00–0x58) sets
  `Visiable = (CurrentCpsVersion != 1) ? (Visible in {0,3,4} && !IsDeleted) : (Visible not in {1,5,6} && !IsDeleted)`,
  then `Enable = true`. `ParamBase.ModelNumberChanged(DBParam, bool)` (IL 0x39–0x90) does the same with
  `dbparam.Visible` and `dbparam.DeleteFlag == 0`. `IsDeleted = DeleteFlag != 0` (`AnsyDBParamPropoty`).
- The same test filters CPS's search: `G2CPS.ParamControls.Extentions.NodeEntitiesFilterExtend.<FilterGroupParamByVisible>b__0`.
  Table views list only parameters with `Visiable` set (`TableParamExtend.GetUsableParams`).
- `CurrentCpsVersion` comes from `G2CPS.Main.exe.Config` `appSettings/CurrentCpsVersion` = **"2"**
  (`BaseApp.Current_Startup`, IL 0x17–0x38: `"1"` → 1, else 2). So this install is build 2, the external build. In
  build 1 DevelopmentVisible (2) parameters are also shown.
- The SDF's current database is RCDB **V1.5.24.000** (`DBVersion.is_current = 1`), the BP515's version. `Param` has
  one row per layout id, except id 6130 which has two rows.

The 7 fields (`Param` rows, *observed*):

| Layout id | Field | `visible` | `delete_flag` | Hidden because |
|---|---|---|---|---|
| 4520 | Special Frame | 0 | **1** | deleted flag |
| 4019 | Quick GPS | 0 | **1** | deleted flag |
| 3625 | Enhanced Channel Access | 0 | **1** | deleted flag |
| 38 | In Call TX Admit | **2** | 0 | DevelopmentVisible (internal build only) |
| 45 | Emergency System | **1** | 0 | UnVisible |
| 3413 | Location Info Revert Channel | 0 | **1** | deleted flag |
| 6499 | RSSI Threshold (file 3 block 1) | **1** | 0 | UnVisible |

None of them has a model-number visibility row. Check against the whole dialog (*observed*, `digital-channel-1.png`
vs `Param`): all 22 fields shown have `visible` = 0 and `delete_flag` = 0. The hidden fields are 2, 6, 3657, 31,
6051 (`visible` 1), 4389, 3934, 3935, 4383, 6189 (`delete_flag` 1) and the 7 above. Two more, 6186 IP Multi Site
Connect and the encryption fields 32 and 37, pass layer 1 and are hidden by layer 3 (Roam and Basic Encryption
Off).

**Comparison with `CPSParams.xlsx`** for every field on a CPS screen in the BP515's 58 files (*observed*, scratch
script `cmp.py`, with the BP515's model-number overrides applied):
- 122 fields that the xlsx marks "both builds" fail layer 1. Examples by file: 3 (the 7 above, PI Encode/Decode,
  Reliable PI Transmit, Auto Start Roam); 5 (Rx Group Including Tx Contact, Dial Rules, Remote Monitor/Radio
  Disable/Radio Check encode and decode, RRS/GPS ports, Alias Display Priority, Send Talker Alias, Encrypt
  Length, Data Bearer Service and Display Caller ID And Alias set hidden by a model rule…); 9 (Compandor, VOX);
  19 (SK2/P1/P2 button lock, registering/incoming call tones); 39 (Roaming stay, Roam RSSI Offset, Roam Interval
  Time); 46 (SK1 Short/Long, SK2 Short/Long, P1–P3 keys, equalizer, handsfree gains); 85 (Priority Sweep Time,
  Channel Marking); 87 (TTS, notify, VOX MIC); 95 and 96 (most emergency settings except type and mode); 104
  (CTCSS deviations); 250 (2-Tone personality fields).
- 16 fields that the xlsx marks internal-only or neither pass layer 1: 738 Roam On, 3405–3407 RRS registration
  timers, 102/103 Auto Reset Mode/Time, 1161 Volume Knob Lock, 1169 Keypad Auto Lock Delay, 6251/6261 Scan Priority
  Channel Tone(+Volume), 3676 Three Band Equalize Mode, 1728–1730 Private/Group/All Call TX Admit, 4125/4128 One
  Touch Call Contact Type/ID. Some of them are hidden by layer 3 (738 is in the Roam category; 102/103 by Two Tone
  Off).
- 111 fields that are not in the xlsx: 62 fail layer 1 (e.g. the 902 licence flags PI, RM, RDE and BE, file 18 menu
  items 6479–6788, file 96 6714–6752) and 49 pass (e.g. Radio Information 6618–6635, tone menu items).

*Inferred*: for this RCDB version, `Param.visible` + `delete_flag` + model rules are CPS's actual source. The
xlsx column is a help document and should only be a fallback, e.g. for RCDB versions the SDF does not describe.
*Unknown*: whether CPS's keys page (SK1/SK2/TK) uses `Visiable` at all. Keys may be drawn by a custom control, and
SK1 Short/Long and SK2 Short/Long have `delete_flag` 1. See "Owner checks".

## Layer 2: model-number visibility (Feature Control)

*Observed*:
- `DBService.dll` `DBDataService.ApplyParamChanges(modelNumber, …)` IL 0x0302: `tempParam.Visible =
  ModelNumberVisibleGroupLink.is_visible` for the groups whose conditions match. The value uses the
  `ParamVisible` enum (0 = visible). `models.json` `visible` rows are these.
- All licence flags in `Comm_Basic` 902 (6645–6652) and their read-only copies in `Comm_RadioInfo` 920
  (6636–6643) have `Param.visible` = 1. For the BP515 the matching groups (same 1-based, dash-free matching as
  `hytera-codeplug/src/models.rs`) set **0 for 6645 Pure Digital (group 965), 6646 Pure Analog (966), 6648 Roam
  (969), 6651 Two Tone (970)**, 6636/6637/6642 (961, 962, 964) and 6703 HDC1200 (1061), and **1 for 6639** (963).
  6647 Priority Interrupt, 6649 Remote Monitor, 6650 Radio Disable Enable and 6652 Basic Encryption stay UnVisible.
- Feature Control (`ui.json` page 322) has two groups. "Custom Authorization" = 6645–6652, which shows exactly the
  4 visible flags (matches `feature-control.png`). "Feature in Subscriber" = 6636–6643. Rule 20125 at load
  (`AuthorizationRelationRule.AppliedPassivityBlockRule`, IL 0x029F–0x0390): if `isReadTemplate` (6616) = 1, the
  Radio Information category is shown and each 6636+i is made visible when 6645+i is visible (otherwise left as
  is). If `isReadTemplate` ≠ 1, the Radio Information category and 6636–6643 are hidden. So the read-only group
  also shows Two Tone, Pure Analog, Pure Digital and Roam (6639 becomes visible through 6648).
- HDC1200 (6703) is on no CPS screen (`ui.json`) and rule 20125 does not use it. Nothing hides or shows it.

## Layer 3: the licence rule (LinkType 20125, `AuthorizationRelationRule`)

*Observed* in `G2CPS.ComplexRule.AuthorizationRelationRule` (`G2LinkRule.dll`):
- `GetLinkType()` returns 20125. Its active parameters are the 8 flags 6645–6652 (`Index` 0–7) and `isReadTemplate`
  6616 (`Index` 8) (`<AppliedPassivityParamRule>b__2…` select by `InitiativeParam.Index`).
- `AppliedPassivityBlockRule` runs **when a configuration is opened** (no event args; it sends
  `AuthorizationRelationEventArgs.IsInit = true`). For each flag it calls `RoamClose` if Roam = "0",
  `EncryptClose` if Basic Encryption = "0", `TwoToneOpen`/`TwoToneClose` for Two Tone = "1"/"0",
  `PureDigitalOpen` if Pure Digital = "1", else `PureAnalogOpen` if Pure Analog = "1". `AppliedPassivityParamRule`
  does the same when a flag is changed on the Feature Control page.
- The methods set `ParamBase.Visiable`, `Enable`, `Value` and option lists (`SetRange`). They then raise
  `AuthorizationRelationEventArgs` with two maps, page name → int and category name → int. The names come
  from `LanguagePackage\en-US.xaml`.
  `G2CPS.ParamControls…SetUiVisibility` stores `value == 1 ? 1 : 0` per page or category and hides a category
  when none of its pages is visible. So **1 = hidden, 0 = shown** (*inferred* from the code; matches the
  screenshots: no Encrypt page).
- They also run on the in-memory **digital/analog channel template** (`Table.DataTemplate`) and every channel row.

**Licence flag Off ("0") → what CPS hides** (`Comm_Basic` block 902; the BP515 has all of them Off). These run
at load. The pure modes only act when On.

| Flag (id) | Hidden fields (layout id, file) | Forced values / options | Pages, categories hidden |
|---|---|---|---|
| Roam (6648) | 6186 IP Multi Site Connect (3, every channel + template, also disabled); 850 Beacon Interval (5, block 579); 6456 menu item "Roam" (18, block 904) | 6186 = 0, 6456 = 0; key-function option 33 removed from the 12 key params of block 50 (value → 0 if it was 33); channel labels 29/20/19 renamed "Use Scan List"/"Auto Start Scan"/"Scan List" (matches `digital-channel-1.png`) | category **Roam** (Conventional › Roam: page Roam = files 38, 39 incl. 4370–4372, 823–834; page Setting = 738 Roam On) |
| Basic Encryption (6652) | 32 Encrypt, 37 Encrypt Key (3, every channel + template); 6464 menu item "Encrypt" (18, 904) | 32 = 0, 37 = 0, 6464 = 0; key option 7 relabelled "Scramble" (removed instead if Pure Digital = 1; a key set to 7 → 0) | page **Encrypt** (Digital Setting › Encrypt, page 117: 1688, 6178–6180, 6185, 6132–6134) |
| Two Tone (6651) | 6500 Signaling Side Tone, 6501 its volume (19, block 633); 6665 menu Signaling Side Tone (18, 904); analog channels (9, every row + template): 6343 Signaling Type, 6348 Personality List, 6345 RX Signaling System, 6346 TX Signaling System, 6491 CTCSS or CDCSS Required For Data, 102 Auto Reset Mode, 103 Auto Reset Time (also disabled) | 6500 = 0, 6501 = 0, 6343 = 0, 6346 = 0, 6491 = 0, 102 = "00", 103 = 10; 92 RX Squelch Mode options 2, 3, 4 removed (value → 0 unless 0 or 1); 6446 One Touch Call "Call Mode" (200, block 807) option 2 Analog removed (value → 0). Only if Pure Analog = 1: also 6248/6258 Group Call Tone(+Volume) hidden and 6248 = 0, menu 6615 Contact and 6659 Group Call Tone hidden, key options 39–43 (One Touch Call/Message 1–5) removed | category **Analog Service** (HDC1200 System/Contact, 2-Tone System/Contact/Personality = files 246–250); page One Touch Call too if Pure Analog = 1 |
| Priority Interrupt (6647) | nothing (the PI channel fields 3934, 3935, 4383 are hidden by layer 1) | none | none |
| Remote Monitor (6649) | nothing (3429, 3430, 3916, 4018 are hidden by layer 1, `visible` = 1) | none | none |
| Radio Disable Enable (6650) | nothing (2808, 3804, 3805 are hidden by layer 1) | none | none |
| Pure Digital (6645), Pure Analog (6646) | nothing when Off | — | — |
| HDC1200 (6703) | not handled by any rule found | — | — |

When a flag is **On** (*observed*, summary only): `RoamOpen`, `EncryptOpen` and `TwoToneOpen` undo the above. Roam
On also adds option 2 "Roam" to 29 when 6186 = 1. Pure Digital On hides the pages Analog Setting, Analog Channel
and Analog Emergency, sets Pure Analog = 0, disables the Two Tone flag (sets it 0) and changes tone, menu
(904/903), key, One Touch Call and zone settings. Pure Analog On hides Voice Service, Network, Digital Channel,
Digital Emergency, Radio Alias and the categories Digital Setting and Contact; it sets PI, Roam, RM, RDE and BE
to 0 and disables them, sets Pure Digital = 0, and hides digital tone and menu settings (6244–6259, 6169–6172,
6460/6461, 6655–6660, scan 1187/1191). Field lists of the pure modes are not fully mapped. Other model checks in
the rule: model number starting with '3' hides pages One Touch Call and Network; characters 39–40 (with
dashes) = "31" removes key option 7. Neither applies to the BP515.

*Inferred*: the forced values mean that **CPS rewrites these fields when it opens a configuration** (e.g. a
channel's Encrypt and Encrypt Key are set to 0 while Basic Encryption is Off). The BP515's unchanged CPS write
matched the read, so these fields were already in that state. *Unknown*: the 12 key params selected in block
50 (`keys` arrays come from `RuntimeHelpers.InitializeArray`, not decoded); *inferred*: they are the 12 key
press settings of `Comm_Hardware` 46 (SK1/SK2/SK3/TK/P1–P3 short/long). Option 33 is not in V1.5.24.000's key
option catalogue, so removing it has no effect on the BP515.

**Proposed replacement for `LICENCES` in `visibility.rs`** (fields only; pages and categories need a separate
list in the settings view):

```
Basic Encryption: 32, 37, 6464      + page "Encrypt" (1688, 6178, 6179, 6180, 6185, 6132, 6133, 6134)
Roam:             6186, 850, 6456   + category "Roam" (files 38, 39 and 738)
Two Tone:         6500, 6501, 6665, 6343, 6348, 6345, 6346, 6491, 102, 103
                                    + category "Analog Service" (files 246-250)
                  (if Pure Analog On: + 6248, 6258, 6615, 6659, page "One Touch Call")
Priority Interrupt, Remote Monitor, Radio Disable Enable, HDC1200: none
```

Remove from the licence lists, because layer 1 already hides them: 3931, 3932, 6544, 6545 (`visible` 1 +
deleted), 1711, 1726, 6131, 6189, 3934, 3935, 4383 (deleted), 6135 (`visible` 1). 2807 Encrypt Service and the
XPT 3333–3338 are on XPT pages (`ui.json` page 250 category 61, page 132), not on the conventional tree. 6533
and 6540 (template channel Encrypt and Key, file 252) are covered because CPS applies the rule to the channel
template.

## Where the licence comes from (licence install)

Research 2026-11-19, read-only (IL of `plug\G2CPS.ParamTemplateBiz.dll`, readable; `G2CPS.Protocol.dll` enum;
`G2CPS.Service.dll`, obfuscated). Question: what does the owner need for CPS to show encryption on the BP515?

*Observed*:
- **Basic Encryption (6652) can't be switched on in CPS's editor.** `Param.visible` = 1 and no row in
  `ModelNumberVisibleGroupLink`, `…EnableGroupLink` or `…DefaultGroupLink` names it (those tables key on
  `Param.param_id`, 7135 for 6652, not on the layout id). Only Pure Digital, Pure Analog, Roam and Two Tone have
  model-number rows, hence the 4 switches on Feature Control.
- **The radio reports its licences in identify item `0x0200`** (`BaseInforItem.LicenseFeature = 512`). The BP515
  answers 64 zero bytes: **no licence is installed**. Its file 18 block 902 and file 253 block 920 also have
  every licence flag 0.
- **Licence codes** (`DeviceItemVM.syncLicenseFeature`): 1500 Pure Digital, 1501 Pure Analog, 1502 Priority
  Interrupt, 1503 Roam, 1504 Remote Monitor, 1505 Radio Disable Enable, 1506 HDC1200, 1507 Two Tone, 1508 Five
  Tone, **1509 Basic Encryption**.
- **Licences are installed with CPS's "Authorization" button** (`DeviceItemVM.Authorization`, `AuthorizationCommand`
  on a radio in the ParamTemplate plugin's device list). Steps, from `<Authorization>b__e`:
  1. Open dialog "Import license(*.zip)". The zip must hold `*.xml` (readme), `*.md5` and one or more `*.fcr`.
  2. readme XML: `license/filelist/package` elements with `objdevice` (radio serial), `file` (`.fcr` name), `md5`,
     and `detail` children with `code` and `value` (value ≠ "0" = licensed). Only the package whose `objdevice`
     equals the connected radio's serial is used ("DeviceFileNotExists" otherwise).
  3. Checks: MD5 of the readme XML = contents of the `.md5` file; MD5 of the `.fcr` = the package's `md5`; at least
     one licensed code ("NoAvailableLicense"); codes allowed for the model ("ExistsMismatchedLicense"); Pure
     Digital and Pure Analog not with Two Tone or Roam respectively ("ErrorLicense").
  4. `CpsServerice.WriteLicenseFile(device, fcrPath)` sends the `.fcr` to the radio as a "secure file"
     (`WriteSecureFile(channel, data, len, regionId)`, region 512, data padded to a multiple of 64; size checks
     against 285 and 162 bytes seen in the flattened code). Then "Authorization succeeded. Please restart the radio
     and read/write the frequency".
  5. The `.fcr` content is made by Hytera for that serial. *Inferred*: the radio checks it, which is why only Hytera can
     issue licences. Not analysed further on purpose.
  6. How the `.fcr` is sent (`G2CPS.Service.dll` `WriteLicenseFile` / `WriteSecureFile`, control flow flattened, calls
     through obfuscated proxies, so partly *inferred*): read the file's bytes (`ReadLicenseFile`, plain read); a
     size check against 285; a 162-byte value; pad to a multiple of 64; append 2 bytes (*inferred* a checksum);
     combine with 10 bytes obtained from the radio first (*inferred* `RequestRandomValue` `E100/0001`, so each push
     differs and a recording can't simply be replayed); then `WriteSecureFileHeadCommand(512, length, 1400)` and
     `WriteSecureFilePackCommand` chunks of up to 1400 bytes ([protocol.md](protocol.md)). The exact bytes need a
     recording of a real push.
- **CPS V1.5.24.000.0003 refuses Basic Encryption for every model** (raw IL of `<Authorization>b__e`, 0x0458–0x0807,
  applied to all 50 models of `ModelNumber.csv`). Branches: (A) model number character 15 = `0` (all AP models
  except "AP580 U1 数字"): only Two Tone; AP32x nothing. (K) BP568K, BP578K, BP578 DIG or model number characters
  39–41 = `051`: only Roam; BP578K and BP578 DIG nothing. (N) all others, incl. BP36x/51x/56x: Pure Digital, Pure
  Analog, Roam, Two Tone; BP36x without Roam and Two Tone; BP515 LF without Roam. 1509 is in the refused list of
  every branch.
- **CPS V1.5.24.000.0003 refuses Basic Encryption for the BP515 U1.** For a model number whose 15th character is
  not `0` (BP515 U1: `P`) and a model name other than BP568K/BP578K/BP578 DIG, codes 1502, 1504, 1505, 1506, 1508
  and **1509** count as "mismatched" and the import stops before anything is sent (`<Authorization>b__e` IL
  0x070A–0x0755). "bp515 lf" also refuses 1503 (Roam); "bp36…" also refuses 1503 and 1507. The device list's
  checkbox list (`MatchAuthorizationItems`) only ever contains 1500, 1501, 1503 and 1507.
- When the ParamTemplate plugin reads a radio (`ReadFrequencyViewModel.UpdateRadioInformation`), it copies the
  device's licence list into blocks 902 and 920 (`SetLicenseFeature`: "1" if checked). For a feature not in that
  list (e.g. Basic Encryption) the stored value is left as read.

*Inferred*: on the BP515, CPS shows encryption once file 18 block 902 `Basic Encryption` = 1 (rule 20125,
`EncryptOpen`). That requires a Basic Encryption licence installed in the radio, and this CPS version doesn't accept
one for the BP515 U1. *Unknown*: whether the radio firmware sets block 902 itself after a licence install (*inferred*
likely, since file 18 comes from the radio); what a newer CPS/RCDB accepts; the exact `0x0200` format with licences
present; whether the radio honours channel encryption without the licence. Setting 902 by hand is not a licence
install and is not done by `hyt`.

`hyt licence check <zip> [--radio <read>]` (`hyt-core/src/licence.rs`, tests `tests/licence.rs`) repeats CPS's checks
offline (top-level files, readme MD5 text compare, serial, `.fcr` name + MD5, feature conflicts, V1.5.24 model
rules incl. the K-model branch). `hyt licence status <read>|--port` decodes item
`0x0200` and the 902/920 flags. Neither sends anything that writes to the radio.

## LinkType numbers in `ParamTemplate.config`

*Observed*:
- Each `<ComplexLink LinkType=N>` is applied by the rule class whose `GetLinkType()` returns N (classes derive
  from `ComlpexRuleBase` in `G2LinkRule.dll`). 297 LinkTypes map to classes there; some (−10, 6, 7, 9, 6000,
  7000, 9000, 10002, 18xxx, 2062, 2100…) are not in that DLL. *Inferred*: they are in `G2CPS.ComplexRule.PDC.dll`,
  `…RVM.dll` or the obfuscated `G2CPS.ComplexRule.dll`. The rule receives the `InitiativeParam`s (matched by `Index`) and
  either `PassivityParam`s (`AppliedPassivityParamRule`, on value change) or `PassivityBlock`s
  (`AppliedPassivityBlockRule`, including on load).
- Header comment of the file, translated: `Count` count link, `IsAppiled` apply or not,
  `InitiativeParamRelation` 0 And / 1 Or, `Mode` variant within a type, `TiggerWay` 0 = value change and complex-link
  initialisation, 1 = row OK/Cancel, 2 = move up/down/left/right (3 is used for virtual-parameter queries),
  `Condition` 0 any / 1 is / 2 is not. `TableParamFixedLinks`: `BlockIndex`, `BlockType`, `BlockAlias`, `LinkLevel`
  0 strong (delete follows) / 1 weak.
- Rules that write `ParamBase.Visiable`: only **20125** (`AuthorizationRelationRule`, 130 writes) among the
  conventional DMR ones. The others are 2504 `ScanListHP2HideDigitalRule` (ENS HP2), 4249 `FavouriteFolderRelationRule`,
  7005/7014 TETRA folder queries and 8006 `MptPrivateCallTypeChangedRule` (MPT). Many others write only `Enable` or
  values, e.g. 2010 `DigitalEncryptRule` (47 `Enable` writes), 2078 `XPTChannelEncyptRule`, 2092
  `PriorDigitalCHScanRule`, 2061 `ScanType_affectScanListPb`, 2002 `ChannelModeStatusRule`, 2031
  `AnalogChannelModeStatusRule`, 2063, 2085, 2089. The full LinkType → class table is in the scratchpad file
  `lt.txt` (format: LinkType, class, Visiable writes, Enable writes).
- Related: 20126 `RadioInformationRule` (Radio Information page), 20128 `CommonMenuRule` (menu block 904, Enable
  only), 2507 `TransferChannelRule` (IP Multi Site Connect ↔ option "Roam List" of 29), 2039/2050/2051 (Quick GPS
  vs RRS/revert channel choices).

## What `hyt` should do

**Done 2026-11-19** (points 1 and 2): `visibility.json` exported by `tools/export-rcdb-options.ps1`, read by
`hytera-codeplug/src/param_visibility.rs`; classification in `hyt-core/src/visibility.rs` (licence table from the
table above, files for the Roam and Analog Service categories, Feature in Subscriber mirroring); model-number
`is_visible` now read as `ParamVisible` (it was inverted in `models.rs`). The scripts are in `tools/cps-analysis/`.
Point 3 (forced values) is not done. Original suggestions:

1. Export `Param.visible` and `Param.delete_flag` (by layout id, row with `delete_flag` 0 first for id 6130) next
   to `options.json`, and classify a field as hidden when `!(visible in {0,3,4}) || delete_flag == 1`, with
   `visible` replaced by the matching model-number row. Use the xlsx column only when the SDF has no row.
2. Replace `LICENCES` with the table above and add page and category hiding (Encrypt page, Roam and Analog Service
   categories). Mirror Feature in Subscriber (6636–6643) from Custom Authorization (6645–6652) when
   `isReadTemplate` = 1.
3. Optionally apply CPS's forced values at load (Roam Off → 6186 = 0 etc.) only as a warning, not as a silent
   change, so `hyt`'s byte-identical round trip is not broken.

## Owner checks (CPS screens)

1. Programmable keys page (Common or Narrowband Setting › Keys or similar): are SK1 Short/Long and SK2 Short/Long
   shown? Layer 1 says they are deleted (1464, 6102–6104), but SK3 and TK are not. A screenshot settles whether
   key pages follow `Visiable`.
2. Conventional tree below "Emergency" (scroll `parameter-tree-conventional.png` down): there should be **no Roam
   and no Analog Service** categories. Also check whether Digital Setting › Location Service/Setting/Data Bearer
   Service/Quick Message are missing for another reason.
3. Analog channel dialog: no Signaling Type, Personality List, RX/TX Signaling System, CTCSS or CDCSS Required
   For Data, Auto Reset Mode/Time; RX Squelch Mode offers only Carrier and CTCSS/CDCSS (Two Tone Off).
4. Fields that layer 1 shows but the xlsx calls internal: Common › Voice Service (Private/Group/All Call TX
   Admit 1728–1730), Digital Setting › RRS (3405–3407), UI › Volume Knob Lock (1161), Keypad Auto Lock Delay
   (1169), Scan Priority Channel Tone(+Volume) (6251/6261), Three Band Equalize Mode (3676), One Touch Call Contact
   Type/ID (4125/4128).
5. Analog Emergency page: only Emergency Type (1623) should be shown; the other file 96 fields have `visible` = 1.
6. Key-function drop-down: option 7 should read "Scramble", not "Scramble/Encryption".
