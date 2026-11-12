# M13-02 — Providers and integration auth

| | |
| --- | --- |
| Status | done |
| Wave | 1 |
| Size | L |
| Depends on | T0 |
| Parallel with | M13-01, M13-03, M13-04 |
| Owns | `crates/protocol/src/provider.rs`, `crates/core/src/providers.rs`, `crates/engine/src/features/providers.rs`, `crates/views/src/providers_settings.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

Settings → Providers lists provider accounts with their auth status and lets
the user connect one through the server's own flow: OAuth in the browser, a
command the user runs, an API key typed into a masked field, or an external
link. The app never stores a credential and never logs one.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/provider` | query `location[directory]` → `{location, data: [Provider.Info]}` |
| `GET /api/provider/{providerID}` | one provider |
| `GET /api/integration` | `{location, data: [Integration.Info]}` |
| `GET /api/integration/{integrationID}` | one integration |
| `POST /api/integration/{id}/connect/oauth` | start → attempt |
| `GET/DELETE /api/integration/{id}/connect/oauth/{attemptID}` | poll / cancel |
| `POST /api/integration/{id}/connect/oauth/{attemptID}/complete` | finish |
| `POST /api/integration/{id}/connect/command` | start a command |
| `GET/DELETE /api/integration/{id}/connect/command/{attemptID}` | poll / cancel |
| `POST /api/integration/{id}/connect/key` | send a key |
| `POST /api/integration/{id}/connect/external` | open an external URL |

`Provider.Info` = `{id, canonical, integrationID, name, activation, package,
compatibility, settings, headers, body}`.
`Integration.Info` = `{id, name, metadata, methods, connections}`;
`methods` is `OAuth | Command | Key | External | Env`; `connections` is
`CredentialInfo | EnvInfo`.

Live v2.2.7 check: `external` methods carry `id`, `label` and a `form` array of
field keys — no URL. Command methods carry an argv array, and Key/OAuth methods
carry a nullable form. The dialog therefore never invents a link for an
external method; it explains that credentials are managed outside the app and
offers only an explicit Complete action.

## 3. Protocol

```rust
pub struct ProviderInfo { pub id: String, pub name: String,
                          pub integration_id: Option<String>,
                          pub activation: Option<String>, pub package: Option<String> }

pub enum IntegrationMethod { OAuth { .. }, Command { .. }, Key { .. },
                             External { .. }, Env { .. }, Unknown { kind: String } }
```

Method payloads are parsed into typed structs only for `Key` and `OAuth`
(label, scopes, url); the rest keep their raw JSON so an unknown method still
renders a disabled row instead of breaking the page.

Fixtures: provider with only `id`/`name`; integration with an unknown method;
connection of type `env`.

## 4. Engine

Commands `ListProviders{request_id}`, `ListIntegrations{request_id}`,
`StartOAuth{integration_id}`, `PollOAuth{integration_id, attempt_id}`,
`CancelOAuth{integration_id, attempt_id}`, `CompleteOAuth{...}`,
`StartCommandAuth{integration_id}`, `PollCommandAuth{...}`,
`SendKey{integration_id, key}`, `OpenExternalAuth{integration_id}`.

Events: `ProvidersLoaded`, `IntegrationsLoaded`, `AuthAttemptStarted{kind,
attempt_id, url_or_command}`, `AuthAttemptUpdated{status}`,
`AuthAttemptFailed{message}`, `ProviderConnected{id}`.

Polling: the engine polls an attempt every 1 s for at most 5 minutes, then
stops and emits a timeout. The UI never polls.

## 5. Security (mandatory)

- The API key is passed as a function argument, never stored in an entity,
  never serialized into an event, never logged.
- `SendKey` zeroizes its buffer after the request (`zeroize` or a manual
  overwrite; no new dependency required).
- External URLs are opened only by an explicit user click, only `https`, and
  only after showing the host.
- A command auth shows the exact command and requires confirmation before the
  attempt starts.
- Error messages never include the request body or headers.

## 6. Core

`ProviderStore`: providers, integrations, per-integration attempt state
(`Idle | AwaitingUser | Polling | Failed | Connected`), and a redacted
`last_error`.

## 7. UI

Provider rows: name, package, activation, connection badge, Connect/Reconnect.
A connect flow is a small dialog per method: OAuth shows the instructions and
"Open browser (host)" plus Check now and Cancel; Command shows the exact argv
with Copy and requires an explicit "I ran this command" before the attempt
starts; Key shows a masked input with paste allowed, no reveal, and clears it
after sending; External has no server-provided URL in v2.2.7, so it states that
credentials are managed outside the app and offers Complete.

## 8. Non-goals

Editing provider headers/bodies, model routing, quota display (M9-06).

## 9. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `provider_minimal_fields_parse` | optionals |
| protocol | `integration_unknown_method_is_disabled` | forward compatibility |
| core | `provider_attempt_state_machine` | cancel/timeout |
| core | `provider_error_is_redacted` | no secrets in messages |
| engine | `key_never_appears_in_debug_output` | `Debug` redaction |
| engine | `oauth_poll_stops_at_timeout` | no infinite poll |

E2E (`tests/M13-02.rs`, no model gate): list providers and integrations on a
fresh project and assert the arrays parse; start a `key` attempt against a
fake integration id and assert the error is 404-shaped with no key material;
cancel a non-existent attempt and assert a clean terminal event (v2.2.7 answers
the cancel with success). Real OAuth is a manual check recorded in `STATUS.md`.

## 10. Acceptance

- [x] Provider list renders with partial data.
- [x] No secret reaches a log, an event, or a `Debug` output.
- [x] Cancel stops polling within one interval.
- [x] No external URL is rendered because the server exposes none; the dialog
      states that credentials are managed outside the app. Any URL that is
      rendered (OAuth) requires a click, is `https` only, and shows the host.

## 11. Risks

Highest-risk surface in the roadmap. Security review is a merge gate for this
node; a failure here blocks the wave.
