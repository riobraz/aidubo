use super::*;
use harness_protocol::{
    AssistantPart, ExecutionOutcome, ModelRef, PermissionRequest, Session, SessionMessage,
    SessionTime, StreamEvent, StructuredError, ToolContent, ToolState,
};

fn session(id: &str, updated: i64) -> Session {
    Session {
        id: id.into(),
        title: Some(format!("Session {id}")),
        project_id: Some("p1".into()),
        agent: None,
        model: None,
        time: SessionTime {
            created: updated,
            updated,
        },
        location: None,
    }
}

#[test]
fn defaults_to_starting() {
    assert_eq!(
        EngineStatus::default(),
        EngineStatus::Starting { port: None }
    );
}

#[test]
fn labels_match_state() {
    let ready = EngineStatus::Ready {
        version: "v2.1.4".into(),
        port: 4096,
    };
    assert_eq!(ready.label(), "Connected");
    assert!(ready.is_ready());
    assert!(!ready.is_failed());

    let failed = EngineStatus::Failed {
        message: "boom".into(),
    };
    assert_eq!(failed.label(), "Engine failed");
    assert!(failed.is_failed());
}

#[test]
fn replace_sorts_newest_first_and_selects_first() {
    let mut store = SessionStore::default();
    store.replace(vec![
        session("old", 10),
        session("new", 30),
        session("mid", 20),
    ]);

    let ids: Vec<_> = store.sessions().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["new", "mid", "old"]);
    assert_eq!(store.selected_id(), Some("new"));
}

#[test]
fn replace_keeps_a_live_selection() {
    let mut store = SessionStore::default();
    store.replace(vec![session("a", 10), session("b", 20)]);
    store.select("a");
    store.replace(vec![session("b", 30), session("a", 10)]);
    assert_eq!(store.selected_id(), Some("a"));
}

#[test]
fn replace_drops_a_selection_that_is_gone() {
    let mut store = SessionStore::default();
    store.replace(vec![session("a", 10), session("b", 20)]);
    store.select("a");
    store.replace(vec![session("b", 20)]);
    assert_eq!(store.selected_id(), Some("b"));
}

#[test]
fn upsert_inserts_and_sorts() {
    let mut store = SessionStore::default();
    store.replace(vec![session("a", 10)]);
    store.upsert(session("b", 20));
    let ids: Vec<_> = store.sessions().iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["b", "a"]);
}

#[test]
fn upsert_replaces_without_duplicating() {
    let mut store = SessionStore::default();
    store.replace(vec![session("a", 10)]);
    let mut updated = session("a", 10);
    updated.title = Some("Renamed".into());
    store.upsert(updated);
    assert_eq!(store.len(), 1);
    assert_eq!(store.sessions()[0].title.as_deref(), Some("Renamed"));
}

#[test]
fn remove_selected_falls_back_to_the_first_remaining() {
    let mut store = SessionStore::default();
    store.replace(vec![session("a", 10), session("b", 20)]);
    store.select("b");
    store.remove("b");
    assert_eq!(store.selected_id(), Some("a"));

    store.remove("a");
    assert_eq!(store.selected_id(), None);
    assert!(store.is_empty());
}

#[test]
fn select_ignores_unknown_ids() {
    let mut store = SessionStore::default();
    store.replace(vec![session("a", 10)]);
    store.select("missing");
    assert_eq!(store.selected_id(), Some("a"));
}

#[test]
fn rename_updates_in_place() {
    let mut store = SessionStore::default();
    store.replace(vec![session("a", 10)]);
    store.rename("a", "New title");
    assert_eq!(store.sessions()[0].title.as_deref(), Some("New title"));
}

fn user_message(id: &str, text: &str, created: i64) -> SessionMessage {
    SessionMessage::User {
        id: id.into(),
        time: harness_protocol::MessageTime {
            created,
            streamed: None,
            completed: None,
        },
        text: text.into(),
    }
}

