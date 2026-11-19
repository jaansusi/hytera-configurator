using System; using System.Text; using System.Linq; using System.Reflection; using System.Reflection.Emit; using System.Collections.Generic;
public static class Pdc {
  static Dictionary<short,OpCode> ops = new Dictionary<short,OpCode>();
  static Pdc(){ foreach(var f in typeof(OpCodes).GetFields(BindingFlags.Public|BindingFlags.Static)){ var o=(OpCode)f.GetValue(null); ops[o.Value]=o; } }
  static string Short(Type t){ if(t==null) return "?"; var n=t.Name; int k=n.IndexOf('`'); if(k>0)n=n.Substring(0,k); return n; }
  public static string Dump(MethodBase mb){
    var sb=new StringBuilder(); MethodBody body; try{ body=mb.GetMethodBody(); }catch(Exception e){ return "  <no body: "+e.Message+">\n"; }
    if(body==null) return "  <no body>\n";
    var il=body.GetILAsByteArray(); var m=mb.Module; int i=0;
    var st=new Stack<string>();
    var targets=new HashSet<int>();
    ParameterInfo[] pars; try{ pars=mb.GetParameters(); }catch{ pars=new ParameterInfo[0]; }
    var lines=new List<KeyValuePair<int,string>>();
    // pre-scan targets
    {
      int j=0; while(j<il.Length){ short v=il[j++]; if(v==0xFE) v=(short)(0xFE00|il[j++]); OpCode o; if(!ops.TryGetValue(v,out o)) break;
        switch(o.OperandType){
          case OperandType.ShortInlineBrTarget: { sbyte d=(sbyte)il[j]; j+=1; targets.Add(j+d); break; }
          case OperandType.InlineBrTarget: { int d=BitConverter.ToInt32(il,j); j+=4; targets.Add(j+d); break; }
          case OperandType.InlineSwitch: { int n=BitConverter.ToInt32(il,j); j+=4; int bas=j+4*n; for(int k=0;k<n;k++){ targets.Add(bas+BitConverter.ToInt32(il,j)); j+=4;} break; }
          case OperandType.InlineNone: break;
          case OperandType.ShortInlineI: case OperandType.ShortInlineVar: j+=1; break;
          case OperandType.InlineVar: j+=2; break;
          case OperandType.InlineI8: case OperandType.InlineR: j+=8; break;
          default: j+=4; break;
        }
      }
    }
    while(i<il.Length){
      int pc=i; short v=il[i++]; if(v==0xFE) v=(short)(0xFE00|il[i++]);
      OpCode o; if(!ops.TryGetValue(v,out o)){ lines.Add(new KeyValuePair<int,string>(pc,"??")); continue; }
      long iv=0; int tok=0; int tgt=-1; int[] sw=null; double rv=0;
      switch(o.OperandType){
        case OperandType.InlineNone: break;
        case OperandType.ShortInlineBrTarget: { sbyte d=(sbyte)il[i]; i+=1; tgt=i+d; break; }
        case OperandType.ShortInlineI: iv=(o.Value==OpCodes.Ldc_I4_S.Value)?(long)(sbyte)il[i]:(long)il[i]; i+=1; break;
        case OperandType.ShortInlineVar: iv=il[i]; i+=1; break;
        case OperandType.InlineVar: iv=BitConverter.ToUInt16(il,i); i+=2; break;
        case OperandType.InlineBrTarget: { int d=BitConverter.ToInt32(il,i); i+=4; tgt=i+d; break; }
        case OperandType.InlineI: iv=BitConverter.ToInt32(il,i); i+=4; break;
        case OperandType.InlineI8: iv=BitConverter.ToInt64(il,i); i+=8; break;
        case OperandType.ShortInlineR: rv=BitConverter.ToSingle(il,i); i+=4; break;
        case OperandType.InlineR: rv=BitConverter.ToDouble(il,i); i+=8; break;
        case OperandType.InlineSwitch: { int n=BitConverter.ToInt32(il,i); i+=4; int bas=i+4*n; sw=new int[n]; for(int k=0;k<n;k++){ sw[k]=bas+BitConverter.ToInt32(il,i); i+=4;} break; }
        default: tok=BitConverter.ToInt32(il,i); i+=4; break;
      }
      string nm0=o.Name;
      if(targets.Contains(pc) && st.Count>0){ lines.Add(new KeyValuePair<int,string>(pc,"/* stack at label: "+string.Join(" | ",st.ToArray())+" */")); }
      string r=Step(m,mb,pars,st,o.Name,iv,rv,tok,tgt,sw);
      if(r!=null) lines.Add(new KeyValuePair<int,string>(pc,r));
    }
    foreach(var kv in lines){ sb.AppendFormat("  {0}{1:X4}: {2}\n", targets.Contains(kv.Key)?"L":" ", kv.Key, kv.Value); }
    return sb.ToString();
  }
  static string Pop(Stack<string> st){ return st.Count>0? st.Pop() : "<?>"; }
  static string Arg(MethodBase mb, ParameterInfo[] pars, int n){ if(!mb.IsStatic){ if(n==0) return "this"; n--; } return n<pars.Length? pars[n].Name : "arg"+n; }
  static string Step(Module m, MethodBase mb, ParameterInfo[] pars, Stack<string> st, string n, long iv, double rv, int tok, int tgt, int[] sw){
    try{
      if(n=="nop"||n.StartsWith("volatile")||n.StartsWith("constrained")||n.StartsWith("readonly")) return null;
      if(n.StartsWith("ldc.i4")){ if(n=="ldc.i4.m1") st.Push("-1"); else if(n.Length==8&&char.IsDigit(n[7])) st.Push(n.Substring(7)); else st.Push(iv.ToString()); return null; }
      if(n=="ldc.i8"){ st.Push(iv.ToString()+"L"); return null; }
      if(n.StartsWith("ldc.r")){ st.Push(rv.ToString()); return null; }
      if(n=="ldnull"){ st.Push("null"); return null; }
      if(n=="ldstr"){ st.Push("\""+m.ResolveString(tok)+"\""); return null; }
      if(n.StartsWith("ldloca")){ st.Push("&V"+iv); return null; }
      if(n.StartsWith("ldloc")){ string k = (n.EndsWith(".s")||n=="ldloc") ? iv.ToString() : n.Substring(6); st.Push("V"+k); return null; }
      if(n.StartsWith("stloc")){ string k = (n.EndsWith(".s")||n=="stloc") ? iv.ToString() : n.Substring(6); return "V"+k+" = "+Pop(st)+";"; }
      if(n.StartsWith("ldarga")){ st.Push("&"+Arg(mb,pars,(int)iv)); return null; }
      if(n.StartsWith("ldarg")){ int k = (n.EndsWith(".s")||n=="ldarg") ? (int)iv : int.Parse(n.Substring(6)); st.Push(Arg(mb,pars,k)); return null; }
      if(n.StartsWith("starg")){ return Arg(mb,pars,(int)iv)+" = "+Pop(st)+";"; }
      if(n=="dup"){ var a=Pop(st); st.Push(a); st.Push(a); return null; }
      if(n=="pop"){ return Pop(st)+";"; }
      if(n=="ret"){ return st.Count>0? "return "+Pop(st)+";" : "return;"; }
      if(n=="br"||n=="br.s"||n=="leave"||n=="leave.s"){ st.Clear(); return "goto IL_"+tgt.ToString("X4")+";"; }
      if(n.StartsWith("brtrue")||n.StartsWith("brfalse")){ var a=Pop(st); return "if ("+(n.StartsWith("brfalse")?"!":"")+"("+a+")) goto IL_"+tgt.ToString("X4")+";"; }
      string[] cmp={"beq","bne.un","bge","bgt","ble","blt","bge.un","bgt.un","ble.un","blt.un"}; string[] sym={"==","!=",">=",">","<=","<",">=",">","<=","<"};
      int ci=Array.IndexOf(cmp, n.EndsWith(".s")? n.Substring(0,n.Length-2):n);
      if(ci>=0){ var b=Pop(st); var a=Pop(st); return "if ("+a+" "+sym[ci]+" "+b+") goto IL_"+tgt.ToString("X4")+";"; }
      if(n=="switch"){ return "switch("+Pop(st)+") goto ["+string.Join(",",sw.Select(x=>"IL_"+x.ToString("X4")).ToArray())+"];"; }
      string[] bin={"add","sub","mul","div","rem","and","or","xor","shl","shr","ceq","cgt","clt","cgt.un","clt.un","div.un","shr.un"}; string[] bs={"+","-","*","/","%","&","|","^","<<",">>","==",">","<",">","<","/",">>"};
      int bi=Array.IndexOf(bin,n); if(bi>=0){ var b=Pop(st); var a=Pop(st); st.Push("("+a+" "+bs[bi]+" "+b+")"); return null; }
      if(n=="not"||n=="neg"){ st.Push((n=="not"?"~":"-")+Pop(st)); return null; }
      if(n.StartsWith("conv")) return null;
      if(n=="ldfld"||n=="ldflda"){ var f=m.ResolveField(tok); st.Push(Pop(st)+"."+f.Name); return null; }
      if(n=="ldsfld"||n=="ldsflda"){ var f=m.ResolveField(tok); st.Push(Short(f.DeclaringType)+"."+f.Name); return null; }
      if(n=="stfld"){ var f=m.ResolveField(tok); var val=Pop(st); var obj=Pop(st); return obj+"."+f.Name+" = "+val+";"; }
      if(n=="stsfld"){ var f=m.ResolveField(tok); return Short(f.DeclaringType)+"."+f.Name+" = "+Pop(st)+";"; }
      if(n=="ldftn"||n=="ldvirtftn"){ if(n=="ldvirtftn") Pop(st); var t=m.ResolveMethod(tok); st.Push("&"+t.Name); return null; }
      if(n=="ldtoken"){ var t=m.ResolveMember(tok); st.Push("token("+t.Name+")"); return null; }
      if(n=="box"||n=="unbox"||n=="unbox.any"||n=="castclass") return null;
      if(n=="isinst"){ var t=m.ResolveType(tok); st.Push("("+Pop(st)+" as "+Short(t)+")"); return null; }
      if(n=="ldlen"){ st.Push(Pop(st)+".Length"); return null; }
      if(n=="newarr"){ var t=m.ResolveType(tok); st.Push("new "+Short(t)+"["+Pop(st)+"]"); return null; }
      if(n.StartsWith("ldelem")){ var idx=Pop(st); var a=Pop(st); st.Push(a+"["+idx+"]"); return null; }
      if(n.StartsWith("stelem")){ var val=Pop(st); var idx=Pop(st); var a=Pop(st); return a+"["+idx+"] = "+val+";"; }
      if(n=="initobj"){ return Pop(st)+" = default;"; }
      if(n=="throw"){ return "throw "+Pop(st)+";"; }
      if(n=="endfinally"){ return "endfinally;"; }
      if(n=="call"||n=="callvirt"||n=="newobj"){
        var t=m.ResolveMethod(tok); int np=t.GetParameters().Length; var args=new string[np]; for(int k=np-1;k>=0;k--) args[k]=Pop(st);
        string a=string.Join(", ",args);
        if(n=="newobj"){ st.Push("new "+Short(t.DeclaringType)+"("+a+")"); return null; }
        string recv = t.IsStatic? Short(t.DeclaringType) : Pop(st);
        string nm=t.Name; string expr;
        if(nm.StartsWith("get_")&&np==0) expr=recv+"."+nm.Substring(4);
        else if(nm.StartsWith("set_")&&np==1) return recv+"."+nm.Substring(4)+" = "+a+";";
        else if(nm=="get_Item") expr=recv+"["+a+"]";
        else expr=recv+"."+nm+"("+a+")";
        var mi=t as MethodInfo; bool isVoid = mi!=null && mi.ReturnType.FullName=="System.Void";
        if(isVoid) return expr+";"; st.Push(expr); return null;
      }
      return "/* "+n+" */";
    }catch(Exception e){ return "/* "+n+" err "+e.Message+" */"; }
  }
}
