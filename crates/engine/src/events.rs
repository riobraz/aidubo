//! Parsing for the server's SSE event stream.
//!
//! OpenCode v2 frames every update as `data: {json}\n\n` with no `event:` line;
//! the discriminant lives in the JSON `type` field. We only model the events
//! the transcript needs for M2/M3 and ignore the rest, so a richer server cannot
//! break rendering.

use harness_protocol::{
    ExecutionOutcome, FormInfo, ModelRef, PermissionReply, PermissionRequest, StreamEvent,
    StructuredError, ToolContent,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct WireEvent {
    #[serde(default)]
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    created: i64,
    #[serde(default)]
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct TextStartedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    ordinal: u64,
}

#[derive(Debug, Deserialize)]
struct TextDeltaData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    ordinal: u64,
    #[serde(default)]
    delta: String,
}

#[derive(Debug, Deserialize)]
struct TextEndedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    ordinal: u64,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct SessionData {
    #[serde(rename = "sessionID")]
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct StepStartedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    #[serde(default)]
    started: i64,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    model: Option<ModelRef>,
}

#[derive(Debug, Deserialize)]
struct FormCreatedData {
    form: FormInfo,
}

#[derive(Debug, Deserialize)]
struct ExecutionFailedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(default)]
    error: Option<FailureError>,
}

#[derive(Debug, Deserialize)]
struct ExecutionInterruptedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FailureError {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct ReasoningOrdinalData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    ordinal: u64,
}

#[derive(Debug, Deserialize)]
struct ReasoningDeltaData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    ordinal: u64,
    #[serde(default)]
    delta: String,
}

#[derive(Debug, Deserialize)]
struct ReasoningEndedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    ordinal: u64,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct ToolEventData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    id: String,
}

#[derive(Debug, Deserialize)]
struct ToolInputStartedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ToolInputDeltaData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    id: String,
    #[serde(default)]
    delta: String,
}

#[derive(Debug, Deserialize)]
struct ToolInputEndedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    id: String,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct ToolCalledData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    id: String,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(default)]
    executed: bool,
}

#[derive(Debug, Deserialize)]
struct ToolSucceededData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    id: String,
    #[serde(default)]
    content: Vec<ToolContent>,
}

#[derive(Debug, Deserialize)]
struct ToolFailedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "assistantMessageID")]
    assistant_message_id: String,
    id: String,
    error: StructuredError,
    #[serde(default)]
    content: Vec<ToolContent>,
}

#[derive(Debug, Deserialize)]
struct PermissionRepliedData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(rename = "requestID")]
    request_id: String,
    reply: PermissionReply,
}

/// The tool argument payload as text; objects become compact JSON.
fn input_text(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text,
        other => serde_json::to_string(&other).unwrap_or_default(),
    }
}

