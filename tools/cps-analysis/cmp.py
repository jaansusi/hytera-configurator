"""Compare CPS's parameter database visibility (Param.visible/delete_flag, with the BP515's model-number
overrides) with CPSParams.xlsx's visibility column, for every field on a CPS screen.

Usage: .venv\\Scripts\\python tools\\cps-analysis\\cmp.py <params.tsv>
where params.tsv comes from sdfq.ps1, e.g.
  powershell -File tools\\cps-analysis\\sdfq.ps1 "SELECT param_id, id, delete_flag, visible, alias_en FROM Param" > %TEMP%\\params.tsv
Read-only; see docs/research-cps-visibility.md.
"""
import collections, json, os, sys, zipfile, re
if len(sys.argv) < 2:
    sys.exit(__doc__)
rows = [l.rstrip('\n').split('\t') for l in open(sys.argv[1], encoding='utf-8', errors='replace')][1:]
rows = [r for r in rows if len(r) >= 5 and r[0].isdigit()]
c = collections.Counter(r[1] for r in rows)
dups = sorted((k for k, v in c.items() if v > 1), key=int)
print('rows', len(rows), 'dup ids', len(dups), dups[:30])
byid = collections.defaultdict(list)
for pid, i, dl, vis, name in [r[:5] for r in rows]:
    byid[int(i)].append((int(pid), int(dl), int(vis), name))

# model-number visibility overrides for the BP515
d = json.load(open(os.path.expandvars(r'%LOCALAPPDATA%\hyt\rcdb\V1.5.24.000\models.json'), encoding='utf-8'))
mn = "55P10B00-1L0BAPU1U100000000CB000000000000-00000NN0000000000000000000000000-00040000".replace('-', '')
groups = collections.defaultdict(list)
for g, s, l, v, p in d['conditions']:
    groups[g].append((s, l, [x for x in v.split(',') if x]))
ok = {g for g in groups if all(mn[s-1:s-1+l] in vals for s, l, vals in groups[g])}
override = {}
for g, p, v in d['visible']:
    if g in ok:
        override[p] = v

# layout ids of the BP515's files
L = json.load(open(r'C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003\RcdbJsons\V1.5.24.000.txt', encoding='utf-8-sig'))['detail']
lay = {f['Id']: f for f in L}

ui = json.load(open(os.path.expandvars(r'%LOCALAPPDATA%\hyt\rcdb\V1.5.24.000\ui.json'), encoding='utf-8'))
on_screen = {p for g, p, o in ui['group_params']}

# xlsx column G
xl = r'C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003\CPSParams.xlsx'
z = zipfile.ZipFile(xl)
ss = []
if 'xl/sharedStrings.xml' in z.namelist():
    sx = z.read('xl/sharedStrings.xml').decode('utf-8')
    for si in re.findall(r'<si>(.*?)</si>', sx, re.S):
        ss.append(''.join(re.findall(r'<t[^>]*>(.*?)</t>', si, re.S)))
sheet = 'xl/worksheets/sheet1.xml'
sx = z.read(sheet).decode('utf-8')
xcls = {}
for row in re.findall(r'<row[^>]*>(.*?)</row>', sx, re.S):
    cells = {}
    for ref, attrs, body in re.findall(r'<c r="([A-Z]+)\d+"([^>]*)>(.*?)</c>', row, re.S):
        v = re.search(r'<v>(.*?)</v>', body, re.S)
        t = re.search(r'<t[^>]*>(.*?)</t>', body, re.S)
        val = None
        if 't="s"' in attrs and v:
            val = ss[int(v.group(1))]
        elif t:
            val = t.group(1)
        elif v:
            val = v.group(1)
        cells[ref] = val
    pid = cells.get('D')
    if pid and pid.strip().isdigit():
        xcls.setdefault(int(pid), cells.get('G'))

def cps_visible(i):
    """External build (CurrentCpsVersion=2): Visible in {0,3,4} and delete_flag == 0."""
    rs = byid.get(i)
    if not rs:
        return None, 'no Param row'
    # one row per id expected for these; if several, prefer delete_flag 0
    pid, dl, vis, name = sorted(rs, key=lambda r: (r[1], r[0]))[0]
    v = override.get(i, vis)
    return (v in (0, 3, 4) and dl == 0), f'visible={vis}{"->"+str(v) if i in override else ""} delete_flag={dl}'

mode = sys.argv[1] if len(sys.argv) > 1 else 'diff'
if mode == 'ids':
    for i in map(int, sys.argv[2].split(',')):
        print(i, lay.get(i, {}).get('Name'), cps_visible(i), 'xlsx:', xcls.get(i))
    sys.exit()
files = set(int(x) for x in sys.argv[2].split(',')) if len(sys.argv) > 2 else None
both = '内外部版本都可见'
diff = collections.defaultdict(list)
for i in sorted(on_screen):
    f = lay.get(i)
    if not f or (files and f['File_Id'] not in files):
        continue
    vis, why = cps_visible(i)
    x = xcls.get(i)
    xvis = (x == both) if x else None
    if vis != xvis:
        diff[(vis, xvis)].append((i, f['Name'], f['File_Id'], why, x))
for k, v in diff.items():
    print('== sdf-visible', k[0], 'xlsx-both', k[1], len(v))
    for r in v:
        print('  ', r)
