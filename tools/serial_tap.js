// Passive serial tap for a 32-bit .NET process using System.IO.Ports.SerialPort.
// Logs COM open/config and every byte written/read (incl. overlapped/IOCP completions)
// as JSON lines. Observes only; never modifies arguments or return values.
'use strict';

const LOG_PATH = 'C:\\Users\\Jaan\\Desktop\\cps\\captures\\serial_log.jsonl';
const log = new File(LOG_PATH, 'a');
function emit(o) { o.t = Date.now(); log.write(JSON.stringify(o) + '\n'); log.flush(); }

function exp(name) {
  for (const m of ['KERNELBASE.dll', 'kernel32.dll']) {
    try { const a = Process.getModuleByName(m).findExportByName(name); if (a) return a; } catch (e) {}
  }
  throw new Error('export not found: ' + name);
}

const GetFileType = new NativeFunction(exp('GetFileType'), 'uint32', ['pointer']);
const charHandles = new Map();          // handle string -> bool (is char device / COM)
const comNames = new Map();             // handle string -> path
function isCom(h) {
  const k = h.toString();
  if (!charHandles.has(k)) charHandles.set(k, GetFileType(h) === 2 /* FILE_TYPE_CHAR */);
  return charHandles.get(k);
}
function hex(p, n) {
  if (n <= 0 || p.isNull()) return '';
  const b = new Uint8Array(p.readByteArray(n));
  let s = ''; for (let i = 0; i < b.length; i++) s += (b[i] < 16 ? '0' : '') + b[i].toString(16);
  return s;
}

const pending = new Map();              // OVERLAPPED* -> {h, buf}

Interceptor.attach(exp('CreateFileW'), {
  onEnter(a) { this.path = a[0].isNull() ? '' : a[0].readUtf16String(); },
  onLeave(r) {
    if (/COM\d+|\\\\\.\\/i.test(this.path) && r.toInt32() !== -1) {
      comNames.set(r.toString(), this.path); charHandles.delete(r.toString());
      emit({ ev: 'open', h: r.toString(), path: this.path });
      send({ ev: 'open', path: this.path });
    }
  }
});

Interceptor.attach(exp('CloseHandle'), {
  onEnter(a) { const k = a[0].toString(); if (comNames.has(k)) { emit({ ev: 'close', h: k }); comNames.delete(k); } charHandles.delete(k); }
});

Interceptor.attach(exp('SetCommState'), {
  onEnter(a) {
    const d = a[1];
    const o = { ev: 'dcb', h: a[0].toString(), baud: d.add(4).readU32(), flags: d.add(8).readU32(),
      byteSize: d.add(18).readU8(), parity: d.add(19).readU8(), stopBits: d.add(20).readU8() };
    emit(o); send(o);
  }
});

Interceptor.attach(exp('SetCommTimeouts'), {
  onEnter(a) { const p = a[1]; emit({ ev: 'timeouts', h: a[0].toString(), v: [0,4,8,12,16].map(o => p.add(o).readU32()) }); }
});

Interceptor.attach(exp('EscapeCommFunction'), {
  onEnter(a) { emit({ ev: 'escape', h: a[0].toString(), fn: a[1].toInt32() }); }   // 5=SETDTR 6=CLRDTR 3=SETRTS 4=CLRRTS
});

Interceptor.attach(exp('PurgeComm'), {
  onEnter(a) { emit({ ev: 'purge', h: a[0].toString(), flags: a[1].toInt32() }); }
});

Interceptor.attach(exp('WriteFile'), {
  onEnter(a) {
    if (!isCom(a[0])) return;
    const n = a[2].toUInt32();
    emit({ ev: 'tx', h: a[0].toString(), n, hex: hex(a[1], n) });
  }
});

Interceptor.attach(exp('ReadFile'), {
  onEnter(a) {
    this.com = isCom(a[0]);
    if (!this.com) return;
    this.h = a[0]; this.buf = a[1]; this.nRead = a[3]; this.ov = a[4];
    if (!this.ov.isNull()) pending.set(this.ov.toString(), { h: this.h.toString(), buf: this.buf });
  },
  onLeave(r) {
    if (!this.com) return;
    if (r.toInt32() !== 0 && !this.nRead.isNull()) {
      const n = this.nRead.readU32();
      if (n > 0) { emit({ ev: 'rx', h: this.h.toString(), n, hex: hex(this.buf, n), via: 'sync' }); if (!this.ov.isNull()) pending.delete(this.ov.toString()); }
    }
  }
});

Interceptor.attach(exp('GetOverlappedResult'), {
  onEnter(a) { this.ov = a[1]; this.n = a[2]; },
  onLeave(r) {
    const k = this.ov.toString(); const p = pending.get(k);
    if (!p || r.toInt32() === 0) return;
    pending.delete(k);
    const n = this.n.readU32();
    if (n > 0) emit({ ev: 'rx', h: p.h, n, hex: hex(p.buf, n), via: 'gor' });
  }
});

Interceptor.attach(exp('GetQueuedCompletionStatus'), {
  onEnter(a) { this.n = a[1]; this.ovp = a[3]; },
  onLeave(r) {
    if (this.ovp.isNull()) return;
    const ov = this.ovp.readPointer(); if (ov.isNull()) return;
    const k = ov.toString(); const p = pending.get(k);
    if (!p) return;
    pending.delete(k);
    const n = this.n.readU32();
    if (n > 0) emit({ ev: 'rx', h: p.h, n, hex: hex(p.buf, n), via: 'iocp' });
  }
});

emit({ ev: 'attached', pid: Process.id, arch: Process.arch });
send({ ev: 'attached', pid: Process.id, arch: Process.arch, log: LOG_PATH });
