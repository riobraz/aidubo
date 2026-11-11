# Post-MVP specifications

Status: active · 2026-11-10

This directory turns [`../ROADMAP.md`](../ROADMAP.md) into **executable work
units**. Each file is one node of a dependency graph, sized so an autonomous
agent can finish it in one loop: read the spec, write the code, run the gates,
record the result.

The specs are derived from three sources:

1. `../ROADMAP.md` (feature sequence and OpenCode API evidence).
2. The live OpenCode v2.2.6 spec at `/doc/openapi.json`, re-read on
   2026-11-10 (139 paths, 147 event names). Every endpoint below was copied
   from it, not from memory.
3. The reference implementations described in
   [`../../docs/research/references.md`](../research/references.md)
   (OpenChamber 2.7.0, Monocode Native 0.16): feature inventory,
   architecture, and measured optimizations.

## 1. How to read a spec

Every spec has the same header and sections:

| Field | Meaning |
| --- | --- |
| `Status` | `ready`, `in progress`, `blocked`, `done` |
| `Wave` | topological level; a node may start only when every dependency is `done` |
| `Depends on` | node ids that must be `done` |
| `Parallel with` | nodes safe to run at the same time |
| `Owns` | files the node creates or rewrites exclusively |
| `Shared touch` | files every node must edit minimally, tagged `// spec:<id>` |

Sections: goal, verified contract, protocol types, engine, core, UI, non-goals,
unit tests, e2e tests, acceptance, risks, verification.

A node is **not** done because the code compiles. It is done when the unit
tests, the e2e gate, and the verification commands in the spec pass, and its
`STATUS.md` records the real output.

**Clean room, always.** Every node is implemented from the contract in the
spec and from black-box behavior. Never copy source, identifiers, comments,
assets, or fixture strings from OpenChamber, Monocode, or OpenCode's UI. See
[`000-conventions.md`](000-conventions.md) §13.

## 2. The loop

For one node:

1. **Claim.** Set `Status: in progress` in the spec header and create
   `docs/plans/<node-id>/PLAN.md` from the spec (goal, steps, test plan,
   risks). Create `STATUS.md` with `Status: in progress`.
2. **Contract first.** Add or update the protocol types and their serde
   fixtures before any IO. Run `cargo test -p harness-protocol`.
3. **Core second.** Add the pure reducer and its tests. Run
   `cargo test -p harness-core`.
4. **Engine third.** Add the command, the HTTP call, and the event mapping.
   Run `cargo test -p harness-engine`.
5. **View last.** Wire the UI. Run `cargo test -p harness-views`.
6. **Gates.** Run the commands in §5. Fix the cause, never the test.
7. **E2E.** Run the node's e2e test with its gate variable. If the environment
   cannot run it, record the exact skip message in `STATUS.md`; do not claim a
   pass.
8. **Close.** Set `Status: done`, append the commit hash to `STATUS.md`, and
   update the table in §7.

Loop stop rule: after **three** failed fix attempts on the same gate, stop,
set `Status: blocked`, and write the failing command and its output verbatim
into `STATUS.md`. Never weaken a test, never `#[ignore]` a failure, never
disable clippy.

## 3. The graph

[`graph.json`](graph.json) is the machine-readable form: nodes, dependencies,
wave, lane, files, test names, and e2e gate. `scripts/spec-graph.py` validates
it and prints the ready set:

```sh
python3 scripts/spec-graph.py --validate
python3 scripts/spec-graph.py --ready
python3 scripts/spec-graph.py --waves
```

### Lanes

Lanes are independent product streams. Within a lane, order is strict; across
lanes, work is parallel.

| Lane | Stream | Nodes |
| --- | --- | --- |
| A | Conversation | M9-01 … M9-06, M12-01 |
| B | Review | M10-01 → M10-02 → M10-03 |
| C | Workspace | M11-01 → M11-02 → M11-03 |
| D | Automation | M12-02, M12-03 |
| E | Integrations | M13-01 … M13-04 |
| F | Terminal | M14-01 ∥ M14-02 → M14-03 |
| G | Preview | M15-01 |
| H | Distribution | M16-01 … M16-04 |

### Waves

| Wave | Nodes | Parallelism |
| --- | --- | --- |
| 0 | E0, T0, M14-02 | 3 |
| 1 | M9-01…06, M10-01, M10-02, M11-01, M12-02, M12-03, M13-01…04, M14-01, M15-01, M16-01…04 | 21 |
| 2 | M10-03, M11-02, M12-01, M14-03 | 4 |
| 3 | M11-03 | 1 |

Wave 1 is the maximum parallelism the dependency graph allows, not a mandate.
A single developer should walk lane by lane; an agent pool should cap
concurrency at the number of **distinct owned files** it can merge cleanly
(practically 3–5).

### Shared-file protocol

`crates/protocol/src/lib.rs`, `crates/engine/src/lib.rs`,
`crates/engine/src/supervisor.rs`, `crates/engine/src/events.rs`,
`crates/core/src/lib.rs`, and `crates/views/src/workspace.rs` are shared.
Feature nodes:

