<#
.SYNOPSIS
  Exports parameter names, units and option values from Hytera CPS's parameter database
  (XMLToolCompactData.sdf) to a portable JSON file that `hyt` uses to label values.

.DESCRIPTION
  Needs the SQL Server Compact 4.0 runtime (installed alongside CPS). Works on a temporary copy of
  the database; CPS's own file is only read. Only English entries (language_type 1) are exported.
  The output contains Hytera data: keep it out of the source tree (default: your user data folder).

  Output: options.json { rcdb_version, source, params: [ { id, name, unit, options: [ { variant, index, range } ] } ] },
  ui.json (CPS's screen tree) and models.json (model-number rules: conditions, ranges, defaults,
  visible, enable, ui_conditions, ui_links; rows as arrays) and visibility.json (per parameter: CPS's
  `visible` value and `delete_flag`, rows [id, visible, delete_flag]), all in the same folder.
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

    # How CPS groups parameters on its screens (categories > pages > groups > parameters), exported
    # as flat link tables for hyt-gui's settings view. Parameters are given by their layout id
    # (Param.id); UiGroupParamLink refers to Param.param_id.
    # Each row becomes an array; $kinds gives each column's type ("i" int, "s" string).
    function Get-Table($sql, $kinds) {
        $cmd.CommandText = $sql
        $rr = $cmd.ExecuteReader()
        $rows = New-Object System.Collections.Generic.List[object]
        while ($rr.Read()) {
            $row = New-Object object[] $kinds.Length
            for ($i = 0; $i -lt $kinds.Length; $i++) {
                if ($kinds[$i] -eq "i") { $row[$i] = [int]$rr.GetValue($i) } else { $row[$i] = [string]$rr.GetValue($i) }
            }
            $rows.Add($row)
        }
        $rr.Close()
        return , $rows
    }
    $ui = [ordered]@{ rcdb_version = $version }
    $ui.roots = Get-Table "SELECT CategoryId, [Index] FROM UiModelCategoryLink WHERE ModelOnlyId = 1" @("i", "i")
    $ui.categories = Get-Table "SELECT Id, NameEn FROM UiCategory" @("i", "s")
    $ui.category_children = Get-Table "SELECT ParentId, ChildId, [Index] FROM UiCategoriesLink" @("i", "i", "i")
    $ui.category_pages = Get-Table "SELECT CategoryId, PageId, [Index] FROM UiCategoryPageLink" @("i", "i", "i")
    $ui.pages = Get-Table "SELECT Id, NameEn, Type FROM UiPage" @("i", "s", "i")
    $ui.page_groups = Get-Table "SELECT PageId, GroupId, [Index] FROM UiPageGroupLink" @("i", "i", "i")
    $ui.groups = Get-Table "SELECT Id, NameEn FROM UiGroup" @("i", "s")
    $ui.group_children = Get-Table "SELECT ParentGroupId, ChildGroupId, [Index] FROM UiGroupsLink" @("i", "i", "i")
    $ui.group_params = Get-Table "SELECT l.GroupId, p.id, l.[Index] FROM UiGroupParamLink l JOIN Param p ON p.param_id = l.ParamId" @("i", "i", "i")

    # Model-number rules: CPS narrows ranges, defaults and visibility by the radio's model number
    # (info item 0x0000, e.g. "55P10B00-1L0BAPU1U1..."). A rule group matches when every condition
    # holds: the model number without '-', characters start..start+length-1 (1-based), is one of
    # the comma-separated values. Parameters are given by their layout id (Param.id); the ModelNumber*
    # tables refer to Param.param_id. See docs/file-format.md section 2b.
    $models = [ordered]@{ rcdb_version = $version; source = "XMLToolCompactData.sdf ($CpsDir)" }
    $models.conditions = Get-Table "SELECT m.model_number_group_id, m.start_index, m.length, m.value, p.id FROM ModelNumberMainGroup m JOIN Param p ON p.param_id = m.param_id ORDER BY m.model_number_group_id, m.model_number_main_group_id" @("i", "i", "i", "s", "i")
    $models.ranges = Get-Table "SELECT r.default_group_id, p.id, r.[Index], r.range_value FROM ModelNumberRangeGroupLink r JOIN Param p ON p.param_id = r.param_id WHERE r.language_type = 1 ORDER BY r.default_group_id, p.id, r.[Index]" @("i", "i", "i", "s")
    $models.defaults = Get-Table "SELECT d.model_number_group_id, p.id, d.default_en FROM ModelNumberDefaultGroupLink d JOIN Param p ON p.param_id = d.param_id ORDER BY d.model_number_group_id, p.id" @("i", "i", "s")
    $models.visible = Get-Table "SELECT v.default_group_id, p.id, v.is_visible FROM ModelNumberVisibleGroupLink v JOIN Param p ON p.param_id = v.param_id ORDER BY v.default_group_id, p.id" @("i", "i", "i")
    $models.enable = Get-Table "SELECT e.model_number_group_id, p.id, e.is_enable FROM ModelNumberEnableGroupLink e JOIN Param p ON p.param_id = e.param_id ORDER BY e.model_number_group_id, p.id" @("i", "i", "i")
    # Screen parts shown/hidden by model number (meaning of link types not yet confirmed).
    $models.ui_conditions = Get-Table "SELECT model_number_group_id, start_index, length, value, releted_id, related_linkType FROM ModelNumberUIMainGroup ORDER BY model_number_group_id, id" @("i", "i", "i", "s", "i", "i")
    $models.ui_links = Get-Table "SELECT model_number_group_id, link_type, relatedUI_id, related_property_type, property_value FROM ModelNumberUILink ORDER BY model_number_group_id, link_id" @("i", "i", "i", "i", "s")

    # Which parameters CPS shows: Param.visible (CPS's ParamVisible enum: 0 shown, 1 never, 2 internal build
    # only, 3-6 RVM variants; the external build shows 0, 3, 4) and Param.delete_flag (shown only when 0).
    # See docs/research-cps-visibility.md. One row per layout id, the delete_flag = 0 row first (as above).
    $visibility = [ordered]@{ rcdb_version = $version; source = "XMLToolCompactData.sdf ($CpsDir)" }
    $seen = @{}
    $rows = New-Object System.Collections.Generic.List[object]
    foreach ($row in (Get-Table "SELECT id, visible, delete_flag FROM Param ORDER BY id, delete_flag, param_id" @("i", "i", "i"))) {
        if ($seen.ContainsKey($row[0])) { continue }
        $seen[$row[0]] = $true
        $rows.Add($row)
    }
    $visibility.params = $rows
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

