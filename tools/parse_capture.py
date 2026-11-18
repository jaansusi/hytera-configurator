"""Decode a Hytera CPS serial capture (JSONL from serial_tap.js) into frames and radio files.

Usage:
    python parse_capture.py <serial_log.jsonl> [--out DIR] [--rcdb-fileid XML] [--frames]

Writes one <fileId>_<name>.bin per file read from (or written to) the radio, and reports checksum
checks. A single stray 00 byte after a 64-byte-multiple frame is normal (CPS sends it) and is
reported as a desync warning.
Protocol reference: ../docs/protocol.md
"""
import argparse, json, os, re, sys, zlib

SYNC = 0x5E
HDR_LEN = 20
LINK_TYPES = {0xFE: 'CONNECT', 0xFD: 'CONNECT_ACK', 0xFB: 'DISCONNECT', 0xFA: 'DISCONNECT_ACK',
              0x00: 'DATA', 0x10: 'ACK'}


def inet_checksum(frame: bytes) -> int:
    """RFC 1071 ones'-complement sum over the whole frame with bytes 18..19 zeroed."""
    b = bytearray(frame)
    b[18:20] = b'\0\0'
    if len(b) % 2:
        b += b'\0'
    s = sum(int.from_bytes(b[i:i + 2], 'big') for i in range(0, len(b), 2))
    while s >> 16:
        s = (s & 0xFFFF) + (s >> 16)
    return ~s & 0xFFFF


def load_frames(path):
    events = [json.loads(line) for line in open(path, encoding='utf-8')]
    frames = []
    for direction in ('tx', 'rx'):
        buf, marks = bytearray(), []
        for e in events:
            if e['ev'] == direction:
                marks.append((len(buf), e['t']))
                buf += bytes.fromhex(e['hex'])
        i = 0
        while i + HDR_LEN <= len(buf):
            if buf[i] != SYNC:
                print(f'warning: {direction} desync at byte {i}', file=sys.stderr)
                i += 1
                continue
            length = int.from_bytes(buf[i + 16:i + 18], 'big')
            t = max((tt for pos, tt in marks if pos <= i), default=None)
            frames.append((t, direction, bytes(buf[i:i + length])))
            i += length
    frames.sort(key=lambda f: (f[0], f[1] != 'tx'))
    return frames


def file_names(xml_path):
    if not xml_path or not os.path.exists(xml_path):
        return {}
    text = open(xml_path, encoding='utf-8-sig').read()
    return {int(a): b for a, b in re.findall(r'<FileId>(\d+)</FileId>\s*<FileName>([^<]+)</FileName>', text)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('capture')
    ap.add_argument('--out', default='radio_read')
    ap.add_argument('--rcdb-fileid', default=r'C:\Program Files (x86)\Hytera\CPSStarter\CPS\V1.5.24.000.0003\RCDB\FileID\V1.5.24.000.xml')
    ap.add_argument('--frames', action='store_true', help='print every frame')
    a = ap.parse_args()

    frames = load_frames(a.capture)
    bad = [f for _, _, f in frames if inet_checksum(f) != int.from_bytes(f[18:20], 'big')]
    print(f'{len(frames)} frames, {len(bad)} with bad link checksum')
    t0 = frames[0][0] if frames else 0
    if a.frames:
        for t, d, f in frames:
            print(f'{t - t0:>7} {d} {LINK_TYPES.get(f[3], hex(f[3])):14} seq={int.from_bytes(f[6:8], "big"):>4} {f[20:].hex(" ")}')

    names = file_names(a.rcdb_fileid)
    pending, files, current = {}, {}, None
    for _, d, f in frames:
        if f[3] != 0x00:
            continue
        p = f[20:]
        op, sub, txn = p[2:4].hex(), p[4:6].hex(), p[6:10]
        if d == 'tx' and op == '01f0' and sub == '0006':        # open file for read
            current = int.from_bytes(p[12:14], 'big')
            files[current] = {}
        elif d == 'tx' and op == '41f8' and sub == '0008':      # read chunk n
            pending[txn] = (current, int.from_bytes(p[12:14], 'big'))
        elif d == 'rx' and op == '51f8' and sub == '0008' and txn in pending:   # read chunk data
            fid, idx = pending.pop(txn)
            files[fid][idx] = p[16:]
        elif d == 'tx' and op == '01f0' and sub == '000b':      # open file for write
            current = int.from_bytes(p[12:14], 'big')
            files[current] = {}
        elif d == 'tx' and op == '41f8' and sub == '0005':      # write chunk n (resends overwrite)
            files[current][int.from_bytes(p[12:14], 'big')] = p[14:]

    os.makedirs(a.out, exist_ok=True)
    for fid, chunks in files.items():
        blob = b''.join(chunks[i] for i in sorted(chunks))
        name = names.get(fid, 'unknown')
        open(os.path.join(a.out, f'{fid:03d}_{name}.bin'), 'wb').write(blob)
        crc_ok = len(blob) >= 20 and zlib.crc32(blob[20:]) == int.from_bytes(blob[12:16], 'little')
        print(f'file {fid:>3} {name:28} {len(blob):>6} bytes  crc32[20:] {"ok" if crc_ok else "MISMATCH"}')


if __name__ == '__main__':
    main()