- put all logic in their owned files;
- add at most one enum variant and one match arm per shared file, appended at
  the end and tagged with `// spec:<id>`;
- never reorder or reformat unrelated lines.

The wave gate (§4) resolves ordering, moves the tagged blocks into the
canonical position, and runs the full suite.

## 4. Wave gates

A gate runs when every node of the wave is `done`:

1. `cargo fmt --all` (the gate may format; feature nodes may not).
2. `cargo clippy --workspace --all-targets -- -D warnings`.
3. `cargo test --workspace`.
4. Every e2e gate that the wave touched, with its env vars.
5. `python3 scripts/spec-graph.py --validate`.
6. Update §7 and the affected `docs/plans/*/STATUS.md`.

Gate nodes are not separate files; they are `kind: "gate"` entries in
`graph.json` with the wave's specs as dependencies.

## 5. Global gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUST_HARNESS_SKIP_E2E=1 cargo test --workspace          # environments without opencode
cargo test -p harness-engine --test sidecar -- --nocapture
python3 scripts/spec-graph.py --validate
```

E2E environment variables:

| Variable | Meaning |
| --- | --- |
| `RUST_HARNESS_SKIP_E2E` | skip every sidecar test |
| `RUST_HARNESS_OPENCODE_BIN` | explicit `opencode` path |
| `RUST_HARNESS_MOCK_PROVIDER` | OpenAI-compatible base URL, one text turn |
| `RUST_HARNESS_MOCK_TOOL_PROVIDER` | mock that streams reasoning and calls a tool |
| `RUST_HARNESS_MOCK_PERMISSION_PROVIDER` | mock that calls a shell tool behind `ask` |
| `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER` | mock driven by a JSON script (T0) |
| `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER_SCRIPT` | path to that script |
| `RUST_HARNESS_PTY_E2E` | run the terminal e2e test |
| `RUST_HARNESS_PACKAGING_E2E` | run packaging smoke tests |
| `RUST_HARNESS_NOTIFY_SINK` | file the notification sink appends to |

## 6. Test policy

- **Unit tests live next to the code** (`#[cfg(test)]`) in `harness-protocol`,
  `harness-core`, and the SSE/API adapters in `harness-engine`.
- **Every spec names its unit tests** in a table; the names are part of the
  contract. Renaming one is a spec change.
- **Every spec names at least one e2e test** that proves a *durable* server
  effect (a session exists, a worktree directory is on disk, an inbox item is
  gone), not just a local event. Two exceptions are explicit: `M14-02` is a
  pure crate with no IO (its recorded-stream fixtures are unit tests, and
  `M14-03` covers the integration), and `M15-01` needs a GUI session (its URL
  and picker policy are unit-tested, and the rest is a recorded manual
  checklist).
- E2E tests must skip cleanly with a clear message when their prerequisite is
  absent, and must assert clean shutdown (no listener left on the port).
- Fixtures are recorded from the live server and committed; unknown variants
  and missing optional fields are mandatory fixture cases.
- No test may depend on the developer's real OpenCode config, credentials, or
  session data. Every test uses a unique temp project and removes it.

## 7. Status

Update this table when a node closes.

| Node | Wave | Status |
| --- | --- | --- |
| E0 engine-bus | 0 | done |
| T0 e2e-harness | 0 | done |
| M14-02 vt-model | 0 | done |
| M9-01 inbox | 1 | done |
| M9-02 fork | 1 | done |
| M9-03 revert | 1 | done |
| M9-04 compaction-context | 1 | done |
| M9-05 forms | 1 | done |
| M9-06 usage | 1 | done |
| M10-01 vcs-status | 1 | done |
| M10-02 diff-model | 1 | done |
| M10-03 diff-view | 2 | done |
| M11-01 projects | 1 | ready |
| M11-02 worktrees | 2 | blocked (M11-01) |
| M11-03 multi-run | 3 | blocked (M11-02) |
| M12-01 goals | 2 | blocked (M9-06) |
| M12-02 schedules | 1 | ready |
| M12-03 notifications | 1 | ready |
| M13-01 mcp | 1 | ready |
| M13-02 providers | 1 | ready |
| M13-03 saved-permissions | 1 | ready |
| M13-04 plugins | 1 | done |
| M14-01 pty-transport | 1 | ready |
| M14-03 terminal-view | 2 | blocked (M14-01, M14-02) |
| M15-01 preview | 1 | ready |
| M16-01 linux-packaging | 1 | ready |
| M16-02 updates | 1 | ready |
| M16-03 session-transfer | 1 | ready |
| M16-04 diagnostics | 1 | done |

## 8. What is deliberately not a node

The icebox in `../ROADMAP.md` stays out: relay, mobile, web/PWA, VS Code,
Slack/Matrix, multi-window, remote servers, git write operations, embedded
editor, voice. A spec that needs one of them starts with an ADR, not a node.

`M15-01` is the only node that may be cancelled without weakening the product.
