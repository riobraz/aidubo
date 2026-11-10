# Reference notes: OpenChamber and Monocode Native

Status: reference · 2026-11-10

This note records what the two reference implementations do, how they do it,
and which of their optimizations the post-MVP specs adopt. It is the evidence
behind [`../specs/000-conventions.md`](../specs/000-conventions.md) §12 and the
performance requirements in the specs.

## 1. What was inspected

| Source | Version | What was read |
| --- | --- | --- |
| OpenChamber | 2.7.0 (`/Applications/OpenChamber.app`) | `app.asar` (Electron main, 40 MB), `web-dist` (React SPA, 61 MB), `@openchamber/web` server modules |
| Monocode Native | 0.16.0-preview.14 (`/Applications/Monocode Native.app`) | `monocode-gpui` binary strings: module paths, SQLite schema, embedded changelog, visual SDK docs |
| OpenCode | 2.2.6 (`~/.bun/bin/opencode`) | `/doc/openapi.json`: 139 paths, 147 event names |

Monocode is a closed-source binary; its structure below comes from symbol and
string paths (`crates/monocode-core/src/…`, `crates/monocode-gpui::…`) and from
its embedded release notes. No code was copied.

## 2. OpenChamber

### Architecture

- **Electron main** (`dist-bundle/main.mjs`, 1.4 MB) supervises the app window
  and a **Bun/Express server** (`@openchamber/web`, ~400 modules).
- The server owns everything the agent needs beside the model: git, GitHub/
  GitLab/Gitea/Forgejo/Gerrit, terminals (`node-pty`), dictation
  (sherpa-onnx), tunnels (Cloudflare/ngrok/Tailscale), relay, schedules,
  workflows, Docker/SSH "spaces".
- **Renderer**: React + Zustand + Vite, code-split per view, **TanStack Virtual
  + Virtua**, Shiki, KaTeX, Mermaid, and **web workers** for markdown and
  highlighting (`streamdown-code.worker`, `markdown-shiki.worker`).
- Transport: one upstream SSE connection, fanned out to many WebSocket clients
  with a replay buffer.

### Measured optimization: delta coalescing

The OpenCode stream emits one event per fragment. OpenChamber measured one
answer at **3,402 delta events, 1.7 characters average, 97% of all frames,
1.3 MB on the wire for 5.7 KB of text**.

`server/lib/event-stream/delta-coalescer.js` merges consecutive deltas before
fan-out:

- window **50 ms**, overridable by `OPENCHAMBER_EVENT_DELTA_COALESCE_MS`
  (`0` disables it, which is also the A/B switch);
- merges `session.text.delta`, `session.reasoning.delta`,
  `session.tool.input.delta` only;
- any other event is a **barrier**: it flushes pending text, so nothing crosses
  a `*.ended` snapshot or a status change;
- bounds: **64 KB** pending text, **64** pending streams — the limit flushes
  early, nothing is dropped;
- the merged event carries the id of its **last** fragment, so a client replay
  cursor always lands on a boundary.

The hub keeps **2,048 events / 8 MB** of replay and invents ids for upstream
frames that have none, prefixing them per process so a cursor from before a
restart misses instead of matching an unrelated sequence.

### Other optimizations

- **Search**: SQLite **FTS5 with the trigram tokenizer** over settled user and
  assistant messages only; live sessions are debounced 1.5 s, backfill is
  paginated and paused, and the file is derived data (a schema change rebuilds
  it).
- **Startup**: engine start overlaps workspace and optional UI loading;
  `startup-performance.js` and `session-warming` prewarm sessions.
- **Caches**: `realpath` cache, provider read cache, git status cache with
  serialized refresh, skills catalog disk cache.
- **Concurrency**: `map-limited.js` caps parallel work; the upstream reader
  raises its stall timeout 3× when several streams are open and backs off
  250 ms → 5 s.

## 3. Monocode Native

### Architecture

- Rust workspace: `crates/monocode-core` (logic, 130+ modules),
  `crates/monocode-gpui` (~80 view modules), `crates/mobile-link`.
- GPUI is **vendored** (`vendor/gpui-pre`), not taken from crates.io.
- **SQLite is the local source of truth**: `sessions`, `mono_blocks`,
  `mono_turns`, `mono_transcripts`, `notes`, reminders. The transcript is a
  block table, not a re-fetch from the agent on every open.
- **Multi-provider by design**: `provider/opencode`, `provider/acp`,
  `provider/devin`, `provider/pi`, plus model catalogs for Claude, Codex, Pi,
  omp, Hermes. The app talks to agent CLIs, not only OpenCode.
- Surfaces: transcript, diffs, explorer, file editor, terminal
  (`alacritty_terminal`), worktrees, review pane, inbox (GitHub/Linear/Jira/
  Azure DevOps), notes, automations, quick open, project search, SSH remote
  projects, mobile host.

### Stated optimizations (from its embedded release notes)

- **Transcript pools** avoid rebuilding; swapping sessions keeps the recently
  rendered transcript and scroll position, and **prefetches neighbouring
  sessions**.
- **Staged initial render**, memoized sidebar cards, deferred cleanup, and
  no redundant transcript saves.
- **One background syntax-highlighting job at a time**.
- Streaming words are revealed at **120–400 ms**, adapting to the stream's
  cadence instead of painting every token; tool steps use paced masked
  entrances; motion respects Reduce Motion.
- Provider usage is cached across Settings and footer, refreshed on request or
  a polling interval, avoiding duplicate CLI probes.
- Background artwork effects run **in a worker** and are cached by image
  revision and theme.
- Startup overlaps workspace loading with optional UI loading.

### Native visual SDK

`monocode-gpui visual check|show` validates and publishes inline visuals
written as a JavaScript function body against a `visual` SDK: `column`, `row`,
`grid`, `card`, `stat`, `table`, `chart` (bar/line/area), `gauge`, `donut`,
`heatmap`, `progress`, controls, `canvas`, and a 3D `scene`. The sandbox has no
DOM, network, files, timers, or imports; state is local to the visual. Style is
semantic (`ink`, `muted`, `accent`, `positive`, `series1..6`) and inherited
from the app.

## 4. What the specs adopt

| Reference finding | Adopted in |
| --- | --- |
| Server-side delta coalescing with barriers and caps | `harness-engine` batching rule, `000-conventions` §6; enforced per feature in M9-01/M9-04 |
| Replay cursor must name a frame boundary | resnapshot rules, `000-conventions.md` §4 reconnect |
| FTS5 trigram search over settled messages | out of scope for these nodes; candidate after M13 |
| SQLite as the local transcript source | M16-03 export/import keeps the server authoritative; local cache is a later ADR |
| Prefetch neighbouring sessions, keep scroll | M11-01 slice 3 (per-project drafts and scroll) |
| One background highlight job | M14-03 and M10-03 perf budgets |
| Paced reveal (120–400 ms) | not adopted: GPUI text streaming is already frame-coalesced; revisit after M9 |
| Virtualize long lists | M10-03, M9-01, M14-03 |
| Semantic visual tokens (Monocode visual SDK) | not adopted; the app has no inline visual surface |

## 5. What deliberately does not transfer

- OpenChamber's server-side database, relay, tunnels, and multi-surface runtime
  exist because it serves web, mobile, and VS Code. A native single-user app
  keeps the sidecar as the only server.
- Monocode's multi-provider abstraction is a product decision, not an
  optimization; `rust-harness` stays an OpenCode client (ADR 0001).
- Neither reference is copied. The specs reimplement behavior against the
  documented OpenCode API.