fn assistant_message(id: &str, created: i64, content: Vec<AssistantPart>) -> SessionMessage {
    SessionMessage::Assistant {
        id: id.into(),
        time: harness_protocol::MessageTime {
            created,
            streamed: None,
            completed: Some(created + 10),
        },
        agent: Some("build".into()),
        model: Some(ModelRef {
            id: "sonnet".into(),
            provider_id: Some("anthropic".into()),
            variant: None,
        }),
        content,
        finish: Some("stop".into()),
        error: None,
    }
}

fn delta(id: &str, ordinal: u64, created: i64, text: &str) -> StreamEvent {
    StreamEvent::TextDelta {
        session_id: "ses_1".into(),
        assistant_message_id: id.into(),
        ordinal,
        created,
        delta: text.into(),
    }
}

#[test]
fn snapshot_orders_messages_and_maps_parts() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");
    assert!(store.is_loading());

    let applied = store.apply_snapshot(
        "ses_1",
        vec![
            assistant_message(
                "msg_2",
                20,
                vec![
                    AssistantPart::Text { text: "Hi".into() },
                    AssistantPart::Reasoning {
                        text: "thinking".into(),
                    },
                    AssistantPart::Tool {
                        id: "t1".into(),
                        name: "bash".into(),
                        executed: true,
                        state: ToolState::Completed {
                            input: r#"{"cmd":"ls"}"#.into(),
                            content: vec![ToolContent::Text { text: "ok".into() }],
                        },
                    },
                ],
            ),
            user_message("msg_1", "hello", 10),
            SessionMessage::Idle {
                id: "msg_3".into(),
                time: harness_protocol::MessageTime {
                    created: 30,
                    streamed: None,
                    completed: None,
                },
                outcome: Some("succeeded".into()),
                error: None,
            },
        ],
    );

    assert!(applied);
    assert!(!store.is_loading());
    let ids: Vec<_> = store.entries().iter().map(Entry::id).collect();
    assert_eq!(ids, ["msg_1", "msg_2"]);

    let Entry::Assistant(assistant) = &store.entries()[1] else {
        panic!("expected an assistant entry");
    };
    assert_eq!(assistant.agent.as_deref(), Some("build"));
    assert_eq!(assistant.model.as_deref(), Some("anthropic/sonnet"));
    assert_eq!(assistant.parts.len(), 3);
    let Content::Tool(tool) = &assistant.parts[2] else {
        panic!("expected a tool part");
    };
    assert_eq!(tool.name, "bash");
    assert_eq!(tool.status, ToolStatus::Completed);
    assert_eq!(tool.input, r#"{"cmd":"ls"}"#);
    assert_eq!(tool.output.as_deref(), Some("ok"));
    assert_eq!(tool.error, None);
}

#[test]
fn reasoning_streams_and_settles() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");

    store.apply_stream(StreamEvent::ReasoningStarted {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        ordinal: 0,
        created: 10,
    });
    store.apply_stream(StreamEvent::ReasoningDelta {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        ordinal: 0,
        created: 10,
        delta: "Let me ".into(),
    });
    store.apply_stream(StreamEvent::ReasoningDelta {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        ordinal: 0,
        created: 10,
        delta: "think.".into(),
    });

    let Entry::Assistant(assistant) = &store.entries()[0] else {
        panic!("expected an assistant entry");
    };
    assert!(matches!(
        &assistant.parts[0],
        Content::Reasoning { text, .. } if text == "Let me think."
    ));

    // The ended frame is authoritative and does not duplicate the part.
    store.apply_stream(StreamEvent::ReasoningEnded {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        ordinal: 0,
        created: 10,
        text: "Let me think carefully.".into(),
    });
    let Entry::Assistant(assistant) = &store.entries()[0] else {
        panic!("expected an assistant entry");
    };
    assert_eq!(assistant.parts.len(), 1);
    assert!(matches!(
        &assistant.parts[0],
        Content::Reasoning { text, .. } if text == "Let me think carefully."
    ));
}

