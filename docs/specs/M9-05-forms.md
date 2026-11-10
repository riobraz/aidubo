# M9-05 — Agent forms

| | |
| --- | --- |
| Status | in-progress |
| Wave | 1 |
| Size | M |
| Depends on | T0 |
| Parallel with | M9-01 … M9-04, M9-06 |
| Owns | `crates/protocol/src/form.rs`, `crates/core/src/form.rs`, `crates/engine/src/features/form.rs`, `crates/views/src/form.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

When the agent asks structured questions, a form appears above the composer.
The user answers each field, required fields gate the Reply button, and the
answer unblocks the turn. Nothing is ever answered automatically.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/session/{sessionID}/form` | `{data: [Form.Info]}` |
| `POST /api/session/{sessionID}/form` | body `Form.CreatePayload` → `{data: Form.Info}` |
| `DELETE /api/session/{sessionID}/form/{formID}` | 204 |
| `POST /api/session/{sessionID}/form/{formID}/reply` | body `{answer: {key: Form.Value}}` |

`Form.Info` = `{id, sessionID, title, metadata, fields: [Form.Field]}`.
Errors: `FormInvalidAnswer`, `FormAlreadySettled` (409), `FormNotFound` (404).
Events: `form.created`, `form.replied`, `form.cancelled`.

### Fields (exact shapes, v2.2.6)

| `type` | Fields |
| --- | --- |
| `string` | `key`, `title?`, `description?`, `required?`, `hidden?`, `when?`, `format?` (`email\|uri\|date\|date-time`), `minLength?`, `maxLength?`, `pattern?`, `placeholder?`, `default?`, `options?`, `custom?` |
| `number` | `key`, `title?`, …, `minimum?`, `maximum?`, `default?` (number or `"Infinity" \| "-Infinity" \| "NaN"`) |
| `integer` | same as `number` |
| `boolean` | `key`, `title?`, …, `default?` |
| `multiselect` | `key`, `title?`, …, `options` (required), `minItems?`, `maxItems?`, `custom?`, `default?` |
| `external` | `key`, `type`, `url` (required), `title?`, `description?` |

`Form.Value` = `string | number | "Infinity" | "-Infinity" | "NaN" | boolean |
string[]`.

## 3. Protocol

```rust
pub enum FormField {
    String { key: String, title: Option<String>, required: bool, hidden: bool,
             format: Option<String>, min_length: Option<u64>, max_length: Option<u64>,
             pattern: Option<String>, placeholder: Option<String>,
             default: Option<String>, options: Vec<FormOption>, custom: bool,
             when: Vec<FormWhen> },
    Number { key: String, integer: bool, required: bool, hidden: bool,
             min: Option<FormNumber>, max: Option<FormNumber>,
             default: Option<FormNumber>, when: Vec<FormWhen> },
    Boolean { key: String, required: bool, hidden: bool, default: Option<bool>,
              when: Vec<FormWhen> },
    Multiselect { key: String, required: bool, hidden: bool, options: Vec<FormOption>,
                  min_items: Option<u64>, max_items: Option<u64>,
                  default: Vec<String>, custom: bool, when: Vec<FormWhen> },
    External { key: String, url: String, title: Option<String> },
    Unknown { key: String, kind: String, raw: serde_json::Value },
}
```

`FormNumber` is `Number(f64) | Infinity | NegativeInfinity | NaN` with a custom
serde that accepts both the number and the string form. `Unknown` renders
read-only and is **never** included in the answer.

Fixtures: one form per field type, one with `type: "color"` (unknown), one with
a missing `title` and `required`.

## 4. Engine

Commands `ListForms{session_id}`, `ReplyForm{session_id, form_id, answer}`,
`DismissForm{session_id, form_id}`. Events `FormsLoaded{session_id, forms}`,
`FormReplied{session_id, form_id}`, `FormDismissed{session_id, form_id}`,
`FormError{session_id, form_id, message}`.

`answer` is built by `harness-core` (`FormStore::answer()`), not by the view.

Resnapshot: on `StreamConnected` and session select, reload forms; a form that
disappeared is removed, never kept as a zombie.

## 5. Core

`FormStore`: forms by id, per-form draft answers, validation.

Rules:

- `required` and not `hidden` and empty → Reply disabled with a reason.
- `hidden` fields are never asked; their `default` is not sent unless the
  server's schema requires it (it does not).
- `format`, `minLength`, `maxLength`, `pattern`, `minimum`, `maximum`,
  `minItems`, `maxItems` are validated client-side for immediate feedback; the
  server remains authoritative.
- `Infinity`/`NaN` are represented as `FormNumber`, never as `f64::INFINITY`
  in the UI.
- `external` fields render a link that the **user** clicks; no auto-open.
- An invalid server reply (`FormInvalidAnswer`) keeps the draft and shows the
  message.

## 6. UI

A card above the composer: title, fields in order, per-field error text, and
**Reply** / **Dismiss**. `when` conditions show/hide fields as answers change.
Keyboard: Tab cycles fields, Cmd/Ctrl+Enter replies, Esc focuses the card (a
second Esc dismisses after confirmation). The composer stays disabled while a
form is pending, so a stray Enter cannot skip it.

## 7. Non-goals

Creating forms from the app UI (the endpoint exists for tests and plugins),
multi-page forms, file upload fields.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `form_parses_every_field_type` | six variants |
| protocol | `form_unknown_field_is_preserved` | forward compatibility |
| protocol | `form_number_accepts_infinity_strings` | special values |
| core | `form_required_blocks_reply` | gating |
| core | `form_unknown_field_is_not_answered` | safe default |
| core | `form_multiselect_bounds_validate` | min/max items |
| core | `form_when_hides_field` | conditional visibility |
| engine | `form_reply_body_uses_key_map` | wire shape |

E2E (`tests/M9-05.rs`, no model gate): create a session, `POST /form` with one
field of each type via the engine's raw API helper, assert `ListForms` returns
it, reply with valid values, assert `GET /form` is empty and a second reply
returns 409. Durable assertion: the form is settled on the server.

## 9. Acceptance

- [ ] Every field type renders and round-trips.
- [ ] Reply is impossible while a required field is empty.
- [ ] Unknown field types never block the rest of the form.
- [ ] A settled form cannot be replied to twice.
- [ ] No answer is ever sent without a user action.

## 10. Risks

Server-side validation messages are user-facing text from the server; treat
them as untrusted but display them as plain text. `when` semantics may grow;
unknown conditions default to "visible" only if the field is not hidden.