/// Parse one SSE `data:` payload. Unknown or malformed frames yield `None`.
pub(crate) fn parse_event(json: &str) -> Option<StreamEvent> {
    let event: WireEvent = serde_json::from_str(json).ok()?;
    let created = event.created;
    let event_id = if event.id.is_empty() {
        format!("created:{created}")
    } else {
        event.id.clone()
    };
    let data = event.data;

    Some(match event.kind.as_str() {
        "session.execution.started" => {
            let data: SessionData = serde_json::from_value(data).ok()?;
            StreamEvent::ExecutionStarted {
                session_id: data.session_id,
            }
        }
        "session.execution.succeeded" => {
            let data: SessionData = serde_json::from_value(data).ok()?;
            StreamEvent::ExecutionFinished {
                session_id: data.session_id,
                event_id,
                outcome: ExecutionOutcome::Succeeded,
            }
        }
        "session.execution.failed" => {
            let data: ExecutionFailedData = serde_json::from_value(data).ok()?;
            let message = data
                .error
                .filter(|error| !error.message.is_empty())
                .map(|error| error.message)
                .unwrap_or_else(|| "The agent turn failed.".to_string());
            StreamEvent::ExecutionFinished {
                session_id: data.session_id,
                event_id,
                outcome: ExecutionOutcome::Failed { message },
            }
        }
        "session.execution.interrupted" => {
            let data: ExecutionInterruptedData = serde_json::from_value(data).ok()?;
            let _ = data.reason;
            StreamEvent::ExecutionFinished {
                session_id: data.session_id,
                event_id,
                outcome: ExecutionOutcome::Interrupted,
            }
        }
        "form.created" => {
            let data: FormCreatedData = serde_json::from_value(data).ok()?;
            StreamEvent::FormCreated { form: data.form }
        }
        "session.compaction.started" => {
            let data: SessionData = serde_json::from_value(data).ok()?;
            StreamEvent::CompactionStarted {
                session_id: data.session_id,
            }
        }
        "session.compaction.ended" => {
            let data: SessionData = serde_json::from_value(data).ok()?;
            StreamEvent::CompactionEnded {
                session_id: data.session_id,
            }
        }
        "session.compaction.failed" => {
            let data: ExecutionFailedData = serde_json::from_value(data).ok()?;
            let message = data
                .error
                .filter(|error| !error.message.is_empty())
                .map(|error| error.message)
                .unwrap_or_else(|| "Compaction failed.".to_string());
            StreamEvent::CompactionFailed {
                session_id: data.session_id,
                message,
            }
        }
        "filesystem.changed" => StreamEvent::FilesystemChanged,
        "session.usage.updated" | "session.usage.recorded" => {
            let data: UsageEventData = serde_json::from_value(data).ok()?;
            StreamEvent::UsageUpdated {
                session_id: data.session_id,
                tokens: data.tokens,
                cost: data.cost,
            }
        }
        "session.step.started" => {
            let data: StepStartedData = serde_json::from_value(data).ok()?;
            StreamEvent::AssistantStarted {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                started: data.started,
                agent: data.agent,
                model: data.model,
            }
        }
        "session.text.started" => {
            let data: TextStartedData = serde_json::from_value(data).ok()?;
            StreamEvent::TextStarted {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                ordinal: data.ordinal,
                created,
            }
        }
        "session.text.delta" => {
            let data: TextDeltaData = serde_json::from_value(data).ok()?;
            StreamEvent::TextDelta {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                ordinal: data.ordinal,
                created,
                delta: data.delta,
            }
        }
        "session.text.ended" => {
            let data: TextEndedData = serde_json::from_value(data).ok()?;
            StreamEvent::TextEnded {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                ordinal: data.ordinal,
                created,
                text: data.text,
            }
        }
        "session.reasoning.started" => {
            let data: ReasoningOrdinalData = serde_json::from_value(data).ok()?;
            StreamEvent::ReasoningStarted {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                ordinal: data.ordinal,
                created,
            }
        }
        "session.reasoning.delta" => {
            let data: ReasoningDeltaData = serde_json::from_value(data).ok()?;
            StreamEvent::ReasoningDelta {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                ordinal: data.ordinal,
                created,
                delta: data.delta,
            }
        }
        "session.reasoning.ended" => {
            let data: ReasoningEndedData = serde_json::from_value(data).ok()?;
            StreamEvent::ReasoningEnded {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                ordinal: data.ordinal,
                created,
                text: data.text,
            }
        }
        "session.tool.input.started" => {
            let data: ToolInputStartedData = serde_json::from_value(data).ok()?;
            StreamEvent::ToolInputStarted {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                tool_id: data.id,
                name: data.name,
            }
        }
        "session.tool.input.delta" => {
            let data: ToolInputDeltaData = serde_json::from_value(data).ok()?;
            StreamEvent::ToolInputDelta {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                tool_id: data.id,
                delta: data.delta,
            }
        }
        "session.tool.input.ended" => {
            let data: ToolInputEndedData = serde_json::from_value(data).ok()?;
            StreamEvent::ToolInputEnded {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                tool_id: data.id,
                input: data.text,
            }
        }
        "session.tool.called" => {
            let data: ToolCalledData = serde_json::from_value(data).ok()?;
            StreamEvent::ToolCalled {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                tool_id: data.id,
                input: input_text(data.input),
                executed: data.executed,
            }
        }
        "session.tool.progress" => {
            let data: ToolEventData = serde_json::from_value(data).ok()?;
            StreamEvent::ToolProgress {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                tool_id: data.id,
            }
        }
        "session.tool.success" => {
            let data: ToolSucceededData = serde_json::from_value(data).ok()?;
            StreamEvent::ToolSucceeded {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                tool_id: data.id,
                content: data.content,
            }
        }
        "session.tool.failed" => {
            let data: ToolFailedData = serde_json::from_value(data).ok()?;
            StreamEvent::ToolFailed {
                session_id: data.session_id,
                assistant_message_id: data.assistant_message_id,
                tool_id: data.id,
                error: data.error,
                content: data.content,
            }
        }
        "permission.asked" => {
            let request: PermissionRequest = serde_json::from_value(data).ok()?;
            StreamEvent::PermissionAsked { request }
        }
        "permission.replied" => {
            let data: PermissionRepliedData = serde_json::from_value(data).ok()?;
            StreamEvent::PermissionReplied {
                session_id: data.session_id,
                request_id: data.request_id,
                reply: data.reply,
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_delta() {
        let json = r#"{"id":"evt_1","created":7,"type":"session.text.delta",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","ordinal":0,"delta":"He"}}"#;
        assert_eq!(
            parse_event(json),
            Some(StreamEvent::TextDelta {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                ordinal: 0,
                created: 7,
                delta: "He".into(),
            })
        );
    }

    #[test]
    fn parses_execution_outcomes() {
        let started = r#"{"type":"session.execution.started","data":{"sessionID":"ses_1"}}"#;
        assert_eq!(
            parse_event(started),
            Some(StreamEvent::ExecutionStarted {
                session_id: "ses_1".into()
            })
        );

        let failed = r#"{"id":"evt_fail","created":9,"type":"session.execution.failed",
            "data":{"sessionID":"ses_1",
            "error":{"type":"provider.quota","message":"No credits","status":402}}}"#;
        assert_eq!(
            parse_event(failed),
            Some(StreamEvent::ExecutionFinished {
                session_id: "ses_1".into(),
                event_id: "evt_fail".into(),
                outcome: ExecutionOutcome::Failed {
                    message: "No credits".into()
                },
            })
        );

        let interrupted = r#"{"id":"evt_int","type":"session.execution.interrupted",
            "data":{"sessionID":"ses_1","reason":"user"}}"#;
        assert_eq!(
            parse_event(interrupted),
            Some(StreamEvent::ExecutionFinished {
                session_id: "ses_1".into(),
                event_id: "evt_int".into(),
                outcome: ExecutionOutcome::Interrupted,
            })
        );

        // Older frames without an id still yield a replay-stable fallback.
        let succeeded = r#"{"created":12,"type":"session.execution.succeeded",
            "data":{"sessionID":"ses_1"}}"#;
        assert_eq!(
            parse_event(succeeded),
            Some(StreamEvent::ExecutionFinished {
                session_id: "ses_1".into(),
                event_id: "created:12".into(),
                outcome: ExecutionOutcome::Succeeded,
            })
        );
    }

    #[test]
    fn parses_form_created() {
        let json = r#"{"id":"evt_form","type":"form.created","data":{"form":{
            "id":"form_1","sessionID":"ses_1","title":"Pick one",
            "fields":[{"key":"choice","type":"string"}]}}}"#;
        assert_eq!(
            parse_event(json),
            Some(StreamEvent::FormCreated {
                form: FormInfo {
                    id: "form_1".into(),
                    session_id: "ses_1".into(),
                    title: "Pick one".into(),
                    fields: vec![harness_protocol::FormField {
                        key: "choice".into(),
                        kind: "string".into(),
                        title: None,
                        description: None,
                        required: false,
                        hidden: false,
                        options: Vec::new(),
                        default: None,
                    }],
                },
            })
        );
    }

    #[test]
    fn parses_step_started() {
        let json = r#"{"type":"session.step.started","data":{"sessionID":"ses_1",
            "agent":"build","model":{"id":"m","providerID":"p"},
            "assistantMessageID":"msg_2","started":99}}"#;
        assert_eq!(
            parse_event(json),
            Some(StreamEvent::AssistantStarted {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                started: 99,
                agent: Some("build".into()),
                model: Some(ModelRef {
                    id: "m".into(),
                    provider_id: Some("p".into()),
                    variant: None,
                }),
            })
        );
    }

    #[test]
    fn parses_reasoning_updates() {
        let started = r#"{"id":"evt_1","created":5,"type":"session.reasoning.started",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","ordinal":0,
                    "native":{"reasoningField":"reasoning_content"}}}"#;
        assert_eq!(
            parse_event(started),
            Some(StreamEvent::ReasoningStarted {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                ordinal: 0,
                created: 5,
            })
        );

        let delta = r#"{"id":"evt_2","created":6,"type":"session.reasoning.delta",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","ordinal":0,
                    "delta":"Let me read the file first."}}"#;
        assert_eq!(
            parse_event(delta),
            Some(StreamEvent::ReasoningDelta {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                ordinal: 0,
                created: 6,
                delta: "Let me read the file first.".into(),
            })
        );

        let ended = r#"{"id":"evt_3","created":7,"type":"session.reasoning.ended",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","ordinal":0,
                    "text":"Let me read the file first."}}"#;
        assert_eq!(
            parse_event(ended),
            Some(StreamEvent::ReasoningEnded {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                ordinal: 0,
                created: 7,
                text: "Let me read the file first.".into(),
            })
        );
    }

    #[test]
    fn parses_tool_lifecycle() {
        let started = r#"{"type":"session.tool.input.started",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2",
                    "id":"call_1","name":"read"}}"#;
        assert_eq!(
            parse_event(started),
            Some(StreamEvent::ToolInputStarted {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                tool_id: "call_1".into(),
                name: "read".into(),
            })
        );

        let delta = r#"{"type":"session.tool.input.delta",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2",
                    "id":"call_1","delta":"{\"path\":"}}"#;
        assert!(matches!(
            parse_event(delta),
            Some(StreamEvent::ToolInputDelta { delta, .. }) if delta == r#"{"path":"#
        ));

        let ended = r#"{"type":"session.tool.input.ended",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","id":"call_1",
                    "text":"{\"path\":\"/tmp/hello.txt\"}"}}"#;
        assert_eq!(
            parse_event(ended),
            Some(StreamEvent::ToolInputEnded {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                tool_id: "call_1".into(),
                input: r#"{"path":"/tmp/hello.txt"}"#.into(),
            })
        );

        // Verbatim payload captured from v2.2.6.
        let called = r#"{"type":"session.tool.called",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","id":"call_1",
                    "input":{"path":"/tmp/hello.txt"},"executed":false}}"#;
        assert_eq!(
            parse_event(called),
            Some(StreamEvent::ToolCalled {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                tool_id: "call_1".into(),
                input: r#"{"path":"/tmp/hello.txt"}"#.into(),
                executed: false,
            })
        );

        let progress = r#"{"type":"session.tool.progress",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","id":"call_1",
                    "metadata":{"truncated":false}}}"#;
        assert_eq!(
            parse_event(progress),
            Some(StreamEvent::ToolProgress {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                tool_id: "call_1".into(),
            })
        );

        let success = r#"{"type":"session.tool.success",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","id":"call_1",
                    "content":[{"type":"text","text":"hello"}],
                    "metadata":{"truncated":false},"executed":false}}"#;
        assert_eq!(
            parse_event(success),
            Some(StreamEvent::ToolSucceeded {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                tool_id: "call_1".into(),
                content: vec![ToolContent::Text {
                    text: "hello".into()
                }],
            })
        );

        let failed = r#"{"type":"session.tool.failed",
            "data":{"sessionID":"ses_1","assistantMessageID":"msg_2","id":"call_1",
                    "error":{"type":"tool.execution","message":"not found"},
                    "executed":false}}"#;
        assert_eq!(
            parse_event(failed),
            Some(StreamEvent::ToolFailed {
                session_id: "ses_1".into(),
                assistant_message_id: "msg_2".into(),
                tool_id: "call_1".into(),
                error: StructuredError {
                    kind: "tool.execution".into(),
                    message: "not found".into(),
                    status: None,
                },
                content: Vec::new(),
            })
        );
    }

    #[test]
    fn parses_permission_events() {
        // Verbatim payload captured from v2.2.6.
        let asked = r#"{"type":"permission.asked",
            "data":{"id":"per_1bcd6ee39001Pr1u4fmvsdkUHT","sessionID":"ses_1",
                    "action":"shell","resources":["echo hello-from-bash"],
                    "save":["echo *"],
                    "source":{"type":"tool","messageID":"msg_2","id":"call_bash"}}}"#;
        let Some(StreamEvent::PermissionAsked { request }) = parse_event(asked) else {
            panic!("expected a permission request");
        };
        assert_eq!(request.id, "per_1bcd6ee39001Pr1u4fmvsdkUHT");
        assert_eq!(request.action, "shell");
        assert_eq!(request.resources, ["echo hello-from-bash"]);

        let replied = r#"{"type":"permission.replied",
            "data":{"sessionID":"ses_1","requestID":"per_1","reply":"reject"}}"#;
        assert_eq!(
            parse_event(replied),
            Some(StreamEvent::PermissionReplied {
                session_id: "ses_1".into(),
                request_id: "per_1".into(),
                reply: PermissionReply::Reject,
            })
        );
    }

    #[test]
    fn parses_usage_updated() {
        let event = parse_event(
            r#"{"type":"session.usage.updated","data":{"sessionID":"s1","cost":0.5,"tokens":{"input":1,"output":2,"reasoning":0,"cache":{"read":0,"write":0}}}}"#,
        )
        .expect("usage event");
        let crate::StreamEvent::UsageUpdated {
            session_id,
            tokens,
            cost,
        } = event
        else {
            panic!("wrong variant");
        };
        assert_eq!(session_id, "s1");
        assert_eq!(tokens.input, 1.0);
        assert_eq!(cost, Some(0.5));
    }

    #[test]
    fn ignores_unknown_and_malformed_frames() {
        assert_eq!(
            parse_event(r#"{"type":"server.connected","data":{}}"#),
            None
        );
        assert_eq!(parse_event("not json"), None);
        assert_eq!(
            parse_event(r#"{"type":"session.text.delta","data":{}}"#),
            None
        );
    }
}

#[derive(Debug, serde::Deserialize)]
struct UsageEventData {
    #[serde(rename = "sessionID")]
    session_id: String,
    #[serde(default)]
    tokens: harness_protocol::TokenUsage,
    #[serde(default)]
    cost: Option<f64>,
}