#[test]
fn tool_lifecycle_updates_one_card_in_place() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");

    store.apply_stream(StreamEvent::ToolInputStarted {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        tool_id: "call_1".into(),
        name: "read".into(),
    });
    store.apply_stream(StreamEvent::ToolInputDelta {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        tool_id: "call_1".into(),
        delta: r#"{"path":"/tmp/a"#.into(),
    });

    let tool = tool_of(&store, "call_1");
    assert_eq!(tool.name, "read");
    assert_eq!(tool.status, ToolStatus::Streaming);
    assert_eq!(tool.input, r#"{"path":"/tmp/a"#);

    store.apply_stream(StreamEvent::ToolInputEnded {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        tool_id: "call_1".into(),
        input: r#"{"path":"/tmp/a"}"#.into(),
    });
    store.apply_stream(StreamEvent::ToolCalled {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        tool_id: "call_1".into(),
        input: r#"{"path":"/tmp/a"}"#.into(),
        executed: false,
    });
    assert_eq!(tool_of(&store, "call_1").status, ToolStatus::Running);

    store.apply_stream(StreamEvent::ToolSucceeded {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        tool_id: "call_1".into(),
        content: vec![ToolContent::Text {
            text: "hello".into(),
        }],
    });
    let tool = tool_of(&store, "call_1");
    assert_eq!(tool.status, ToolStatus::Completed);
    assert_eq!(tool.output.as_deref(), Some("hello"));
    assert_eq!(tool.error, None);
    // Every event updated the same card.
    assert_eq!(assistant_of(&store).parts.len(), 1);
}

#[test]
fn failed_tool_keeps_its_message() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");

    store.apply_stream(StreamEvent::ToolFailed {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        tool_id: "call_1".into(),
        error: StructuredError {
            kind: "tool.execution".into(),
            message: "not found".into(),
            status: None,
        },
        content: vec![ToolContent::Text {
            text: "details".into(),
        }],
    });

    let tool = tool_of(&store, "call_1");
    assert_eq!(tool.status, ToolStatus::Error);
    assert_eq!(tool.error.as_deref(), Some("not found"));
    assert_eq!(tool.output.as_deref(), Some("details"));
}

#[test]
fn tool_events_for_other_sessions_are_ignored() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");
    assert!(!store.apply_stream(StreamEvent::ToolInputStarted {
        session_id: "ses_2".into(),
        assistant_message_id: "msg_1".into(),
        tool_id: "call_1".into(),
        name: "read".into(),
    }));
    assert!(store.entries().is_empty());
}

fn assistant_of(store: &TranscriptStore) -> &AssistantEntry {
    let Entry::Assistant(assistant) = &store.entries()[0] else {
        panic!("expected an assistant entry");
    };
    assistant
}

fn tool_of<'a>(store: &'a TranscriptStore, id: &str) -> &'a ToolCall {
    assistant_of(store)
        .parts
        .iter()
        .find_map(|part| match part {
            Content::Tool(tool) if tool.id == id => Some(tool),
            _ => None,
        })
        .expect("expected a tool part")
}

#[test]
fn intentional_interrupts_do_not_read_as_failures() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");

    let mut aborted = assistant_message(
        "msg_2",
        20,
        vec![AssistantPart::Text {
            text: "working".into(),
        }],
    );
    let mut failed = assistant_message("msg_3", 30, vec![]);
    if let SessionMessage::Assistant { error, .. } = &mut aborted {
        *error = Some(StructuredError {
            kind: "aborted".into(),
            message: "Step interrupted".into(),
            status: None,
        });
    }
    if let SessionMessage::Assistant { error, .. } = &mut failed {
        *error = Some(StructuredError {
            kind: "provider.quota".into(),
            message: "No credits".into(),
            status: Some(402),
        });
    }

    assert!(store.apply_snapshot("ses_1", vec![aborted, failed]));

    let Entry::Assistant(aborted) = &store.entries()[0] else {
        panic!("expected an assistant entry");
    };
    assert_eq!(aborted.error, None);
    assert!(matches!(
        &aborted.parts[0],
        Content::Text { text, .. } if text == "working"
    ));

    let Entry::Assistant(failed) = &store.entries()[1] else {
        panic!("expected a second assistant entry");
    };
    assert_eq!(failed.error.as_deref(), Some("No credits"));
}

