# Hytera CPS (G2 / Auctus platform) radio programming protocol

Recovered on 2026-11-18 by recording Hytera CPS V1.5.24.000.0003 reading from and writing to a
BP515 U1 (firmware V1.5.36.001) over the USB programming cable. Captures (decoder: `tools/parse_capture.py`):

- `captures/read_2026-11-18.jsonl`: full Read (648 frames, all checksums valid).
- `captures/write-unchanged_2026-11-18.jsonl`: full Write of the configuration just read, unchanged (615 frames, all
  checksums valid).

Status: **read and write paths observed, implemented and verified live** on the BP515 (2026-11-18).

## 1. Transport

| Item | Value |
|---|---|
| Port | USB serial cable (Prolific PL2303GC), shows up as `COM3` here |
| Line settings | 115200 baud, 8 data bits, no parity, 1 stop bit, no flow control |
| Modem lines | CPS clears RTS and DTR right after opening |
| Read timeout | 100 ms (`ReadTotalTimeoutConstant`) |

CPS opens the port, works, then closes it again; the port is free while CPS is idle.

## 2. Link layer (big-endian)

Every frame has a 20-byte header, optionally followed by a payload.

| Offset | Size | Field | Observed values |
|---|---|---|---|
| 0 | 1 | Sync | `0x5E` |
| 1 | 1 | Constant | `0x6E` |
| 2 | 1 | Version | `0x01` |
| 3 | 1 | Frame type | `FE` connect, `FD` connect ack, `FB` disconnect, `FA` disconnect ack, `00` data, `10` ack |
| 4 | 1 | Source | `0x20` = PC, `0x10` = radio |
| 5 | 1 | Destination | `0x10` / `0x20` |
| 6 | 2 | Sequence | data frames count from 1 in each session; an ack repeats the sequence of the frame it acknowledges |
| 8 | 3 | Zero | `00 00 00` |
| 11 | 1 | Constant | `0x01` |
| 12 | 1 | Flag | `01` on data frames and on PC acks; `00` on radio acks and connect/disconnect frames |
| 13 | 3 | Zero | `00 00 00` |
| 16 | 2 | Total length | header + payload |
| 18 | 2 | Checksum | RFC 1071 Internet checksum over the **whole frame** with this field set to 0 (pad odd lengths with one zero byte) |

Every data frame is acknowledged by the other side with a type-`10` frame carrying the same sequence number.
One request/response exchange looks like:

```
PC    -> DATA  seq=n   (request)
radio -> ACK   seq=n
radio -> DATA  seq=n   (response)
PC    -> ACK   seq=n
```

Session open/close (exact bytes from the capture):

```
PC    -> 5e 6e 01 fe 20 10 00 00 00 00 00 01 00 00 00 00 00 14 7f 6e   CONNECT
radio -> 5e 6e 01 fd 10 20 00 00 00 00 00 01 00 00 00 00 00 14 8f 5f   CONNECT_ACK
...
PC    -> 5e 6e 01 fb 20 10 00 00 00 00 00 01 00 00 00 00 00 14 7f 71   DISCONNECT
radio -> 5e 6e 01 fa 10 20 00 00 00 00 00 01 00 00 00 00 00 14 8f 62   DISCONNECT_ACK
```

## 3. Application layer (payload of DATA frames, big-endian)

| Offset | Size | Field |
|---|---|---|
| 0 | 2 | Constant `0x0A0B` |
| 2 | 2 | Opcode. A response sets bit `0x1000` (`0003` → `1003`, `01F0` → `11F0`, `41F8` → `51F8`) |
| 4 | 2 | Sub-command |
| 6 | 4 | Transaction ID. CPS keeps incrementing it across sessions; the radio echoes it back |
| 10 | 2 | Data length |
| 12 | n | Data |

