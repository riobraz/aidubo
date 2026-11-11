# ADR 0003 — PTY transport is a WebSocket

- **Status:** Accepted
- **Date:** 2026-11-11
- **Deciders:** Rust Harness maintainers
- **Related:** [Architecture](../ARCHITECTURE.md), [M14-01 spec](../specs/M14-01-pty-transport.md), [spike evidence](../spikes/pty-transport.md)

> The M14-01 spec says "write ADR 0002"; 0002 is already the terminal crate, so
> this decision is recorded as 0003.

## Context

M14 needs a shell dock. The sidecar exposes `/api/pty` for management, but the
connect endpoint is documented only as a boolean while the spike showed it
upgrades to a WebSocket. The client needs to stream output, send keystrokes,
resize, and resume after a reconnect without duplicating text.

Measured behavior on opencode v2.2.7 (details in the spike):

- `GET /api/pty/{id}/connect?ticket=…` is a WebSocket upgrade; output arrives
  as text frames, cursor updates as binary frames prefixed with NUL and JSON.
- `POST /api/pty/{id}/connect-token` requires `x-opencode-ticket: 1`; tickets
  are single-use and valid for about 60 s.
- `cursor` is a byte offset into the server's output buffer; a reconnect
  replays from that offset.
- Input is raw bytes; resize is an HTTP `PUT`, not a frame.
- `DELETE` terminates the OS process.

## Decision

Use `tokio-tungstenite` for the PTY socket, with TLS features disabled because
the sidecar is loopback-only:

```toml
tokio-tungstenite = { version = "0.30", default-features = false, features = ["connect", "handshake"] }
```

The engine owns the connection and:

1. requests a fresh ticket immediately before each connect;
2. builds the `ws://127.0.0.1:{port}` URL with the ticket and optional cursor,
   never logging or echoing it;
3. sends input as binary frames (raw bytes, no client-side JSON);
4. treats binary frames that begin with `0x00` plus `{"cursor":N}` as cursor
   updates, and every text frame as output;
5. batches output into ≤ 16 ms / ≤ 64 KiB `PtyOutput` events;
6. resizes with `PUT /api/pty/{id}` and deletes with `DELETE`, which ends both
   the socket and the process.

The engine parses the cursor frames so they are never rendered as output, but
it does not count frame bytes; a reconnect must use a cursor the server
reported, not a local count, because the server counts raw PTY bytes while the
frames are lossily re-encoded UTF-8.

## Consequences

- One new dependency (`tokio-tungstenite` + `tungstenite`) and tokio's `net`
  feature. TLS backends stay out; `wss://` is not supported and is not needed
  on loopback.
- `harness-views` stays IO-free: it sends engine commands and reads
  `PtyOutput`/`PtyClosed` events.
- Reconnect is not automatic in this node. A dropped socket reports
  `PtyClosed`/`PtyError`; reconnecting from the last server-reported cursor is
  a later view decision (M14-02) so the UI can avoid duplicating replayed
  output.
- Ticket expiry only affects the handshake; established streams keep working.
  Tests still request one ticket per connect to match the contract.
- If OpenCode later changes the control frame or drops the ticket header, the
  spike document and this ADR must be re-verified before the view relies on it.
