<#
.SYNOPSIS
  Exports CPS's link rules (which settings are greyed out, hidden or forced when another setting
  has a given value) from Hytera CPS's parameter database (XMLToolCompactData.sdf) to a JSON file
  that `hyt` loads next to the option catalogue.

.DESCRIPTION
  Needs the SQL Server Compact 4.0 runtime (installed alongside CPS). Works on a temporary copy of
  the database; CPS's own file is only read. The output contains Hytera data: keep it out of the
  source tree (default: %LOCALAPPDATA%\hyt\rcdb\<version>\links.json).

  The tables are exported flat, one array per row, with parameters given by their layout id
  (Param.id, as in RcdbJsons) instead of the database's Param.param_id:
    rules        [link_main_id, link_id, enable_flag, link_relation_type, initiative_logic_type,
                  passivity_logic_type, link_title]
    initiative   [link_main_id, link_initiative_param_id, layout id, condition_logic_type, param_index]
    conditions   [link_initiative_param_id, link_type, condition, condition_self_logic, condition_index]
    passivity    [link_main_id, link_passivity_param_id, layout id, condition_logic_type, param_index]
    results      [link_passivity_param_id, link_result_type, result, result_self_logic, result_index]
    model_numbers[link_main_id, model_number, model_number_logic_type]
  A layout id of -1 means the database parameter has no Param row.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\export-rcdb-links.ps1
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

    # Each row becomes an array; $kinds gives each column's type ("i" int, "s" string).
    function Get-Table($sql, $kinds) {
        $cmd.CommandText = $sql
        $rr = $cmd.ExecuteReader()
        $rows = New-Object System.Collections.Generic.List[object]
        while ($rr.Read()) {
            $row = New-Object object[] $kinds.Length
            for ($i = 0; $i -lt $kinds.Length; $i++) {
                $v = $rr.GetValue($i)
                if ($kinds[$i] -eq "i") {
                    if ($v -is [DBNull]) { $row[$i] = -1 } else { $row[$i] = [int]$v }
                } else {
                    $row[$i] = [string]$v
                }
            }
            $rows.Add($row)
        }
        $rr.Close()
        return , $rows
    }

    $doc = [ordered]@{
        rcdb_version = $version
        source       = "XMLToolCompactData.sdf ($CpsDir)"
    }
    $doc.rules = Get-Table "SELECT link_main_id, link_id, enable_flag, link_relation_type, initiative_logic_type, passivity_logic_type, link_title FROM LinkMain ORDER BY link_main_id" @("i", "i", "i", "i", "i", "i", "s")
    $doc.initiative = Get-Table "SELECT l.link_main_id, l.link_initiative_param_id, p.id, l.condition_logic_type, l.param_index FROM LinkInitiativeParam l LEFT JOIN Param p ON p.param_id = l.param_id ORDER BY l.link_main_id, l.param_index" @("i", "i", "i", "i", "i")
    $doc.conditions = Get-Table "SELECT link_initiative_param_id, link_type, condition, condition_self_logic, condition_index FROM LinkInitiativeCondition ORDER BY link_initiative_param_id, condition_index" @("i", "i", "s", "i", "i")
    $doc.passivity = Get-Table "SELECT l.link_main_id, l.link_passivity_param_id, p.id, l.condition_logic_type, l.param_index FROM LinkPassivityParam l LEFT JOIN Param p ON p.param_id = l.param_id ORDER BY l.link_main_id, l.param_index" @("i", "i", "i", "i", "i")
    $doc.results = Get-Table "SELECT link_passivity_param_id, link_result_type, result, result_self_logic, result_index FROM LinkPassivityResult ORDER BY link_passivity_param_id, result_index" @("i", "i", "s", "i", "i")
    $doc.model_numbers = Get-Table "SELECT link_main_id, model_number, model_number_logic_type FROM LinkModelNumber" @("i", "s", "i")
    $conn.Close()
}
finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
}

if (-not $OutFile) {
    $OutFile = Join-Path $env:LOCALAPPDATA "hyt\rcdb\$version\links.json"
}
New-Item -ItemType Directory -Force (Split-Path $OutFile) | Out-Null
$json = $doc | ConvertTo-Json -Depth 4 -Compress
[IO.File]::WriteAllText($OutFile, $json, (New-Object System.Text.UTF8Encoding($false)))
Write-Output "Exported $($doc.rules.Count) link rules ($($doc.initiative.Count) controlling and $($doc.passivity.Count) dependent parameters) for RCDB $version to $OutFile"