#[test]
fn stale_snapshots_are_ignored() {
    let mut store = TranscriptStore::default();
    store.begin("ses_2");
    assert!(!store.apply_snapshot("ses_1", vec![user_message("msg_1", "old", 1)]));
    assert!(store.entries().is_empty());
}

#[test]
fn streaming_deltas_accumulate_and_final_text_replaces() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");

    assert!(store.apply_stream(StreamEvent::ExecutionStarted {
        session_id: "ses_1".into()
    }));
    assert!(store.is_busy());

    store.apply_stream(StreamEvent::AssistantStarted {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_2".into(),
        started: 20,
        agent: Some("build".into()),
        model: None,
    });
    store.apply_stream(delta("msg_2", 0, 20, "Hel"));
    store.apply_stream(delta("msg_2", 0, 20, "lo"));
    store.apply_stream(StreamEvent::TextEnded {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_2".into(),
        ordinal: 0,
        created: 20,
        text: "Hello there".into(),
    });

    let Entry::Assistant(assistant) = &store.entries()[0] else {
        panic!("expected an assistant entry");
    };
    assert_eq!(
        assistant.parts,
        vec![Content::Text {
            ordinal: 0,
            text: "Hello there".into(),
        }]
    );

    assert!(store.apply_stream(StreamEvent::ExecutionFinished {
        session_id: "ses_1".into(),
        outcome: ExecutionOutcome::Succeeded,
    }));
    assert!(!store.is_busy());
    assert_eq!(store.error(), None);
}

#[test]
fn failed_turns_surface_their_message() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");
    store.apply_stream(StreamEvent::ExecutionStarted {
        session_id: "ses_1".into(),
    });
    store.apply_stream(StreamEvent::ExecutionFinished {
        session_id: "ses_1".into(),
        outcome: ExecutionOutcome::Failed {
            message: "No credits".into(),
        },
    });
    assert!(!store.is_busy());
    assert_eq!(store.error(), Some("No credits"));
    store.clear_error();
    assert_eq!(store.error(), None);
}

#[test]
fn events_for_other_sessions_are_ignored() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");
    assert!(!store.apply_stream(StreamEvent::ExecutionStarted {
        session_id: "ses_2".into()
    }));
    assert!(!store.is_busy());
    assert!(store.entries().is_empty());
}

#[test]
fn accept_prompt_upserts_the_user_message() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");
    assert!(store.accept_prompt("ses_1", "msg_1", "hello", 10));
    assert!(store.accept_prompt("ses_1", "msg_1", "hello again", 11));
    assert_eq!(store.entries().len(), 1);
    assert!(matches!(
        &store.entries()[0],
        Entry::User { text, created, .. } if text == "hello again" && *created == 11
    ));
    assert!(!store.accept_prompt("ses_2", "msg_9", "no", 1));
}

fn permission(id: &str, session: &str) -> PermissionRequest {
    PermissionRequest {
        id: id.into(),
        session_id: session.into(),
        action: "shell".into(),
        resources: vec![format!("echo {id}")],
        save: Vec::new(),
        message: None,
        source: None,
    }
}

#[test]
fn permission_queue_is_session_scoped_and_deduped() {
    let mut queue = PermissionQueue::default();
    queue.begin("ses_1");
    assert!(queue.push(permission("per_1", "ses_1")));
    assert!(!queue.push(permission("per_1", "ses_1")));
    assert!(queue.push(permission("per_2", "ses_1")));
    assert!(!queue.push(permission("per_3", "ses_2")));
    assert_eq!(queue.len(), 2);
    assert_eq!(
        queue.front().map(|request| request.id.as_str()),
        Some("per_1")
    );

    assert!(queue.remove("per_1"));
    assert!(!queue.remove("per_1"));
    assert_eq!(
        queue.front().map(|request| request.id.as_str()),
        Some("per_2")
    );

    // A snapshot replaces the queue; one for another session is stale.
    assert!(queue.replace("ses_1", vec![permission("per_4", "ses_1")]));
    assert_eq!(
        queue.front().map(|request| request.id.as_str()),
        Some("per_4")
    );
    assert!(!queue.replace("ses_2", Vec::new()));
    assert_eq!(queue.len(), 1);

    // Changing the selection clears the queue.
    queue.begin("ses_2");
    assert!(queue.is_empty());
    assert!(!queue.push(permission("per_5", "ses_1")));
    assert!(queue.push(permission("per_6", "ses_2")));
}

