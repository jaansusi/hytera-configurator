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

No `.pdb` debug symbols ship except for `G2CPS.Main.exe` and `G2CPS.Starter.exe`, and those two are protected anyway.

## Data resources shipped with CPS

See [file-format.md](file-format.md): `RcdbJsons\`, `RcdbXmls\`, `RCDB\` (FileID, TableLink, DMR, Json),
`Templates\`, `Config\ModelNumberRuleTypeInfo.xml`, `CPSParams.xlsx`, `DifferentParams.csv`,
`DataCheckConfig.config`, `DataValidationConfig.config`, `ComplexRuleConfig.Config`.

## How the analysis was done (tool notes)

- **REA** (Claude Code plugin `rea@rea` 6.9.0) for managed metadata: `inspect_managed_artifact`,
  `inspect_managed_members`, `inspect_managed_native_boundaries`. REA paths on this machine are written as
  `/Program Files (x86)/...` (a leading `/` = drive C:); `/C:/...` fails.
- REA's bundled Frida lacks its native binding for Node 26, so its `*_frida_*` tools fail. Use the Python
  `frida` package instead (see [capture-howto.md](capture-howto.md)).
- Large REA results overflow the chat; request `detail: "summary"`, then page or filter the retained Evidence.
