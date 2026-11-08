# Security Policy

## Supported versions

Rust Harness is pre-1.0. Security fixes land on the latest release and on
`main`; older releases are not maintained. When reporting, include the version
or commit you tested.

## Reporting a vulnerability

Please do **not** open a public issue for a security problem.

- Preferred: use GitHub private vulnerability reporting ("Report a
  vulnerability" in the repository's **Security** tab), if enabled.
- Otherwise, contact the maintainers privately. Include:
  - the affected version/commit and platform;
  - reproduction steps or a proof of concept;
  - impact and any known mitigations.

We aim to acknowledge within 3 business days and to give a remediation timeline
after triage. Please allow us to ship a fix before public disclosure.

## Threat model

Rust Harness is a native GPUI client. It launches and supervises an
`opencode serve` sidecar and talks to it over loopback HTTP and SSE.

Trust boundaries:

- The sidecar is the only component that touches project files, credentials,
  and session storage. The UI never reads OpenCode's storage directly.
- The sidecar binds to `127.0.0.1` on a random port with a random per-launch
  password passed through `OPENCODE_SERVER_PASSWORD`. The password is never
  written to disk.
- The OpenCode binary is external: we locate, launch, and monitor it, but we do
  not patch or fork it.
- Project file contents and tool output are untrusted data. They are rendered
  as text; the UI does not execute links or commands from them.

### In scope (our code)

- Sidecar lifecycle: process handling, port selection, credential handling.
- Request/response and SSE parsing in `harness-engine` and `harness-protocol`.
- Rendering of untrusted model/tool output without executing it.
- Leakage of credentials, prompts, or message content through logs or local
  persistence.
- Local state persistence and app-local configuration.

### Out of scope

- Vulnerabilities in OpenCode itself or in configured providers/models —
  report those upstream.
- Issues that require an attacker to already control the local user account,
  the project directory, or the `opencode` binary.
- Denial of service caused by a malicious local process.
- Findings in dependencies without a demonstrated impact on Rust Harness.

## Security practices

- Loopback-only bind, random port, per-launch basic auth.
- Never log prompts, message content, tokens, or credentials; redact URLs that
  could carry secrets.
- Treat tool output and file contents as untrusted when rendering.
- Never auto-approve permission requests.
- Pin and verify the sidecar binary before launch once packaging exists (M7).
- Treat unknown events and message variants as ignorable so a server upgrade
  cannot crash the UI.

## Disclosure policy

We credit reporters in the advisory unless they prefer to remain anonymous.
