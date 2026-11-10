# M16-02 — Signed update check

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | M |
| Depends on | E0 |
| Parallel with | M16-01, M16-03, M16-04 |
| Owns | `crates/app/src/update.rs`, `crates/protocol/src/update.rs`, `packaging/update-manifest.schema.json` |
| Shared touch | `app/src/main.rs`, `README.md` |

## 1. Goal

On launch the app checks a signed manifest and reports "up to date",
"available", or "unavailable" without blocking startup. It never installs
anything silently; the strongest action is "download and verify", then reveal
the file.

## 2. Manifest

```json
{
  "version": "0.2.0",
  "channel": "stable",
  "artifacts": [
    { "os": "macos", "arch": "aarch64", "url": "https://…/RustHarness-0.2.0.dmg",
      "sha256": "…", "size": 123456789 }
  ],
  "notes_url": "https://…/notes/0.2.0",
  "issued_at": 1762732800,
  "signature": "base64(ed25519(signature_payload))"
}
```

`signature_payload` is the compact JSON of every field except `signature`,
serialized deterministically (sorted keys, no whitespace). The public key is
compiled into the app; rotation requires a release that trusts both keys for
one version.

Verification order: fetch → parse → signature → `version > current` →
artifact for `(os, arch)` → (on download) `sha256` and `size`.

## 3. States

```rust
pub enum UpdateState {
    UpToDate { checked_at: i64 },
    Available { version: String, url: String, sha256: String, notes_url: Option<String> },
    Unavailable { reason: String, checked_at: i64 },
}
```

- `Unavailable` covers offline, bad signature, no artifact, and malformed
  JSON. It never blocks startup and never shows a modal.
- A check runs at most once per 24 h, on launch, on a background task.
- "Download" saves to the Downloads directory, verifies the hash, then reveals
  the file. It never executes or installs it.
- No request is made when the setting is off.

## 4. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| `harness-protocol` | `update_manifest_parses_and_round_trips` | schema |
| `harness-app` | `update_signature_rejects_tampered_payload` | signature |
| `harness-app` | `update_signature_rejects_tampered_hash` | artifact |
| `harness-app` | `update_version_compare_is_numeric` | 0.10 > 0.9 |
| `harness-app` | `update_missing_arch_is_unavailable` | wrong platform |
| `harness-app` | `update_download_verifies_hash` | no corrupt install |

E2E (`tests/M16-02.rs`, no model gate): start a local HTTP server serving a
fixture manifest signed with a test key (injected via a test-only trust
override), assert `Available`; tamper one byte and assert `Unavailable`; stop
the server and assert `Unavailable` within the timeout. No real network.

## 5. Acceptance

- [ ] A tampered manifest or artifact never shows "available".
- [ ] Startup is never blocked by the check.
- [ ] Download verifies before revealing.
- [ ] Offline is quiet, not an error dialog.

## 6. Risks

The update channel is a security-sensitive download path. The signature is
verified before the URL is even shown, the key is pinned, and the feature can
be disabled by setting. An ADR is required before adding auto-install (out of
scope).
