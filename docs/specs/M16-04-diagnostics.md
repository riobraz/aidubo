# M16-04 — Redacted diagnostics

| | |
| --- | --- |
| Status | done |
| Wave | 1 |
| Size | S |
| Depends on | E0 |
| Parallel with | M16-01 … M16-03 |
| Owns | `crates/app/src/diagnostics.rs`, `crates/app/tests/diagnostics.rs` |
| Shared touch | `app/src/main.rs`, `README.md` |

## 1. Goal

"Copy diagnostics" produces a short report a user can paste into an issue:
versions, platform, engine state, project/session counts, and the last few
errors. It contains no prompt text, no message content, no tokens, and no
credential-bearing URLs.

## 2. Report

```
Rust Harness 0.1.0 (aarch64-apple-darwin)
macOS 15.1 · GPUI 0.3.11 (gpui-pre)
OpenCode 2.2.6 · ready on 127.0.0.1:53124
Projects 2 · Sessions 148 · Selected ses_…
Stream: connected · last event 3 s ago
Errors (last 5):
  - engine: sidecar exited with status 1
  - protocol: unknown event variant (ignored)
Paths: ~/Library/Application Support/Rust Harness
```

Rules:

- No message, prompt, tool output, file path inside a project, or session
  title.
- URLs are reduced to `scheme://host[:port]/…`; userinfo is dropped.
- The home directory becomes `~`.
- Errors are re-redacted on insert (defence in depth), capped at 5 × 200 chars.

## 3. Redaction

Applied to every string field:

| Pattern | Replacement |
| --- | --- |
| `(?i)authorization:\s*\S+` | `Authorization: [redacted]` |
| `(?i)bearer\s+[A-Za-z0-9._~+/-]+=*` | `Bearer [redacted]` |
| `(?i)basic\s+[A-Za-z0-9+/]+=*` | `Basic [redacted]` |
| `sk-[A-Za-z0-9_-]{8,}` | `sk-[redacted]` |
| `ghp_[A-Za-z0-9]{8,}`, `xox[bap]-…` | `[redacted]` |
| `([?&](?:key\|token\|password\|secret\|api_key)=)[^&\s]+` | `$1[redacted]` |
| `//user:pass@` | `//[redacted]@` |
| home directory prefix | `~` |

## 4. Entry points

- Help menu → "Copy diagnostics" (clipboard).
- `rust-harness --diagnostics` prints the report and exits 0 without starting
  the engine or opening a window.

## 5. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| `harness-app` | `diagnostics_redacts_authorization_headers` | header rule |
| `harness-app` | `diagnostics_redacts_bearer_and_basic` | both schemes |
| `harness-app` | `diagnostics_redacts_provider_keys` | `sk-`, `ghp_`, `xoxb-` |
| `harness-app` | `diagnostics_redacts_query_secrets` | `?token=` |
| `harness-app` | `diagnostics_drops_url_userinfo` | `user:pass@` |
| `harness-app` | `diagnostics_replaces_home_with_tilde` | path policy |
| `harness-app` | `diagnostics_never_contains_message_text` | fixture prompt absent |

E2E (`crates/app/tests/diagnostics.rs`): run the built binary with
`--diagnostics`, assert exit 0, stdout contains `Rust Harness` and `OpenCode`
lines, and contains none of the fixture secrets or the sentinel prompt string.
No engine is started, so this test does not need `opencode`.

## 6. Acceptance

- [ ] The report has the fields above and nothing else.
- [ ] A prompt copied into an error message is redacted or dropped.
- [ ] `--diagnostics` exits without a window or a sidecar.
- [ ] The output fits on one screen for a normal install.

## 7. Risks

Redaction is regex-based and can miss a novel secret shape. The report is
deliberately narrow (versions, counts, status) so there is little room to leak;
the tests cover the shapes the app itself can produce.
