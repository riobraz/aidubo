//! M12-02 — create, fire, pause, resume and delete a schedule.

mod support;

use std::time::{Duration, Instant};

use async_channel::Receiver;
use harness_engine::{CreateScheduleRequest, Engine, EngineEvent};
use harness_protocol::{ScheduleInfo, ScheduleStatus, ScheduleWhen};
use support::mock::{MockProvider, Script};

fn list_schedules(
    engine: &Engine,
    directory: &str,
    events: &Receiver<EngineEvent>,
    request_id: u64,
    session_id: &str,
    deadline: Instant,
) -> Vec<ScheduleInfo> {
    assert!(engine.load_schedules(directory, request_id, session_id));
    let EngineEvent::SchedulesLoaded { result, .. } =
        support::wait_for(events, deadline, |event| {
            matches!(
                event,
                EngineEvent::SchedulesLoaded {
                    request_id: id,
                    ..
                } if *id == request_id
            )
        })
    else {
        unreachable!()
    };
    result.expect("schedule list failed")
}

#[test]
fn schedules_a_prompt() {
    if support::skip_e2e() {
        eprintln!("skipping schedule e2e test");
        return;
    }
    if !support::opencode_available() {
        eprintln!("skipping schedule e2e test: opencode was not found");
        return;
    }

    let mock = MockProvider::start(Script::single("ok"));
    let project = support::project("schedule");
    project.use_mock_provider(&mock.base_url());
    project.enable_schedules();

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
    let session_id = session.id;

    // A one-shot relative schedule must fire on the server's clock.
    assert!(started.engine.create_schedule(
        &started.directory,
        1,
        CreateScheduleRequest {
            session_id: session_id.clone(),
            name: Some("later".into()),
            when: ScheduleWhen::in_("2 seconds"),
            if_condition: None,
            until: None,
            ends: Some(serde_json::json!({ "times": 1 })),
            prompt: "scheduled".into(),
        },
    ));
    let EngineEvent::SchedulesLoaded { result, .. } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::SchedulesLoaded { request_id: 1, .. })
        })
    else {
        unreachable!()
    };
    let created = result
        .expect("schedule list failed")
        .into_iter()
        .find(|schedule| schedule.name == "later")
        .expect("created schedule missing");

    let fired_deadline = Instant::now() + Duration::from_secs(20);
    let mut fired = false;
    let mut request_id = 10;
    while Instant::now() < fired_deadline {
        let schedules = list_schedules(
            &started.engine,
            &started.directory,
            &events,
            request_id,
            &session_id,
            fired_deadline,
        );
        request_id += 1;
        if schedules
            .iter()
            .find(|schedule| schedule.id == created.id)
            .is_some_and(|schedule| schedule.run_count > 0 || schedule.last_fired_at.is_some())
        {
            fired = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(fired, "the relative schedule never fired");

    // A recurring schedule can be paused and resumed without losing its count.
    assert!(started.engine.create_schedule(
        &started.directory,
        20,
        CreateScheduleRequest {
            session_id: session_id.clone(),
            name: Some("loop".into()),
            when: ScheduleWhen::every("1 hour"),
            if_condition: None,
            until: None,
            ends: None,
            prompt: "again".into(),
        },
    ));
    let EngineEvent::SchedulesLoaded { result, .. } =
        support::wait_for(&events, deadline, |event| {
            matches!(event, EngineEvent::SchedulesLoaded { request_id: 20, .. })
        })
    else {
        unreachable!()
    };
    let looped = result
        .expect("schedule list failed")
        .into_iter()
        .find(|schedule| schedule.name == "loop")
        .expect("recurring schedule missing");

    assert!(started.engine.pause_schedule(
        &started.directory,
        30,
        session_id.clone(),
        looped.id.clone()
    ));
    support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SchedulesLoaded { request_id: 30, .. })
    });
    let paused = list_schedules(
        &started.engine,
        &started.directory,
        &events,
        31,
        &session_id,
        deadline,
    );
    let paused = paused
        .iter()
        .find(|schedule| schedule.id == looped.id)
        .expect("paused schedule missing");
    assert_eq!(paused.status, ScheduleStatus::Paused);
    let paused_runs = paused.run_count;

    std::thread::sleep(Duration::from_secs(3));
    let stable = list_schedules(
        &started.engine,
        &started.directory,
        &events,
        32,
        &session_id,
        deadline,
    );
    let stable = stable
        .iter()
        .find(|schedule| schedule.id == looped.id)
        .expect("paused schedule missing");
    assert_eq!(stable.run_count, paused_runs);

    assert!(started.engine.resume_schedule(
        &started.directory,
        40,
        session_id.clone(),
        looped.id.clone()
    ));
    support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SchedulesLoaded { request_id: 40, .. })
    });
    let resumed = list_schedules(
        &started.engine,
        &started.directory,
        &events,
        41,
        &session_id,
        deadline,
    );
    assert_eq!(
        resumed
            .iter()
            .find(|schedule| schedule.id == looped.id)
            .expect("resumed schedule missing")
            .status,
        ScheduleStatus::Active
    );

    assert!(started.engine.delete_schedule(
        &started.directory,
        50,
        session_id.clone(),
        looped.id.clone()
    ));
    support::wait_for(&events, deadline, |event| {
        matches!(event, EngineEvent::SchedulesLoaded { request_id: 50, .. })
    });
    let after_delete = list_schedules(
        &started.engine,
        &started.directory,
        &events,
        51,
        &session_id,
        deadline,
    );
    assert!(
        !after_delete.iter().any(|schedule| schedule.id == looped.id),
        "deleted schedule is still listed"
    );

    support::shutdown(started.engine, started.port);
}
