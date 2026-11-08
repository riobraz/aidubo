# ADR 0001 — Use OpenCode as the agent engine

- **Status:** Accepted
- **Date:** 2026-11-08
- **Deciders:** Rust Harness maintainers
- **Related:** [Architecture](../ARCHITECTURE.md)

## Context

We are building a native desktop AI coding harness in Rust with GPUI. The
reference product, OpenChamber, is an Electron/TypeScript application: its
desktop, web, VS Code, and mobile surfaces are all clients of the OpenCode agent.

OpenCode v2 already exposes a stable engine boundary:

- `opencode serve` runs a headless HTTP server on loopback.
- `GET /doc` publishes an OpenAPI 3.1 spec.
- `GET /event` and `GET /global/event` stream SSE events, beginning with
  `server.connected`.
- Events are granular enough for native rendering: `session.text.delta`,
  `session.reasoning.delta`, `session.tool.called`, `permission.asked`, and
  related lifecycle events.
- The server owns config discovery, provider authentication, tools, permissions,
  LSP/MCP integration, file edits, git state, and session storage.

Rebuilding that engine would mean reimplementing provider clients, the tool
execution sandbox, permission handling, session persistence, and dozens of
integration surfaces — with no product differentiation, since the engine is not
what makes our harness distinct.

The product differentiation we want is the native experience: instant startup,
a GPU-rendered transcript that stays smooth while tokens stream, low memory use,
real keyboard-first interaction, and OS integration that an Electron shell
cannot match.

## Decision

We will build Rust Harness as a **native GPUI client** that supervises an
`opencode serve` sidecar and communicates with it over loopback HTTP and SSE.

Specifically:

1. **The agent engine is OpenCode.** We do not reimplement the agent loop,
   provider integrations, tools, or permission model in Rust.
2. **The UI is native.** GPUI renders all pixels. No embedded browser, no
   TypeScript UI runtime, no Electron.
3. **The sidecar is supervised.** The app locates, launches, authenticates,
   health-checks, restarts, and terminates the `opencode` process.
4. **OpenCode owns user state.** The app reuses OpenCode's config, credentials,
   and session storage rather than duplicating them.
5. **The boundary is the API, not the source.** We target the published HTTP/SSE
   contract and pin a tested OpenCode version; we do not patch or fork OpenCode
   for the MVP.

The first milestone delivers chat and sessions only. Diff review, editor,
terminal, and multi-surface support are deliberately deferred.

## Consequences

### Positive

- The MVP ships in weeks rather than quarters: the hardest correctness surface
  (agent execution) is already built and battle-tested.
- Session history, providers, auth, and configuration carry over from existing
  OpenCode/OpenChamber installations with no migration.
- The Rust codebase stays small and focused: protocol types, state reducers,
  sidecar lifecycle, and UI.
- The engine can be upgraded independently, and the same UI can later target
  remote servers.
- MIT/Apache-2.0 licensing is compatible across the stack.

### Negative

- The application ships a Node/Bun sidecar, so the download is larger than a
  pure-Rust binary, and we inherit Node runtime startup latency.
- The app depends on an external API. Breaking changes in OpenCode v2 require
  coordinated upgrades and contract tests.
- Users need an OpenCode-supported provider/auth setup; the native app cannot
  simplify that away in the MVP.
- Two runtimes (GPUI and tokio) must be composed deliberately, with a clean
  channel boundary and no shared mutable state.
- Exactly reusing OpenCode's on-disk state limits how far we can diverge from its
  data model.

### Follow-ups and risks

- Resolve how v2 scopes requests to a project directory (the generated client
  passes `Location.Ref { directory }`; the HTTP surface may use a header or
  parameter) before implementing sessions.
- Pin the OpenCode version, verify its binary at launch, and fail with a clear
  diagnostic on version mismatch.
- Add contract tests that assert the MVP endpoints and event names still exist in
  `/doc`.
- Measure sidecar size and startup time in M0; decide bundle-vs-download before
  packaging.
- Treat unknown event variants as ignorable so a server upgrade cannot crash the
  UI.

## Alternatives considered

### Fully native Rust agent

Reimplement providers, tools, sandboxing, permissions, sessions, LSP, and MCP in
Rust.

*Rejected for the MVP.* It offers the smallest binary and maximum control, but it
is months of work, carries the largest correctness and security surface, and
delays the only differentiating part — the native experience.

### UI-only remote client

A GPUI client that connects only to an already-running OpenChamber/OpenCode
server, with no local engine.

*Rejected as the primary mode.* It is the smallest possible scope, but it cannot
deliver a self-contained desktop app: users would have to install and run a
server separately. It remains a plausible future mode once the client exists.

### Reuse OpenChamber's Electron UI

Keep the existing TypeScript UI and wrap or incremental-migrate it.

*Rejected.* It does not deliver the native performance, memory, and integration
goals that motivate the project, and it retains the full Electron stack.

## References

- [Architecture](../ARCHITECTURE.md)
- OpenChamber: <https://github.com/openchamber/openchamber>
- OpenCode: <https://github.com/anomalyco/opencode>
- OpenCode server docs: <https://opencode.ai/docs/server/>
- GPUI: <https://github.com/zed-industries/zed>