#[test]
fn declined_tool_failure_keeps_its_message() {
    let mut store = TranscriptStore::default();
    store.begin("ses_1");
    store.apply_stream(StreamEvent::ToolFailed {
        session_id: "ses_1".into(),
        assistant_message_id: "msg_1".into(),
        tool_id: "call_1".into(),
        error: StructuredError {
            kind: "aborted".into(),
            message: "The user declined this tool call".into(),
            status: None,
        },
        content: Vec::new(),
    });

    let tool = tool_of(&store, "call_1");
    assert_eq!(tool.status, ToolStatus::Error);
    assert_eq!(
        tool.error.as_deref(),
        Some("The user declined this tool call")
    );
}

fn command_item(name: &str) -> PickerItem {
    PickerItem::Command {
        name: name.into(),
        description: String::new(),
    }
}

#[test]
fn drafts_find_triggers_at_the_cursor() {
    let mut draft = PromptDraft::default();
    draft.edit("@src/ma", 7, true);
    let trigger = draft.trigger().unwrap();
    assert_eq!(trigger.kind, TriggerKind::File);
    assert_eq!(trigger.start, 0);
    assert_eq!(trigger.query, "src/ma");

    // A sigil inside a word is just text.
    draft.edit("mail@example.com", 16, true);
    assert!(draft.trigger().is_none());

    // Commands only open at the start of the input.
    draft.edit("please /rev", 10, true);
    assert!(draft.trigger().is_none());
    draft.edit("  /rev", 6, true);
    let trigger = draft.trigger().unwrap();
    assert_eq!(trigger.kind, TriggerKind::Command);
    assert_eq!(trigger.query, "rev");

    // Skills and snippets work after whitespace.
    draft.edit("use $open", 9, true);
    assert_eq!(draft.trigger().unwrap().kind, TriggerKind::Skill);
    draft.edit("use #rev", 8, true);
    assert_eq!(draft.trigger().unwrap().kind, TriggerKind::Snippet);

    // A selection is not a trigger.
    draft.edit("@re", 3, false);
    assert!(draft.trigger().is_none());
}

#[test]
fn selecting_a_file_inserts_a_mention_and_wire_range() {
    let mut draft = PromptDraft::default();
    draft.edit("@src/ma", 7, true);
    let edit = draft
        .select_file("src/main.rs", path_to_file_uri("/tmp/project/src/main.rs"))
        .unwrap();
    assert_eq!(edit.text, "@src/main.rs");
    assert_eq!(edit.cursor, "@src/main.rs".len());
    assert!(draft.trigger().is_none());

    let DraftSend::Prompt(request) = draft.to_send().unwrap() else {
        panic!("expected a prompt");
    };
    assert_eq!(request.files.len(), 1);
    let mention = request.files[0].mention.as_ref().unwrap();
    assert_eq!(mention.start, 0);
    assert_eq!(mention.end, 12);
    assert_eq!(mention.text, "src/main.rs");
    assert_eq!(request.files[0].uri, "file:///tmp/project/src/main.rs");
    assert_eq!(
        request.files[0].source,
        Some(harness_protocol::FileSource::Uri {
            uri: "file:///tmp/project/src/main.rs".into()
        })
    );
}

