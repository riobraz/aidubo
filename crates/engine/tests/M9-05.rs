//! M9-05 — create, list, answer and settle a form.

mod support;

use std::time::{Duration, Instant};

use harness_engine::EngineEvent;
use support::mock::{MockProvider, Script};

#[test]
fn answers_a_form_and_it_settles() {
    if support::skip_e2e() {
        eprintln!("skipping form e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping form e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("ok"));
    let project = support::project("form");
    project.use_mock_provider(&mock.base_url());

    let started = support::start_engine(&project);
    let events = started.events.clone();
    let deadline = Instant::now() + Duration::from_secs(60);
    support::wait_for_stream(&events, deadline);

    assert!(started.engine.create_session(&started.directory,));
    let EngineEvent::SessionCreated { session } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SessionCreated { .. })
    }) else {
        unreachable!()
    };
    let id = session.id;

    assert!(started.engine.create_form(
        &started.directory,
        &id,
        "Ask",
        serde_json::json!([{ "key": "name", "type": "string", "required": true }]),
    ));
    let EngineEvent::FormCreated { form } = support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::FormCreated { .. })
    }) else {
        unreachable!()
    };
    assert_eq!(form.fields.len(), 1);

    assert!(started.engine.load_forms(&started.directory, &id));
    let EngineEvent::FormsLoaded { forms, .. } = support::wait_for(
        &events,
        deadline,
        |event| matches!(event, EngineEvent::FormsLoaded { forms, .. } if !forms.is_empty()),
    ) else {
        unreachable!()
    };
    assert_eq!(forms.len(), 1);

    assert!(started.engine.reply_form(
        &started.directory,
        &id,
        form.id.clone(),
        serde_json::json!({"name":"Ada"})
    ));
    support::wait_for(
        &events,
        deadline,
        |event| matches!(event, EngineEvent::FormSettled { form_id } if form_id == &form.id),
    );

    assert!(started.engine.load_forms(&started.directory, &id));
    let EngineEvent::FormsLoaded { forms, .. } = support::wait_for(
        &events,
        deadline,
        |event| matches!(event, EngineEvent::FormsLoaded { forms, .. } if forms.is_empty()),
    ) else {
        unreachable!()
    };
    assert!(forms.is_empty());

    support::shutdown(started.engine, started.port);
}
