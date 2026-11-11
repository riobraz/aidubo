# M13-04 — Plugin status

| | |
| --- | --- |
| Status | in-progress |
| Wave | 1 |
| Size | S |
| Depends on | T0 |
| Parallel with | M13-01 … M13-03 |
| Owns | `crates/protocol/src/plugin.rs`, `crates/core/src/plugins.rs`, `crates/engine/src/features/plugins.rs`, `crates/views/src/plugins_settings.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

Settings → Plugins shows what is installed, where it came from, whether it is
active or failed, and whether an update exists. Updating is an explicit button
with confirmation; the app never installs anything silently.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/plugin` | query `location[directory]` → `{location, data: [Plugin.Info]}` |
| `GET /api/plugin/gui` | GUI plugins |
| `POST /api/plugin/check` | run an update check |
| `POST /api/plugin/update` | update one plugin |

`Plugin.Info` = `{id, source, features, state}`.
`source`: `builtin`, `package{target, version?, outdated?, updating?}`,
`local{path}`, `sdk`. `state`: `active` or `failed{error, ref}`.

## 3. Protocol

```rust
pub enum PluginSource { Builtin, Package { target: String, version: Option<String>,
                                          outdated: bool, updating: bool },
                        Local { path: String }, Sdk, Unknown { kind: String } }

pub enum PluginState { Active, Failed { error: String, reference: Option<String> },
                       Unknown { raw: String } }
```

Fixture: one plugin per source and state, plus an unknown source kind.

## 4. Engine

Commands `ListPlugins{request_id}`, `CheckPlugins{request_id}`,
`UpdatePlugin{id}`. Events `PluginsLoaded`, `PluginsChecked`,
`PluginUpdated{id}`, `PluginError{id, message}`.

`CheckPlugins` is rate-limited to one call per 60 s, enforced in the engine.

## 5. Core

`PluginStore`: rows sorted by `id`; `error` text truncated to 400 characters.
A `failed` plugin shows its reference (a log path) as text, never as a click.

## 6. UI

Rows: id, source badge (`builtin`, `package 1.2.3`, `local`, `sdk`), state.
A package row with `outdated` shows "Update" (confirmation: "This runs the
plugin's update command."). Failed rows expand to show the error.

## 7. Non-goals

Installing a plugin from a URL, editing plugin config, running arbitrary
commands for a plugin.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `plugin_source_variants_parse` | four sources |
| protocol | `plugin_failed_state_keeps_error` | failed shape |
| core | `plugin_sorted_and_error_truncated` | display safety |
| engine | `plugin_check_is_rate_limited` | no spam |

E2E (`tests/M13-04.rs`, no model gate): list plugins on a fresh project and
assert the array parses (it may be empty); call `CheckPlugins` twice in a row
and assert the second is refused by the rate limit; update a non-existent id
and assert a clean error. No plugin is installed by the test.

## 9. Acceptance

- [ ] Installed plugins and their state are visible.
- [ ] No update runs without confirmation.
- [ ] Errors are visible as text and never a link.
- [ ] Check cannot be spammed.

## 10. Risks

`/api/plugin/update` runs external code. The app shows the source and target
before confirming and never calls it in the background.