$uiFile = Join-Path (Split-Path $OutFile) "ui.json"
$uiJson = $ui | ConvertTo-Json -Depth 4 -Compress
[IO.File]::WriteAllText($uiFile, $uiJson, (New-Object System.Text.UTF8Encoding($false)))
Write-Output "Exported CPS's screen structure ($($ui.pages.Count) pages, $($ui.group_params.Count) parameter links) to $uiFile"

$modelsFile = Join-Path (Split-Path $OutFile) "models.json"
$modelsJson = $models | ConvertTo-Json -Depth 4 -Compress
[IO.File]::WriteAllText($modelsFile, $modelsJson, (New-Object System.Text.UTF8Encoding($false)))
Write-Output "Exported model-number rules ($($models.conditions.Count) conditions, $($models.ranges.Count) ranges, $($models.defaults.Count) defaults, $($models.visible.Count) visibility rows) to $modelsFile"

$visibilityFile = Join-Path (Split-Path $OutFile) "visibility.json"
$visibilityJson = $visibility | ConvertTo-Json -Depth 4 -Compress
[IO.File]::WriteAllText($visibilityFile, $visibilityJson, (New-Object System.Text.UTF8Encoding($false)))
Write-Output "Exported which parameters CPS shows ($($visibility.params.Count) parameters) to $visibilityFile"
