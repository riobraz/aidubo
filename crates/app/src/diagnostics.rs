//! Redacted diagnostics report.

/// Everything the report needs. Kept as data so it is testable.
pub struct ReportInput<'a> {
    pub app_version: &'a str,
    pub target: &'a str,
    pub os: &'a str,
    pub opencode: &'a str,
    pub engine: &'a str,
    pub data_dir: &'a str,
    pub errors: &'a [String],
    pub home: Option<&'a str>,
}

/// Render the report. Every string is redacted before it is written.
pub fn render(input: &ReportInput<'_>) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Rust Harness {} ({})\n",
        input.app_version, input.target
    ));
    out.push_str(&format!("{}\n", redact(input.os, input.home)));
    out.push_str(&format!(
        "OpenCode: {}\n",
        redact(input.opencode, input.home)
    ));
    out.push_str(&format!("Engine: {}\n", redact(input.engine, input.home)));
    out.push_str(&format!("Paths: {}\n", redact(input.data_dir, input.home)));
    out.push_str(&format!("Errors (last {}):\n", input.errors.len().min(5)));
    for error in input.errors.iter().take(5) {
        let redacted = redact(error, input.home);
        let trimmed: String = redacted.chars().take(200).collect();
        out.push_str(&format!("  - {trimmed}\n"));
    }
    out
}

/// Remove credentials and the home path from free-form text.
pub fn redact(text: &str, home: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(home) = home.filter(|home| !home.is_empty()) {
        out = out.replace(home, "~");
    }
    for scheme in ["Bearer ", "Basic "] {
        out = redact_after_prefix(&out, scheme);
    }
    for prefix in ["sk-", "ghp_", "xoxb-", "xoxp-"] {
        out = redact_after_prefix(&out, prefix);
    }
    out = redact_query_secrets(&out);
    redact_userinfo(&out)
}

fn redact_after_prefix(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(prefix) {
        let (before, after) = rest.split_at(index);
        out.push_str(before);
        out.push_str(prefix);
        let token_end = after[prefix.len()..]
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '}' | ')'))
            .map(|offset| prefix.len() + offset)
            .unwrap_or(after.len());
        out.push_str("[redacted]");
        rest = &after[token_end..];
    }
    out.push_str(rest);
    out
}

fn redact_query_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'outer: while let Some(index) = rest.find(['?', '&']) {
        let (before, after) = rest.split_at(index + 1);
        out.push_str(before);
        let key_end = after
            .find('=')
            .map(|offset| (offset, &after[..offset]))
            .filter(|(_, key)| {
                matches!(
                    key.to_ascii_lowercase().as_str(),
                    "key" | "token" | "password" | "secret" | "api_key" | "access_token"
                )
            });
        if let Some((offset, key)) = key_end {
            out.push_str(key);
            out.push('=');
            let value = &after[offset + 1..];
            let end = value
                .find(|ch: char| ch == '&' || ch.is_whitespace())
                .unwrap_or(value.len());
            out.push_str("[redacted]");
            rest = &value[end..];
            continue 'outer;
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn redact_userinfo(text: &str) -> String {
    let Some(scheme) = text.find("://") else {
        return text.to_string();
    };
    let authority_start = scheme + 3;
    let rest = &text[authority_start..];
    let authority_end = rest
        .find(|ch: char| ch == '/' || ch.is_whitespace())
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let Some(at) = authority.rfind('@') else {
        return text.to_string();
    };
    format!("{}://[redacted]@{}", &text[..scheme], &rest[at + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_redacts_authorization_headers() {
        assert_eq!(
            redact("Authorization: Bearer abc.def.ghi", None),
            "Authorization: Bearer [redacted]"
        );
    }

    #[test]
    fn diagnostics_redacts_bearer_and_basic() {
        assert_eq!(redact("Bearer sk-secret", None), "Bearer [redacted]");
        assert_eq!(redact("Basic dXNlcjpwYXNz", None), "Basic [redacted]");
    }

    #[test]
    fn diagnostics_redacts_provider_keys() {
        assert_eq!(redact("key sk-12345678", None), "key sk-[redacted]");
        assert_eq!(redact("key ghp_abcdef12", None), "key ghp_[redacted]");
        assert_eq!(redact("key xoxb-123456", None), "key xoxb-[redacted]");
    }

    #[test]
    fn diagnostics_redacts_query_secrets() {
        assert_eq!(
            redact("http://h/p?token=abc&x=1", None),
            "http://h/p?token=[redacted]&x=1"
        );
    }

    #[test]
    fn diagnostics_drops_url_userinfo() {
        assert_eq!(
            redact("http://user:pass@host/path", None),
            "http://[redacted]@host/path"
        );
    }

    #[test]
    fn diagnostics_replaces_home_with_tilde() {
        assert_eq!(redact("/Users/alice/x", Some("/Users/alice")), "~/x");
    }

    #[test]
    fn diagnostics_report_has_no_secrets() {
        let errors = vec!["request failed: http://h/?api_key=zzz".to_string()];
        let report = render(&ReportInput {
            app_version: "0.1.0",
            target: "aarch64-apple-darwin",
            os: "macOS 15.1",
            opencode: "2.2.6",
            engine: "ready on 127.0.0.1:1234",
            data_dir: "/Users/alice/Library/Application Support/Rust Harness",
            errors: &errors,
            home: Some("/Users/alice"),
        });
        assert!(report.contains("Rust Harness 0.1.0"));
        assert!(!report.contains("zzz"), "{report}");
        assert!(!report.contains("/Users/alice"), "{report}");
    }
}