#[test]
fn mention_offsets_are_utf16_and_survive_edits_before_them() {
    let mut draft = PromptDraft::default();
    draft.edit("@src/main.rs", 12, true);
    draft
        .select_file("src/main.rs", path_to_file_uri("/tmp/project/src/main.rs"))
        .unwrap();

    // Insert emoji before the mention: byte offsets shift by 4, UTF-16 by 2.
    let text = "😀 @src/main.rs x";
    draft.edit(text, text.len(), true);
    assert_eq!(draft.files().len(), 1);
    let DraftSend::Prompt(request) = draft.to_send().unwrap() else {
        panic!("expected a prompt");
    };
    let mention = request.files[0].mention.as_ref().unwrap();
    assert_eq!((mention.start, mention.end), (3, 15));
    assert_eq!(&text[5..17], "@src/main.rs");

    // Deleting the token drops the attachment instead of sending stale offsets.
    draft.edit("@main.rs", 8, true);
    assert!(draft.files().is_empty());
    let DraftSend::Prompt(request) = draft.to_send().unwrap() else {
        panic!("expected a prompt");
    };
    assert!(request.files.is_empty());
}

#[test]
fn removing_a_file_removes_its_mention() {
    let mut draft = PromptDraft::default();
    draft.edit("read @src/ma now", 12, true);
    draft
        .select_file("src/main.rs", path_to_file_uri("/tmp/project/src/main.rs"))
        .unwrap();
    assert_eq!(draft.text(), "read @src/main.rs now");
    assert_eq!(draft.files().len(), 1);

    let edit = draft.remove_file(0).unwrap();
    assert_eq!(edit.text, "read  now");
    assert!(draft.files().is_empty());
}

#[test]
fn agents_and_skills_become_chips_without_inline_text() {
    let mut draft = PromptDraft::default();
    draft.edit("@", 1, true);
    draft.select_agent("build").unwrap();
    assert_eq!(draft.text(), "");
    assert_eq!(draft.agents().len(), 1);

    draft.edit("$", 1, true);
    draft.select_skill("opencode", "OpenCode").unwrap();
    assert_eq!(draft.text(), "");
    assert_eq!(draft.skills().len(), 1);

    let DraftSend::Prompt(request) = draft.to_send().unwrap() else {
        panic!("expected a prompt");
    };
    assert_eq!(request.agents[0].name, "build");
    assert_eq!(request.skills[0].id, "opencode");
    assert!(draft.remove_agent(0));
    assert!(draft.remove_skill(0));
    assert!(!draft.is_sendable());
}

#[test]
fn commands_send_through_the_command_endpoint_and_clear_when_edited_away() {
    let mut draft = PromptDraft::default();
    draft.edit("/rev", 4, true);
    draft.select_command("review").unwrap();
    assert_eq!(draft.text(), "/review ");
    assert!(draft.command().is_some());

    draft.edit("/review HEAD~1", 14, true);
    let DraftSend::Command(command) = draft.to_send().unwrap() else {
        panic!("expected a command");
    };
    assert_eq!(command.name, "review");
    assert_eq!(command.text, "HEAD~1");

    draft.edit("/other", 6, true);
    assert!(draft.command().is_none());
}

#[test]
fn removing_a_command_keeps_the_arguments_as_a_plain_prompt() {
    let mut draft = PromptDraft::default();
    draft.edit("/rev", 4, true);
    draft.select_command("review").unwrap();
    draft.edit("/review HEAD~1", 14, true);

    let edit = draft.remove_command().unwrap();
    assert_eq!(edit.text, "HEAD~1");
    assert_eq!(edit.cursor, "HEAD~1".len());
    assert!(draft.command().is_none());
    let DraftSend::Prompt(request) = draft.to_send().unwrap() else {
        panic!("expected a prompt");
    };
    assert_eq!(request.text, "HEAD~1");

    // Without an active command there is nothing to strip.
    assert!(draft.remove_command().is_none());
}

#[test]
fn snippets_expand_in_place() {
    let mut draft = PromptDraft::default();
    draft.edit("please #rev", 11, true);
    let edit = draft.select_snippet("Review the changes.").unwrap();
    assert_eq!(edit.text, "please Review the changes.");
    assert_eq!(edit.cursor, edit.text.len());
    assert!(draft.trigger().is_none());
}

