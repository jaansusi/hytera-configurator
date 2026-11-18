<#
.SYNOPSIS
  Exports parameter names, units and option values from Hytera CPS's parameter database
  (XMLToolCompactData.sdf) to a portable JSON file that `hyt` uses to label values.

.DESCRIPTION
  Needs the SQL Server Compact 4.0 runtime (installed alongside CPS). Works on a temporary copy of
  the database; CPS's own file is only read. Only English entries (language_type 1) are exported.
  The output contains Hytera data: keep it out of the source tree (default: your user data folder).

  Output: { rcdb_version, source, params: [ { id, name, unit, options: [ { variant, index, range } ] } ] }
  `range` is CPS's text, e.g. "RAM:0,UI:High" or "RAM:[3,99],UI:[15,495],Step:1"; `variant` is CPS's
  `different_version` (0 = always; other values are model/firmware variants).

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\export-rcdb-options.ps1
#>
param(
    [string]$CpsDir = "C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003",
    [string]$OutFile = ""
)
$ErrorActionPreference = "Stop"

$sdf = Join-Path $CpsDir "XMLToolCompactData.sdf"
if (-not (Test-Path $sdf)) { throw "Not found: $sdf" }
$dll = Get-ChildItem "$env:ProgramFiles\Microsoft SQL Server Compact Edition\v4.0" -Recurse -Filter System.Data.SqlServerCe.dll -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $dll) { throw "SQL Server Compact 4.0 runtime not found (it is installed with CPS)." }
Add-Type -Path $dll.FullName

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("hyt-rcdb-" + [Guid]::NewGuid().ToString("N") + ".sdf")
Copy-Item $sdf $tmp
try {
    $conn = New-Object System.Data.SqlServerCe.SqlCeConnection("Data Source=$tmp")
    $conn.Open()
    $cmd = $conn.CreateCommand()

    $cmd.CommandText = "SELECT rcdb_version FROM DBVersion WHERE is_current = 1"
    $version = [string]$cmd.ExecuteScalar()
    if (-not $version) { throw "No current RCDB version in the database." }

    $params = @{}
    $order = New-Object System.Collections.Generic.List[int]
    # delete_flag = 1 does not mean "absent": 163 such parameters are in V1.5.24.000's layout.
    # For an id that appears twice, the row with delete_flag = 0 comes first and wins.
    $cmd.CommandText = "SELECT id, alias_en, param_unit FROM Param ORDER BY id, delete_flag, param_id"
    $r = $cmd.ExecuteReader()
    while ($r.Read()) {
        $id = [int]$r.GetValue(0)
        if ($params.ContainsKey($id)) { continue }
        $params[$id] = [ordered]@{
            id      = $id
            name    = [string]$r.GetValue(1)
            unit    = [string]$r.GetValue(2)
            options = New-Object System.Collections.Generic.List[object]
        }
        $order.Add($id)
    }
    $r.Close()

    $cmd.CommandText = "SELECT p.id, d.different_version, d.[Index], d.range_value FROM DefaultRangeValue d JOIN Param p ON p.param_id = d.param_id WHERE d.language_type = 1 ORDER BY p.id, d.different_version, d.[Index]"
    $r = $cmd.ExecuteReader()
    while ($r.Read()) {
        $id = [int]$r.GetValue(0)
        if (-not $params.ContainsKey($id)) { continue }
        $params[$id].options.Add([ordered]@{
            variant = [int]$r.GetValue(1)
            index   = [int]$r.GetValue(2)
            range   = [string]$r.GetValue(3)
        })
    }
    $r.Close()
    $conn.Close()
}
finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
}

if (-not $OutFile) {
    $OutFile = Join-Path $env:LOCALAPPDATA "hyt\rcdb\$version\options.json"
}
New-Item -ItemType Directory -Force (Split-Path $OutFile) | Out-Null
$doc = [ordered]@{
    rcdb_version = $version
    source       = "XMLToolCompactData.sdf ($CpsDir)"
    params       = @($order | ForEach-Object { $params[$_] })
}
$json = $doc | ConvertTo-Json -Depth 6 -Compress
[IO.File]::WriteAllText($OutFile, $json, (New-Object System.Text.UTF8Encoding($false)))
$withOptions = @($order | Where-Object { $params[$_].options.Count -gt 0 }).Count
Write-Output "Exported $($order.Count) parameters ($withOptions with options) for RCDB $version to $OutFile"
