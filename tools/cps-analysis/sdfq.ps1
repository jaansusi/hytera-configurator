# Query CPS's parameter database (XMLToolCompactData.sdf) read-only, on a temp copy. Several queries: separate with ';;'.
# Usage: powershell -ExecutionPolicy Bypass -File tools\cps-analysis\sdfq.ps1 "SELECT id, visible, delete_flag FROM Param"
# Delete %TEMP%\hyt-sdfq-copy.sdf to re-copy after a CPS update.
param([string]$Sql)
$ErrorActionPreference = "Stop"
$sdf = "C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003\XMLToolCompactData.sdf"
$dll = Get-ChildItem "$env:ProgramFiles\Microsoft SQL Server Compact Edition\v4.0" -Recurse -Filter System.Data.SqlServerCe.dll -ErrorAction SilentlyContinue | Select-Object -First 1
Add-Type -Path $dll.FullName
# Work on a copy in the temp folder (never in the repo: it is Hytera data).
$tmp = Join-Path ([IO.Path]::GetTempPath()) "hyt-sdfq-copy.sdf"
if (-not (Test-Path $tmp)) { Copy-Item $sdf $tmp }
$conn = New-Object System.Data.SqlServerCe.SqlCeConnection("Data Source=$tmp")
$conn.Open()
$cmd = $conn.CreateCommand()
foreach ($q in $Sql -split ';;') {
  $cmd.CommandText = $q
  $r = $cmd.ExecuteReader()
  $names = @(); for ($i = 0; $i -lt $r.FieldCount; $i++) { $names += $r.GetName($i) }
  ($names -join "`t")
  while ($r.Read()) { $vals = @(); for ($i = 0; $i -lt $r.FieldCount; $i++) { $vals += [string]$r.GetValue($i) }; ($vals -join "`t") }
  $r.Close()
}
$conn.Close()
