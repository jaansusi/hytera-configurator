# What we know about Hytera CPS itself

Hytera CPS ("Customer Programming Software", G2 generation, built on the Auctus baseband platform) is the
reference implementation we are replacing. This page records how it is put together, so later work does not
repeat the analysis.

## Installation

| Item | Value |
|---|---|
| Launcher | `C:\Program Files (x86)\Hytera\CPSStarter\G2CPS.Starter.exe`, manifest **`requireAdministrator`** |
| Main program | `C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003\G2CPS.Main.exe`, manifest `asInvoker` |
| Version | `V1.5.24.000.0003` (`G2CPS.Main.exe.Config`); also sets `CurrentLanguage=en-US`, `IsNeedChenkSN=true` |
| Runtime | .NET Framework 4, 32-bit (x86) |

Because the launcher demands admin rights, `G2CPS.Main.exe` always runs **elevated** when started normally. Tools
that attach to it (Frida, debuggers) must also run elevated. Windows hides the path of the elevated process from
non-elevated tools.

## Assemblies

Protected with **.NET Reactor** (scrambled names, encrypted strings, flattened control flow, invalid metadata
rows, invalid call tokens). Static reading of these is slow and unreliable:

`G2CPS.Communication.dll` (serial link, packets), `G2CPS.Protocol.dll`, `G2CPS.Service.dll`,
`G2CPS.DataManager.dll`, `G2CPS.DBService.dll`, `G2CPS.DBService.Entity.dll`, `G2CPS.ComplexRule.dll`,
`G2CPS.ImportExport.dll`, `G2CPS.ImportExportMange.dll`, `G2CPS.Main.exe`, `G2CPS.Starter.exe`.

Readable names that survived in `G2CPS.Communication.dll`: `G2CPS.Communication.{MessageType, ReceiveEventArgs,
ApplicationChannel}`, `…BasicChannel.{ChannelBase, UartChannel, HostUartChannel, ReceivedDataHandle}`,
`Auctus.Host.Contact.{HostContact, HostPacket, HOST_RESULT}`, `Auctus.Host.{HostConsole, HostIO}`,
`Auctus.Host.HostSerialPort.{HostPort, HostRead, HostWrite}`. It talks to the cable through
`System.IO.Ports.SerialPort`; its only direct native imports are Reactor's `LoadLibrary`/`GetProcAddress`.

**Not protected** (normal .NET, readable): `G2CPS.Link.dll` (rules linking related settings in the editor, *not*
the radio link), `CommonBase.*`, `G2CPS.RcdbService.dll`, `G2CPS.FileData*.dll`, `G2CPS.GenFile.dll`,
`G2CPS.DataConvertService.dll`, `G2CPS.DataValidationService.dll`, `G2CPS.EncryptService.dll` (7 KB),
`Rcdb2CLR.dll`, `CLIWrapperHfcRcdb.dll`, `KeyEncryptCLR.dll`, `ImportTrunkAirKeyCLR.dll`. These are the place
to look for codec details (e.g. how list files or CRCs are built) if captures are not enough.
Also readable (found 2026-11-19, [research-cps-visibility.md](research-cps-visibility.md)):
`plug\ComplexRule\G2CPS.ComplexRule.G2LinkRule.dll` (the `ParamTemplate.config` rule classes, one per `LinkType`,
via `GetLinkType()`; table in `tools/cps-analysis/linktypes.txt`), `G2CPS.Data.dll` (`ParamBase`, e.g.
`InitVisualState`), `G2CPS.ParamControls.dll`, `G2CPS.BaseMain.dll`, `DBService.dll` (enums such as
`ParamVisible`, `DBDataService`), and `G2CPS.ComplexRule.dll`'s `AuthorizationRelationEventArgs`.

**Reading .NET code without a decompiler** (`tools/cps-analysis/`, read-only): `ildump.ps1` (raw IL) and `pdc.ps1`
+ `Pdc.cs` (pseudo-C#) load an assembly with `Assembly.ReflectionOnlyLoadFrom`, so no CPS code runs;
`sdfq.ps1` queries a temp copy of `XMLToolCompactData.sdf`; `cmp.py` compares the database's visibility with the
xlsx column.

No `.pdb` debug symbols ship except for `G2CPS.Main.exe` and `G2CPS.Starter.exe`, and those two are protected anyway.

## Data resources shipped with CPS

See [file-format.md](file-format.md): `RcdbJsons\`, `RcdbXmls\`, `RCDB\` (FileID, TableLink, DMR, Json),
`Templates\`, `Config\ModelNumberRuleTypeInfo.xml`, `CPSParams.xlsx`, `DifferentParams.csv`,
`DataCheckConfig.config`, `DataValidationConfig.config`, `ComplexRuleConfig.Config`.

`CPSParams.xlsx` (observed) is CPS's parameter help: header row in Chinese, then per parameter: category,
module, parameter ID (= layout field `Id`), English/Chinese name, visibility (internal/external build),
and descriptions, notes and option explanations in Chinese and English, for DMR and for TETRA. hyt
reads the English DMR columns at run time (`hytera-codeplug/src/help.rs`). Column G is a visibility class
(`内外部版本都可见` both builds, `仅内部版本可见` internal build only, `内外部版本都不可见` neither); it explains part of
what CPS hides, see [file-format.md](file-format.md) (encryption/licence notes). `RCDB\Json\Description.txt`
despite its name is only a layout file (no help).

## How the analysis was done (tool notes)

- **REA** (Claude Code plugin `rea@rea` 6.9.0) for managed metadata: `inspect_managed_artifact`,
  `inspect_managed_members`, `inspect_managed_native_boundaries`. REA paths on this machine are written as
  `/Program Files (x86)/...` (a leading `/` = drive C:); `/C:/...` fails.
- REA's bundled Frida lacks its native binding for Node 26, so its `*_frida_*` tools fail. Use the Python
  `frida` package instead (see [capture-howto.md](capture-howto.md)).
- Large REA results overflow the chat; request `detail: "summary"`, then page or filter the retained Evidence.
