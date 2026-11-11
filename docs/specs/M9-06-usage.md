# M9-06 — Usage totals and provider budgets

| | |
| --- | --- |
| Status | done |
| Wave | 1 |
| Size | S |
| Depends on | T0 |
| Parallel with | M9-01 … M9-05 |
| Owns | `crates/protocol/src/usage.rs`, `crates/core/src/usage.rs`, `crates/engine/src/features/usage.rs`, `crates/views/src/usage_footer.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

The footer shows the session's tokens and cost, and a budgets panel shows the
provider limits when the server exposes them. Missing data renders as
"unknown"; it is never shown as zero.

Boundary with M9-04: M9-04 owns the *context meter* (window share) and the
compaction state. This node owns *cost/token totals* and *provider budgets*.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/experimental/usage/limits` | `{location, data: UsageLimits.Info \| null}`; 503 `ServiceUnavailable` is normal |

`UsageLimits.Info` = `{workspace, go, budgets}`. `Session.Info` carries
`tokens: {input, output, reasoning, cache{read, write}}` and `cost: number`.
Events: `session.usage.updated`, `session.usage.recorded`.

`budgets` entries are parsed into `Budget { provider, label?, used?, limit?,
reset_at? }` with `#[serde(default)]` on every field; the fixture is recorded
during implementation and unknown fields are preserved but not shown.

## 3. Protocol

```rust
pub struct UsageLimits { pub budgets: Vec<Budget>, pub workspace: Option<Budget> }

pub struct Budget {
    pub provider: String,
    pub label: Option<String>,
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub reset_at: Option<i64>,
}
```

Fixture: `data: null`, a 503 body, and a budget with only `provider`.

## 4. Engine

Command `LoadUsageLimits{request_id}`. Event
`UsageLimitsLoaded{request_id, result: Result<UsageLimits, String>}`.

A 503 maps to `Ok(UsageLimits { budgets: vec![], workspace: None })` with a
`note: Some("limits unavailable")`, not an error toast.

Resnapshot: load limits when the engine becomes `Ready` and when the budgets
panel opens; never poll faster than once a minute.

## 5. Core

`UsageStore`: `limits`, `last_loaded_at`, `unavailable: bool`.

Rules:

- A `null` payload means "not available for this workspace"; keep the previous
  values only if they are younger than 10 minutes, otherwise show "unknown".
- `used`/`limit` are `Option`; a percentage is computed only when both exist.
- Cost is never summed across sessions by the app; show the server's number.

## 6. UI

Footer: `1,204 in · 388 out · $0.07` for the open session. A budgets popover
lists each provider with a progress bar when both values exist, and
"unknown" otherwise. No network indicator flicker for a 503.

## 7. Non-goals

Spend forecasting, alerts, quota enforcement, cross-session aggregation.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `usage_limits_null_data_parses` | empty payload |
| protocol | `budget_missing_fields_default` | partial budget |
| core | `usage_limits_503_is_unavailable_not_error` | no error toast |
| core | `budget_percentage_needs_both_values` | no divide by zero |
| core | `stale_limits_expire_after_ten_minutes` | unknown, not stale numbers |

E2E (`tests/M9-06.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`): after one
turn, assert the session snapshot reports non-zero tokens; call
`LoadUsageLimits` and accept either a payload or `unavailable` — both are valid
for a local workspace, and the test asserts the engine does not emit an error
event in the 503 case.

## 9. Acceptance

- [ ] Missing limits never surface as an error dialog.
- [ ] A budget with no `limit` renders "unknown", not 0%.
- [ ] Footer values come from the session snapshot/events only.
- [ ] No polling loop; limits load on demand and on `Ready`.

## 10. Risks

The endpoint is experimental and may change shape. The parser is defensive and
the node must not be a dependency of anything except M12-01's budget display.