#[test]
fn path_to_file_uri_encodes_unsafe_bytes() {
    assert_eq!(path_to_file_uri("/tmp/a b"), "file:///tmp/a%20b");
    assert_eq!(path_to_file_uri("/tmp/100%"), "file:///tmp/100%25");
    assert_eq!(path_to_file_uri("relative"), "file:///relative");
}

#[test]
fn picker_filters_wraps_and_ignores_stale_results() {
    let (mut picker, effect) = PickerState::open(
        PickerKind::Command,
        "",
        vec![command_item("init"), command_item("review")],
    );
    assert_eq!(effect, PickerEffect::None);
    assert_eq!(picker.visible_len(), 2);

    picker.move_highlight(-1);
    assert_eq!(
        picker.selected().map(PickerItem::label),
        Some("/review".into())
    );
    picker.move_highlight(1);
    assert_eq!(
        picker.selected().map(PickerItem::label),
        Some("/init".into())
    );

    let effect = picker.update_query("rev");
    assert_eq!(effect, PickerEffect::None);
    assert_eq!(picker.visible_len(), 1);
    assert_eq!(
        picker.selected().map(PickerItem::label),
        Some("/review".into())
    );

    picker.update_query("zzz");
    assert_eq!(picker.visible_len(), 0);
    assert!(picker.selected().is_none());
}

#[test]
fn file_picker_merges_agents_and_drops_stale_file_results() {
    let agent = PickerItem::Agent {
        id: "build".into(),
        name: "Build".into(),
        description: "default".into(),
    };
    let (mut picker, effect) = PickerState::open(PickerKind::File, "", vec![agent]);
    let PickerEffect::FetchFiles { request_id, .. } = effect else {
        panic!("expected a fetch");
    };
    assert_eq!(request_id, 1);
    assert!(picker.loading());

    assert!(!picker.apply_file_results(0, vec![PickerItem::File { path: "old".into() }]));
    assert!(picker.apply_file_results(
        request_id,
        vec![PickerItem::File {
            path: "src/main.rs".into()
        }]
    ));
    assert!(!picker.loading());
    assert_eq!(picker.visible_len(), 2);
    let labels: Vec<String> = picker.visible().map(PickerItem::label).collect();
    assert_eq!(labels, ["src/main.rs", "Build"]);

    picker.update_query("main");
    assert_eq!(picker.visible_len(), 1);
}

#[test]
fn session_store_records_model_and_agent() {
    let mut store = SessionStore::default();
    store.replace(vec![session("ses_1", 10)]);
    assert!(store.set_model(
        "ses_1",
        ModelRef {
            id: "gpt-5".into(),
            provider_id: Some("openai".into()),
            variant: None,
        }
    ));
    assert!(store.set_agent("ses_1", "plan"));
    let selected = store.selected().unwrap();
    assert_eq!(selected.model.as_ref().unwrap().id, "gpt-5");
    assert_eq!(selected.agent.as_deref(), Some("plan"));
    assert!(!store.set_model("missing", ModelRef::default()));
}

#[test]
fn relative_time_covers_each_unit() {
    let now = 100_000_000_000;
    assert_eq!(relative_time(now, 0), "");
    assert_eq!(relative_time(now, now), "just now");
    assert_eq!(relative_time(now, now - 30_000), "just now");
    assert_eq!(relative_time(now, now - 60_000), "1m ago");
    assert_eq!(relative_time(now, now - 3_600_000), "1h ago");
    assert_eq!(relative_time(now, now - 86_400_000), "1d ago");
    assert_eq!(relative_time(now, now - 604_800_000), "1w ago");
    assert_eq!(relative_time(now, now - 2_592_000_000), "1mo ago");
    assert_eq!(relative_time(now, now - 31_536_000_000), "1y ago");
    assert_eq!(relative_time(now, now + 10_000), "just now");
}
