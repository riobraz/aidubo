//! Scheduled prompts.
//!
//! The sidecar owns the clock. These types only describe what it accepts and
//! returns, so unknown values stay readable instead of failing the parse.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A cadence accepted by `Schedule.CreatePayload.when`.
///
/// `info.when` carries an extra `type` discriminator; serde ignores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScheduleWhen {
    In {
        #[serde(rename = "in")]
        in_: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
    At {
        at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
    Every {
        every: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
    Cron {
        cron: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
    /// A form this client does not know yet.
    Other(serde_json::Value),
}

impl ScheduleWhen {
    pub fn in_(value: impl Into<String>) -> Self {
        Self::In {
            in_: value.into(),
            tz: None,
        }
    }

    pub fn at(value: impl Into<String>) -> Self {
        Self::At {
            at: value.into(),
            tz: None,
        }
    }

    pub fn every(value: impl Into<String>) -> Self {
        Self::Every {
            every: value.into(),
            tz: None,
        }
    }

    pub fn cron(value: impl Into<String>) -> Self {
        Self::Cron {
            cron: value.into(),
            tz: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleStatus {
    Active,
    Paused,
    Done,
    Unknown(String),
}

impl Default for ScheduleStatus {
    fn default() -> Self {
        Self::Unknown(String::new())
    }
}

impl ScheduleStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Unknown(value) => value,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done)
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Active => "Active",
            Self::Paused => "Paused",
            Self::Done => "Done",
            Self::Unknown(value) => value.as_str(),
        }
    }
}

impl Serialize for ScheduleStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ScheduleStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "active" => Self::Active,
            "paused" => Self::Paused,
            "done" => Self::Done,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleOutcome {
    Fired,
    Late,
    Checked,
    Expired,
    Skipped,
    Disabled,
    Failed,
    Unknown(String),
}

impl ScheduleOutcome {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Fired => "fired",
            Self::Late => "late",
            Self::Checked => "checked",
            Self::Expired => "expired",
            Self::Skipped => "skipped",
            Self::Disabled => "disabled",
            Self::Failed => "failed",
            Self::Unknown(value) => value,
        }
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed)
    }
}

impl Serialize for ScheduleOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ScheduleOutcome {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "fired" => Self::Fired,
            "late" => Self::Late,
            "checked" => Self::Checked,
            "expired" => Self::Expired,
            "skipped" => Self::Skipped,
            "disabled" => Self::Disabled,
            "failed" => Self::Failed,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct ScheduleTime {
    #[serde(default)]
    pub created: f64,
    #[serde(default)]
    pub updated: f64,
}

/// `Schedule.Info` as returned by the server.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ScheduleInfo {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "sessionID", default)]
    pub session_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub when: Option<ScheduleWhen>,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub status: ScheduleStatus,
    #[serde(rename = "nextDueAt", default)]
    pub next_due_at: Option<f64>,
    #[serde(rename = "endsAt", default)]
    pub ends_at: Option<f64>,
    #[serde(rename = "remainingRuns", default)]
    pub remaining_runs: Option<i64>,
    #[serde(rename = "runCount", default)]
    pub run_count: i64,
    #[serde(rename = "lastFiredAt", default)]
    pub last_fired_at: Option<f64>,
    #[serde(rename = "lastOutcome", default)]
    pub last_outcome: Option<ScheduleOutcome>,
    #[serde(rename = "lastOutput", default)]
    pub last_output: Option<String>,
    #[serde(default)]
    pub time: ScheduleTime,
}

impl ScheduleInfo {
    pub fn is_active(&self) -> bool {
        self.status.is_active()
    }

    pub fn is_done(&self) -> bool {
        self.status.is_done()
    }
}

/// `Schedule.CreatePayload`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CreateScheduleRequest {
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub when: ScheduleWhen,
    #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
    pub if_condition: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends: Option<serde_json::Value>,
    pub prompt: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_parses_every_when_form() {
        for (json, expected) in [
            (r#"{"in":"2 seconds"}"#, ScheduleWhen::in_("2 seconds")),
            (
                r#"{"at":"2030-01-01T00:00:00Z"}"#,
                ScheduleWhen::at("2030-01-01T00:00:00Z"),
            ),
            (r#"{"every":"5 minutes"}"#, ScheduleWhen::every("5 minutes")),
            (r#"{"cron":"0 * * * *"}"#, ScheduleWhen::cron("0 * * * *")),
        ] {
            let parsed: ScheduleWhen = serde_json::from_str(json).expect("when");
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn schedule_when_serializes_in_keyword() {
        let value = serde_json::to_value(ScheduleWhen::in_("90 seconds")).expect("json");
        assert_eq!(value, serde_json::json!({ "in": "90 seconds" }));
    }

    #[test]
    fn schedule_parses_info_with_type_discriminator() {
        let info: ScheduleInfo = serde_json::from_str(
            r#"{
                "id":"sch_1","sessionID":"ses_1","name":"nightly",
                "when":{"type":"every","every":"1 hour","tz":"UTC"},
                "tz":"UTC","status":"active","nextDueAt":10.0,"endsAt":20.0,
                "remainingRuns":2,"runCount":3,"lastFiredAt":9.0,
                "lastOutcome":"fired","lastOutput":"",
                "time":{"created":1.0,"updated":2.0}
            }"#,
        )
        .expect("info");
        assert_eq!(
            info.when,
            Some(ScheduleWhen::Every {
                every: "1 hour".into(),
                tz: Some("UTC".into()),
            })
        );
        assert_eq!(info.status, ScheduleStatus::Active);
        assert_eq!(info.last_output.as_deref(), Some(""));
        assert_eq!(info.time.created, 1.0);
    }

    #[test]
    fn schedule_unknown_status_is_kept() {
        let info: ScheduleInfo =
            serde_json::from_str(r#"{"id":"x","status":"archived","runCount":2}"#).expect("info");
        assert_eq!(info.status, ScheduleStatus::Unknown("archived".into()));
        assert_eq!(info.run_count, 2);
    }

    #[test]
    fn schedule_unknown_when_is_kept() {
        let info: ScheduleInfo =
            serde_json::from_str(r#"{"id":"x","when":{"solar":"noon"}}"#).expect("info");
        assert!(matches!(info.when, Some(ScheduleWhen::Other(_))));
    }

    #[test]
    fn schedule_create_omits_absent_optionals() {
        let request = CreateScheduleRequest {
            session_id: "ses_1".into(),
            name: None,
            when: ScheduleWhen::in_("2 seconds"),
            if_condition: None,
            until: None,
            ends: None,
            prompt: "hello".into(),
        };
        let value = serde_json::to_value(&request).expect("json");
        assert_eq!(
            value,
            serde_json::json!({
                "sessionID": "ses_1",
                "when": { "in": "2 seconds" },
                "prompt": "hello"
            })
        );
    }
}
