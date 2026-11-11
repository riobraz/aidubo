# PTY transport spike (M14-01)

- **Date:** 2026-11-11
- **Server:** opencode v2.2.7 (x86_64, macOS 15.8.1)
- **Method:** a throwaway `opencode serve` on loopback (random password, temp
  project), `curl` for the HTTP endpoints, and Node 24's built-in `WebSocket`
  for the connect endpoint. Payloads below are the bytes actually received on
  the wire. Nothing was copied from a reference implementation.

## 1. What is the connect response?

A **WebSocket upgrade**. `GET /api/pty/{id}/connect` answers the HTTP upgrade,
then the socket carries frames. The OpenAPI document describes it as "Establish
a WebSocket connection streaming PTY output and accepting terminal input"; the
probe confirms it.

First frames for a fresh `/bin/cat` PTY (hex):

```
server -> client  binary  007b22637572736f72223a307d   NUL + {"cursor":0}
client -> server  binary  4142                        raw input "AB"
server -> client  text    4142                        echoed output "AB"
server -> client  binary  007b22637572736f72223a327d   NUL + {"cursor":2}
```

The first control frame arrives before any output. No SSE or chunked-JSON
framing exists on this path.

## 2. What does `cursor` mean?

A **byte offset into the PTY's raw output buffer**, not a sequence number or an
opaque token.

- Fresh connection: the server announces `{"cursor":0}`.
- `cursor=1` on reconnect replayed only `42` (`B`) — the second byte of the
  earlier `AB` output.
- `cursor=0` replayed the full `4142` and then `{"cursor":2}`.
- `cursor=1000` (past the end) replayed nothing and announced `{"cursor":2}`.
- `cursor=garbage` behaved like `0` (full replay) in an earlier probe.

The offset counts raw PTY bytes, not the UTF-8 text that reaches the browser: a
PTY that emitted `ff 00 61 62` produced the text frame `efbfbd 00 61 62`
(invalid `ff` replaced by U+FFFD, three bytes) while the control frame reported
`{"cursor":4}`.

## 3. Does a reconnect with the last cursor replay missed output?

**Yes, from the cursor to the end of the server's retained buffer.** In the
probe, a short-lived session replayed all 117 buffered bytes at `cursor=0`,
replayed from byte 5 at `cursor=5`, and replayed nothing at `cursor=1000`. No
explicit retention limit was exposed; treat replay as best effort and use only
the server-reported cursor on reconnect, never a locally counted frame length.

## 4. What are the resize and input frames?

- **Input:** raw bytes, sent as client text or binary frames. Sending text `A`
  echoed `A`; sending a JSON text frame echoed the JSON literally; sending
  binary `01` followed by JSON echoed `^A{...}`. There is no client-side typed
  command frame.
- **Output:** server text frames only. Invalid UTF-8 is replaced with U+FFFD
  before it leaves the server.
- **Cursor control:** server binary frames prefixed with NUL and a JSON object
  `{"cursor":N}`. These are not terminal output.
- **Resize:** HTTP `PUT /api/pty/{id}` with `{"size":{"rows":R,"cols":C}}`
  (optionally a new `title`). Inside a shell, `stty size` reported `24 80`
  before the call and `30 100` after, and the PUT returned `200` with the
  refreshed `Pty`. Resize is **not** a socket frame.

Hex excerpt from the shell probe (the full prompt is omitted):

```
before  ... 737474792073697a650d0a 32342038300d0a ...   stty size\r\n24 80\r\n
after   ... 737474792073697a650d0a 3330203130300d0a ... stty size\r\n30 100\r\n
```

## 5. What happens on ticket expiry mid-stream?

Tickets are **single-use** and reported with `expires_in: 60`:

- Reusing a ticket closes the handshake with code `1006`.
- A ticket first used after 61 s also closes with `1006`.
- An already established connection is unaffected: output at t=62 s was still
  echoed after the ticket had expired.

So expiry gates the handshake only. Request a ticket immediately before each
connect. `POST /api/pty/{id}/connect-token` additionally requires the header
`x-opencode-ticket: 1` (values such as `true` return `403 Forbidden`).

## 6. Does DELETE terminate the process?

**Yes.** `DELETE /api/pty/{id}` returned `204`; the process from `Pty.pid` was
alive immediately before (`kill -0` succeeded) and gone immediately after
(`kill -0` failed with "No such process"); `GET /api/pty` no longer listed it;
and the open socket closed with code `1000`. DELETE is not a detach.

## Consequence

The transport is a WebSocket, so the change is recorded in
[ADR 0003](../adr/0003-pty-websocket-transport.md) before the view depends on
it. The engine client uses one ticket per connection, sends input as binary
frames, treats NUL-prefixed binary frames as cursor updates, resizes over HTTP,
and batches output into ≤ 16 ms / ≤ 64 KiB events.
