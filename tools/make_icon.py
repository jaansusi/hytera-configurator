"""Draw the hyt-gui icon (a simple handheld radio) and write PNG + ICO files. Stdlib only."""
import struct, sys, zlib
from pathlib import Path

def render(n):
    px = bytearray(n * n * 4)
    s = n / 256.0
    def put(x, y, rgba):
        i = (y * n + x) * 4
        px[i:i+4] = bytes(rgba)
    def inside_round(x, y, x0, y0, x1, y1, r):
        if x < x0 or x > x1 or y < y0 or y > y1: return False
        cx = min(max(x, x0 + r), x1 - r); cy = min(max(y, y0 + r), y1 - r)
        return (x - cx) ** 2 + (y - cy) ** 2 <= r * r
    for y in range(n):
        for x in range(n):
            X, Y = (x + 0.5) / s, (y + 0.5) / s
            c = None
            if inside_round(X, Y, 8, 8, 248, 248, 48): c = (31, 94, 168, 255)          # background
            if inside_round(X, Y, 150, 28, 172, 110, 10): c = (236, 242, 250, 255)      # antenna
            if inside_round(X, Y, 78, 88, 182, 232, 22): c = (236, 242, 250, 255)       # body
            if inside_round(X, Y, 96, 108, 164, 152, 8): c = (31, 94, 168, 255)         # display
            for gy in (172, 192, 212):                                                  # speaker grille
                if inside_round(X, Y, 98, gy - 4, 162, gy + 4, 4): c = (120, 150, 190, 255)
            put(x, y, c or (0, 0, 0, 0))
    return bytes(px)

def png(n):
    raw = b"".join(b"\x00" + render_cache[n][y*n*4:(y+1)*n*4] for y in range(n))
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")

out = Path(sys.argv[1])
sizes = [16, 32, 48, 128, 256]
render_cache = {n: render(n) for n in sizes}
pngs = {n: png(n) for n in sizes}
(out / "icon.png").write_bytes(pngs[256])
(out / "32x32.png").write_bytes(pngs[32])
(out / "128x128.png").write_bytes(pngs[128])
ico_sizes = [16, 32, 48, 256]
header = struct.pack("<HHH", 0, 1, len(ico_sizes))
offset = 6 + 16 * len(ico_sizes)
entries, data = b"", b""
for n in ico_sizes:
    d = pngs[n]
    entries += struct.pack("<BBBBHHII", n % 256, n % 256, 0, 0, 1, 32, len(d), offset + len(data))
    data += d
(out / "icon.ico").write_bytes(header + entries + data)
print("wrote", sorted(p.name for p in out.iterdir()))
