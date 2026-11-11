//! Signed update manifest wire types.
//!
//! The signature covers a canonical form of every field except `signature`
//! itself. Canonical here means object keys sorted recursively and no
//! whitespace, so the signed bytes do not depend on the producer's field
//! order or on `serde_json`'s map feature flags.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const INVALID_MANIFEST: &str = "This is not an update manifest.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub artifacts: Vec<UpdateArtifact>,
    #[serde(default)]
    pub notes_url: Option<String>,
    #[serde(default)]
    pub issued_at: i64,
    #[serde(default)]
    pub signature: String,
    /// Fields added by a future release stay in the payload and therefore
    /// inside the signature instead of being dropped on parse.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateArtifact {
    pub os: String,
    pub arch: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl UpdateManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|_| INVALID_MANIFEST.to_string())?;
        if manifest.version.trim().is_empty() {
            return Err(INVALID_MANIFEST.to_string());
        }
        Ok(manifest)
    }

    /// The exact bytes the signature is computed over.
    pub fn signature_payload(&self) -> Result<Vec<u8>, String> {
        let mut value = serde_json::to_value(self)
            .map_err(|_| "Could not encode the update manifest.".to_string())?;
        if let Some(object) = value.as_object_mut() {
            object.remove("signature");
        }
        let mut payload = String::new();
        write_canonical(&value, &mut payload);
        Ok(payload.into_bytes())
    }

    pub fn artifact(&self, os: &str, arch: &str) -> Option<&UpdateArtifact> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.os == os && artifact.arch == arch)
    }
}

impl UpdateArtifact {
    /// `sha256` in the manifest may be written upper or lower case; both name
    /// the same digest.
    pub fn sha256_matches(&self, hex: &str) -> bool {
        self.sha256.eq_ignore_ascii_case(hex)
    }
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(boolean) => out.push_str(if *boolean { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(string) => {
            // A string always encodes; fall back to an escaped form rather
            // than panicking in a signature path.
            match serde_json::to_string(string) {
                Ok(encoded) => out.push_str(&encoded),
                Err(_) => out.push_str("\"\""),
            }
        }
        Value::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(value, out);
            }
            out.push(']');
        }
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                match serde_json::to_string(key) {
                    Ok(encoded) => out.push_str(&encoded),
                    Err(_) => out.push_str("\"\""),
                }
                out.push(':');
                if let Some(value) = object.get(key) {
                    write_canonical(value, out);
                }
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn manifest() -> UpdateManifest {
        UpdateManifest {
            version: "0.2.0".into(),
            channel: "stable".into(),
            artifacts: vec![UpdateArtifact {
                os: "macos".into(),
                arch: "aarch64".into(),
                url: "https://example.test/RustHarness-0.2.0.dmg".into(),
                sha256: "ABC123".into(),
                size: 123,
                extra: Map::new(),
            }],
            notes_url: Some("https://example.test/notes/0.2.0".into()),
            issued_at: 1_762_732_800,
            signature: "ignored".into(),
            extra: Map::new(),
        }
    }

    #[test]
    fn update_manifest_parses_and_round_trips() {
        let raw = json!({
            "version": "0.2.0",
            "channel": "stable",
            "artifacts": [{
                "os": "macos", "arch": "aarch64",
                "url": "https://example.test/app.dmg",
                "sha256": "abc", "size": 10,
                "future": "kept"
            }],
            "notes_url": "https://example.test/notes",
            "issued_at": 1762732800,
            "signature": "c2ln",
            "new_top_level_field": true
        });
        let parsed = UpdateManifest::parse(raw.to_string().as_bytes()).expect("manifest");
        assert_eq!(parsed.version, "0.2.0");
        assert_eq!(parsed.artifacts[0].arch, "aarch64");
        assert_eq!(parsed.artifacts[0].extra["future"], "kept");
        assert_eq!(parsed.extra["new_top_level_field"], true);
        assert_eq!(
            parsed.artifact("macos", "aarch64"),
            Some(&parsed.artifacts[0])
        );
        assert!(parsed.artifact("linux", "x86_64").is_none());
        assert!(parsed.artifacts[0].sha256_matches("AbC"));

        // Serializing and parsing again loses nothing, signature included.
        let encoded = serde_json::to_vec(&parsed).expect("encode");
        let again = UpdateManifest::parse(&encoded).expect("reparse");
        assert_eq!(again, parsed);

        assert!(UpdateManifest::parse(b"not json").is_err());
        assert!(UpdateManifest::parse(br#"{"channel":"stable"}"#).is_err());
    }

    #[test]
    fn signature_payload_is_canonical_and_excludes_signature() {
        let mut first = manifest();
        first.signature = "one".into();
        let mut second = manifest();
        second.signature = "two".into();

        let first_payload = first.signature_payload().expect("payload");
        assert_eq!(first_payload, second.signature_payload().expect("payload"));
        let text = String::from_utf8(first_payload.clone()).expect("utf8");
        assert!(!text.contains("signature"), "{text}");
        assert!(!text.contains(' '), "payload must not contain whitespace");
        assert!(
            text.starts_with("{\"artifacts\":["),
            "keys must be sorted: {text}"
        );

        // Changing any signed field changes the payload.
        let mut changed = manifest();
        changed.artifacts[0].sha256 = "other".into();
        assert_ne!(changed.signature_payload().expect("payload"), first_payload);
    }
}
