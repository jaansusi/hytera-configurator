# Pseudo-C# from IL for a .NET assembly's types (reflection-only load: no CPS code runs; uses Pdc.cs).
# Usage: powershell -ExecutionPolicy Bypass -File tools\cps-analysis\pdc.ps1 <path to dll> <type regex> [method regex]
param($dll, $typePattern, $methodPattern = '.')
$dir = Split-Path $dll
$cps = 'C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003'
Add-Type -Path (Join-Path $PSScriptRoot 'Pdc.cs')
[AppDomain]::CurrentDomain.add_ReflectionOnlyAssemblyResolve({ param($s,$e)
  $n = (New-Object System.Reflection.AssemblyName $e.Name).Name
  foreach ($d in @($dir, $cps, "$cps\plug", "$cps\plug\ComplexRule")) { $p = Join-Path $d "$n.dll"; if (Test-Path $p) { return [System.Reflection.Assembly]::ReflectionOnlyLoadFrom($p) } }
  try { return [System.Reflection.Assembly]::ReflectionOnlyLoad($e.Name) } catch { return $null } })
$a = [System.Reflection.Assembly]::ReflectionOnlyLoadFrom($dll)
try { $types = $a.GetTypes() } catch [System.Reflection.ReflectionTypeLoadException] { $types = $_.Exception.Types | ? { $_ } }
$bf = [System.Reflection.BindingFlags]'Public,NonPublic,Instance,Static,DeclaredOnly'
foreach ($t in $types | ? { $_.FullName -match $typePattern }) {
  "########## TYPE $($t.FullName)"
  foreach ($f in $t.GetFields($bf)) { if ($f.Name -like 'CS$*') { continue }; $c = ''; if ($f.IsLiteral) { try { $c = ' = ' + $f.GetRawConstantValue() } catch {} }; "  FIELD $($f.FieldType.Name) $($f.Name)$c" }
  foreach ($mb in @($t.GetConstructors($bf)) + @($t.GetMethods($bf))) {
    if ($mb.Name -notmatch $methodPattern) { continue }
    $ps = try { ($mb.GetParameters() | % { "$($_.ParameterType.Name) $($_.Name)" }) -join ', ' } catch { '?' }
    "=== METHOD $($mb.Name)($ps)"
    [Pdc]::Dump($mb)
  }
}
