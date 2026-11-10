# M14-01 — PTY transport spike and client

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | M |
| Depends on | T0 |
| Parallel with | M14-02 |
| Owns | `crates/protocol/src/pty.rs`, `crates/engine/src/features/pty.rs`, `crates/engine/tests/M14-01.rs`, `docs/spikes/pty-transport.md` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs` |

## 1. Goal

Prove, with a real server, how `/api/pty/{id}/connect` streams bytes, then
implement the smallest client that can create a PTY, read output, resize it,
and delete it. The spike result is a checked-in document; the transport choice
is made from evidence, not assumption.

This is a spike **with a deliverable**, not research: the node is done when the
e2e test reads real bytes from a real PTY.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `POST /api/pty` | body `{command, args, cwd, title, env}` (+ `location`) → `{location, data: Pty}` |
| `GET /api/pty` | `{location, data: [Pty]}` |
| `GET /api/pty/{ptyID}` / `PUT` / `DELETE` | one PTY |
| `POST /api/pty/{ptyID}/connect-token` | header `x-opencode-ticket` → `{ticket, expires_in}` |
| `GET /api/pty/{ptyID}/connect` | query `ticket`, `cursor`, `location[directory]` — **documented as a boolean, actually a stream** |

`Pty` = `{id, title, command, args, cwd, status: "running"|"exited", pid,
exitCode?}`. Events: `pty.created`, `pty.updated`, `pty.exited`,
`pty.deleted`.

## 3. Spike questions (answer in `docs/spikes/pty-transport.md`)

1. Is the connect response raw bytes, SSE, chunked JSON, or a WebSocket
   upgrade?
2. What does `cursor` mean — byte offset, sequence, or opaque token?
3. Does a reconnect with the last cursor replay missed output? How much?
4. What are the resize and input frames? (An HTTP `PUT`? Frames on the same
   stream? A second endpoint?)
5. What happens on ticket expiry mid-stream?
6. Does `DELETE` terminate the process (check `pid`), or only detach?

Record the raw first 200 bytes (hex) for each answer. If the transport is a
WebSocket, write ADR 0002 before implementing; if it is SSE/JSON, no ADR.

Fallback: if connect cannot stream at all, stop and record the failure; the
fallback is `/api/experimental/session/{id}/terminal` (M14-03 uses the same
grid model either way).

## 4. Protocol

```rust
pub struct PtyProcess { pub id: String, pub title: String, pub command: String,
                        pub args: Vec<String>, pub cwd: String,
                        pub status: PtyStatus, pub pid: u32, pub exit_code: Option<u32> }

pub enum PtyStatus { Running, Exited, Unknown(String) }

pub struct PtyTicket { pub ticket: String, pub expires_in: u64 }
```

## 5. Engine

Commands `CreatePty{request}`, `ListPtys`, `ResizePty{id, cols, rows}`,
`WritePty{id, bytes}`, `DeletePty{id}`, `ConnectPty{id}`.

Events `PtyCreated{pty}`, `PtyOutput{id, bytes}` (batched, see below),
`PtyExited{id, code}`, `PtyClosed{id}`, `PtyError{id, message}`.

Batching: output is coalesced into ≤ 16 ms chunks and ≤ 64 KB per event, so a
`yes` command cannot flood the channel with one event per byte.

## 6. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `pty_parses_exited_without_code` | optional exit code |
| protocol | `pty_status_unknown_is_kept` | forward compatibility |
| engine | `pty_output_is_batched_by_time_or_size` | ≤ 16 ms / 64 KB |
| engine | `pty_ticket_is_requested_before_connect` | auth order |

E2E (`tests/M14-01.rs`, no model gate): create a PTY running
`/bin/sh -c 'printf hello; sleep 5'` in the temp project, connect, assert the
first bytes are `hello`, resize to 100x30 (no error), delete, assert the
process is gone (`kill -0` fails) and `GET /api/pty` no longer lists it.
Durable assertion: the OS process and the server list.

## 7. Acceptance

- [ ] The spike document answers all six questions with evidence.
- [ ] Output arrives and is batched.
- [ ] Delete terminates the process, not only the connection.
- [ ] The transport decision is recorded before the view depends on it.

## 8. Risks

WebSocket support would add a dependency and an auth path. That is exactly why
the spike runs first, alone, with no UI.
