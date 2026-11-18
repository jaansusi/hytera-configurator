# Recording what CPS sends to the radio

Use this whenever a question can only be answered by watching CPS work, for example the write path or how a
single edited setting changes the files.

## How it works

`tools/serial_tap.js` is a Frida script injected into `G2CPS.Main.exe`. It hooks the Windows serial-port calls
(`CreateFileW`, `SetCommState`, `SetCommTimeouts`, `EscapeCommFunction`, `PurgeComm`, `WriteFile`, `ReadFile`,
`GetOverlappedResult`, `GetQueuedCompletionStatus`, `CloseHandle`). It writes one JSON line per event to
`captures/serial_log.jsonl`. It only observes: it never changes arguments or return values. Received bytes
arrive through I/O completion ports (`via: "iocp"`), because .NET's `SerialPort` uses overlapped I/O.

Event types: `attached`, `open` (path), `dcb` (baud/bits/parity/stop), `timeouts`, `escape` (5/6 = set/clear DTR,
3/4 = set/clear RTS), `purge`, `tx`/`rx` (`n` bytes, `hex`), `close`. Each event has a millisecond timestamp `t`.

## Steps

1. One-time setup (already done on this PC): `python -m venv .venv` and
   `.venv\Scripts\python -m pip install -r requirements.txt` in the project folder.
2. Start CPS as usual (it runs elevated) and connect the radio.
3. Right-click `tools\start_tap.cmd` and choose **Run as administrator**. Expect `tapping pid …` and an
   `attached` message.
4. Do the operation in CPS (Read, Write, …).
5. Stop: create an empty file `tools\tap.stop` (a non-admin shell can do this), or close the console window.
6. Copy `captures\serial_log.jsonl` to a descriptive name (e.g. `write-unchanged_<date>.jsonl`). It cannot be
   renamed while CPS is still running, because the hook's file handle lives inside the CPS process. Delete the
   original after closing CPS: the recorder **appends**, so a leftover file mixes recordings.
7. Decode: `.venv\Scripts\python tools\parse_capture.py captures\<file>.jsonl --out captures\<file>_files`
   (add `--frames` for a frame-by-frame listing).

## Notes

- `parse_capture.py` understands the read and write commands. Extend it when new opcodes appear.
- Captures contain the radio's full configuration, including any encryption keys stored in it. Keep them private.
