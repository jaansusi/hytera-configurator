# Dump the raw IL of a .NET assembly's types (reflection-only load: no CPS code runs).
# Usage: powershell -ExecutionPolicy Bypass -File tools\cps-analysis\ildump.ps1 <path to dll> <type name regex>
param($dll, $typePattern)
$dir = Split-Path $dll
$cps = 'C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003'
$src = @'
using System; using System.Text; using System.Reflection; using System.Reflection.Emit; using System.Collections.Generic;
public static class IlDump {
  static Dictionary<short,OpCode> ops = new Dictionary<short,OpCode>();
  static IlDump(){ foreach(var f in typeof(OpCodes).GetFields(BindingFlags.Public|BindingFlags.Static)){ var o=(OpCode)f.GetValue(null); ops[o.Value]=o; } }
  static string Tok(Module m,int t){
    try{
      switch(t>>24){
        case 0x70: return "\""+m.ResolveString(t).Replace("\n","\n")+"\"";
        case 0x06: case 0x0A: case 0x2B: { var mb=m.ResolveMember(t); return (mb.DeclaringType!=null?mb.DeclaringType.Name+"::":"")+mb.Name; }
        case 0x04: { var f=m.ResolveField(t); return f.DeclaringType.Name+"::"+f.Name; }
        case 0x01: case 0x02: case 0x1B: return m.ResolveType(t).FullName;
        default: return "tok"+t.ToString("X8");
      }
    }catch(Exception){ try{ var mb=m.ResolveMember(t); return (mb.DeclaringType!=null?mb.DeclaringType.Name+"::":"")+mb.Name; }catch{ } return "tok"+t.ToString("X8")+"?"; }
  }
  public static string Dump(MethodBase mb){
    var sb=new StringBuilder(); MethodBody body; try{ body=mb.GetMethodBody(); }catch(Exception e){ return "  <no body: "+e.Message+">\n"; }
    if(body==null) return "  <no body>\n";
    var il=body.GetILAsByteArray(); var m=mb.Module; int i=0;
    while(i<il.Length){
      int start=i; short v=il[i++]; if(v==0xFE) v=(short)(0xFE00|il[i++]);
      OpCode o; if(!ops.TryGetValue(v,out o)){ sb.AppendFormat("  IL_{0:X4}: ??{1:X}\n",start,v); continue; }
      string arg="";
      switch(o.OperandType){
        case OperandType.InlineNone: break;
        case OperandType.ShortInlineBrTarget: { sbyte d=(sbyte)il[i]; i+=1; arg="IL_"+(i+d).ToString("X4"); break; }
        case OperandType.ShortInlineI: case OperandType.ShortInlineVar: arg=((o.Value==OpCodes.Ldc_I4_S.Value)?((sbyte)il[i]).ToString():il[i].ToString()); i+=1; break;
        case OperandType.InlineVar: arg=BitConverter.ToUInt16(il,i).ToString(); i+=2; break;
        case OperandType.InlineBrTarget: { int d=BitConverter.ToInt32(il,i); i+=4; arg="IL_"+(i+d).ToString("X4"); break; }
        case OperandType.InlineI: arg=BitConverter.ToInt32(il,i).ToString(); i+=4; break;
        case OperandType.InlineI8: arg=BitConverter.ToInt64(il,i).ToString(); i+=8; break;
        case OperandType.ShortInlineR: arg=BitConverter.ToSingle(il,i).ToString(); i+=4; break;
        case OperandType.InlineR: arg=BitConverter.ToDouble(il,i).ToString(); i+=8; break;
        case OperandType.InlineSwitch: { int n=BitConverter.ToInt32(il,i); i+=4; int bas=i+4*n; var l=new List<string>(); for(int k=0;k<n;k++){ l.Add("IL_"+(bas+BitConverter.ToInt32(il,i)).ToString("X4")); i+=4;} arg="("+string.Join(",",l)+")"; break; }
        default: { int t=BitConverter.ToInt32(il,i); i+=4; arg=Tok(m,t); break; }
      }
      sb.AppendFormat("  IL_{0:X4}: {1} {2}\n",start,o.Name,arg);
    }
    return sb.ToString();
  }
}
'@
Add-Type -TypeDefinition $src -Language CSharp
[AppDomain]::CurrentDomain.add_ReflectionOnlyAssemblyResolve({ param($s,$e)
  $n = (New-Object System.Reflection.AssemblyName $e.Name).Name
  foreach ($d in @($dir, $cps, "$cps\plug", "$cps\plug\ComplexRule")) { $p = Join-Path $d "$n.dll"; if (Test-Path $p) { return [System.Reflection.Assembly]::ReflectionOnlyLoadFrom($p) } }
  try { return [System.Reflection.Assembly]::ReflectionOnlyLoad($e.Name) } catch { return $null } })
$a = [System.Reflection.Assembly]::ReflectionOnlyLoadFrom($dll)
try { $types = $a.GetTypes() } catch [System.Reflection.ReflectionTypeLoadException] { $types = $_.Exception.Types | ? { $_ } }
$bf = [System.Reflection.BindingFlags]'Public,NonPublic,Instance,Static,DeclaredOnly'
foreach ($t in $types | ? { $_.FullName -match $typePattern }) {
  "########## TYPE $($t.FullName)"
  foreach ($f in $t.GetFields($bf)) { $c = ''; if ($f.IsLiteral) { try { $c = ' = ' + $f.GetRawConstantValue() } catch {} }; "  FIELD $($f.FieldType.Name) $($f.Name)$c" }
  foreach ($mb in @($t.GetConstructors($bf)) + @($t.GetMethods($bf))) {
    $ps = try { ($mb.GetParameters() | % { "$($_.ParameterType.Name) $($_.Name)" }) -join ', ' } catch { '?' }
    "=== METHOD $($mb.Name)($ps)"
    [IlDump]::Dump($mb)
  }
}