Strings in this layer are **UTF-16BE**. (Strings inside the radio's files are UTF-16LE, see section 7.)

### Commands observed

| Purpose | Opcode / sub | Request data | Response data |
|---|---|---|---|
| Get radio info item | `0003` / `0002` | `u16 item` | `u16 status, u16 item, u16 len, value` (usually UTF-16BE text). Status 1 = item not supported (len 0) |
| Unknown query | `0003` / `001B` | `00 00` | `00 00 00 01`. CPS sends it once during identification, between items `0xFFF6` and `0xFFF4` |
| Enter programming session | `01F0` / `0000` | `u8 mode` (`00` read, `01` write) | `u16 status` (0 = OK) |
| Open file for reading | `01F0` / `0006` | `u16 fileId, u16 chunkSize` (CPS uses `0x0400`) | `u16 status, u32 fileSize` |
| Read chunk | `41F8` / `0008` | `u16 chunkIndex` (1-based) | `u32 chunkIndex, bytes[≤chunkSize]` |
| Open file for writing | `01F0` / `000B` | `u16 fileId, u32 fileSize, u16 chunkSize` (`0x0400`) | `u16 status` |
| Write chunk | `41F8` / `0005` | `u16 chunkIndex` (1-based), `bytes[≤chunkSize]` | `u32 chunkIndex` |
| Leave programming session | `01F0` / `0001` | `u8 mode, 00` (`00 00` read, `01 00` write) | `u16 status` |

Example (open `Comm_Basic`, file 18, then read it):

```
tx 0a0b 01f0 0006 00000016 0004 | 0012 0400          open file 0x12, chunk 1024
rx 0a0b 11f0 0006 00000016 0006 | 0000 00000198      status 0, size 408
tx 0a0b 41f8 0008 00000017 0002 | 0001              read chunk 1
rx 0a0b 51f8 0008 00000017 019c | 00000001 <408 bytes>
```

A file is read as `ceil(fileSize / chunkSize)` chunks. CPS works strictly one request at a time: it sends the next
request only after acknowledging the previous response. (An earlier note here about overlapping requests was an
artifact of equal timestamps in the recording.) Matching responses to requests by transaction ID is still the safe
way to decode recordings.

### Info items read by CPS (from this radio)

| Item | Value | Probable meaning |
|---|---|---|
| 0 | `55P10B00-1L0BAPU1U1…-00040000` | model code / feature string |
| 1 | `V1.5.36.001` | firmware version |
| 2 | `V1.5.24.000` | RCDB (file layout) version → selects `RcdbJsons\*.txt` |
| 3 | `V1.6.03.001` | other component version |
| 6 | `V1.0.03.024.AP.BP.U1` | hardware / board version |
| 10 | `0010412200000006` | radio ID / band info (unconfirmed) |
| 11 | `R23N141382` | serial number |
| 17 | `BP515 U1` | model name |
| 0xFFF0 | `DMR` | product type |
| 0xFFF6 | `2024-07-10` | last programming date (CPS writes today's date into `Comm_Basic` on every write) |

## 4. Full read sequence used by CPS

1. Open COM port, 115200 8N1, clear RTS and DTR.
2. Session 1: CONNECT, 20 info queries (`0003`), DISCONNECT. CPS uses this to identify the model and RCDB version.
   Exact order of items: `0002, 0000, 0011, 0001, 0003, 000A, 000B, FFF0, 0009, FFF6, [001B query], FFF4, 0008,
   0007, 0100, 0045, 0006, 0200, 0065, 0071` (also `CPS_IDENTIFY_SEQUENCE` in `crates/hytera-proto/src/session.rs`).
   Items 7, 8 and 9 answer status 1 (not supported) on the BP515. Transaction IDs start at 1 and keep counting
   into session 2.
3. Session 2: CONNECT, enter programming (`01F0/0000`), then for each file ID in the RCDB file list: open (`01F0/0006`) and read chunks (`41F8/0008`); then leave (`01F0/0001`) and DISCONNECT.
4. Close port. A full read of 58 files (about 50 KB) took about 5.7 s.

Every transfer completed without error; no challenge, login or password exchange appeared anywhere in the capture.

## 5. Full write sequence used by CPS

Recorded writing back the configuration it had just read, without changes. Took about 10 s for 47 KB.

1. Open the port as for reading.
2. Session 1: identification, identical to the read (same 20 queries in the same order). CPS's transaction IDs
   continued from its earlier read (started at `0xA1`).
3. Session 2: CONNECT, enter programming in **write** mode (`01F0/0000`, data `01`), then for each file: open for
   writing (`01F0/000B` with ID, size, chunk size) and write chunks 1..n (`41F8/0005`); then leave
   (`01F0/0001`, data `01 00`) and DISCONNECT. Nothing else follows: no separate commit or restart command.
   **Afterwards the radio is unavailable for a few seconds** (observed with `hyt`, 2026-11-18): it ACKed the next
   CONNECT/identify request but did not respond within 5 s; seconds later it answered normally. Most likely it
   restarts to load the new settings (inferred). CPS does not talk to the radio right after a write.
4. **55 of the 58 files are written.** Not written: 242 `QEU_C_E2EEKEYLIST`, 245 `QEU_C_BTMatchedList`,
   253 `Comm_RadioInfo`.
5. **Order:** the read order, except that each one-level file of a `TableLink.xml` pair goes immediately before its
   two-level partner: 7 before 6, 81 before 80, 39 before 38, 85 before 84, 162 before 161, 180 before 179
   (165/166 and 167/168 are already in that order). Implemented as `write_order` in `crates/hytera-codeplug/src/rcdb.rs`.
6. The file contents CPS sends are described in [file-format.md](file-format.md) ("Preparing files for writing").

## 6. Timing, resends and an extra byte

- **The radio sometimes ignores a request** while it is busy writing to flash: no ACK, no response. CPS resends the
  identical frame (same sequence number and transaction ID) about every **920 ms** until it is acknowledged. Seen 3
  times during the write: once for the first chunk of `Comm_Basic` (18), twice for the first chunk of
  `Comm_Service_Setting` (87). The radio then ACKs within about 40 ms and responds once.
- Other write-chunk responses arrive within about 220 ms; read responses within about 20 ms.
- **Extra `00` byte:** after every frame whose total length is an exact multiple of 64 bytes, CPS writes one extra
  byte `00` (seen 3 of 3 times: 320-byte frames carrying 286-byte files 165, 167 and 247; never after any other
  frame). 64 bytes is the USB full-speed packet size, so this is most likely a workaround for the USB serial adapter
  (inferred). The radio skips the byte while looking for the next `5E`. The radio never sends such a byte.

`hytera-proto` follows all of this: 900 ms ACK timeout, up to 5 resends, 5 s for a response once acknowledged, and
the extra byte.

## 7. What travels inside the files

The bytes returned by "read chunk" are the radio's settings files. Their container header, CRC and field layout
are described in [file-format.md](file-format.md). Unresolved questions are tracked in
[open-questions.md](open-questions.md).
