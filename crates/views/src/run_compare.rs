//! Pure view-model helpers for the multi-run compare pane.
//!
//! Rendering lives in `workspace.rs`; this module owns the decisions that are
//! worth testing without a window: how columns fall back to tabs, how a state
//! is labeled, how the transcript tail is bounded, and how a group is built
//! from the form.

use std::collections::HashMap;

use harness_core::{MAX_RUNS_PER_GROUP, Run, RunGroup, RunState};
use harness_protocol::ModelRef;

/// Characters of each run's streamed text kept for the column tail.
pub const TAIL_LIMIT: usize = 600;

/// Below this viewport width the pane shows one run at a time, switchable by
/// tabs, instead of columns.
pub const TABS_BELOW: f32 = 860.0;

/// How many columns fit before the pane scrolls horizontally.
pub const MAX_VISIBLE_COLUMNS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareLayout {
    /// One column plus a run tab bar.
    Tabs,
    /// Up to this many columns side by side.
    Columns(usize),
}

pub fn layout_for(width: f32, runs: usize) -> CompareLayout {
    if width < TABS_BELOW {
        return CompareLayout::Tabs;
    }
    CompareLayout::Columns(runs.clamp(2, MAX_VISIBLE_COLUMNS))
}

/// A short, stable label for the badge under a column header.
pub fn state_label(state: &RunState) -> &'static str {
    match state {
        RunState::Pending => "Queued",
        RunState::Starting => "Starting…",
        RunState::Running => "Running",
        RunState::Done => "Done",
        RunState::Failed { .. } => "Failed",
        RunState::Discarded => "Discarded",
    }
}

/// The badge is colored by this class rather than by message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateTone {
    Muted,
    Live,
    Success,
    Danger,
}

pub fn state_tone(state: &RunState) -> StateTone {
    match state {
        RunState::Pending | RunState::Discarded => StateTone::Muted,
        RunState::Starting | RunState::Running => StateTone::Live,
        RunState::Done => StateTone::Success,
        RunState::Failed { .. } => StateTone::Danger,
    }
}

/// `provider/model` when the provider is known, otherwise the bare model id.
pub fn model_label(model: &ModelRef) -> String {
    match model.provider_id.as_deref().filter(|id| !id.is_empty()) {
        Some(provider) => format!("{provider}/{}", model.id),
        None => model.id.clone(),
    }
}

/// Keep the end of a run's streamed text, on a character boundary, with a
/// leading ellipsis when anything was dropped.
pub fn tail(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    let skip = count - limit;
    let mut tail: String = text.chars().skip(skip).collect();
    tail.insert(0, '…');
    tail
}

/// Per-run streaming tails, keyed by session id.
#[derive(Debug, Default)]
pub struct RunTails {
    tails: HashMap<String, String>,
}

impl RunTails {
    pub fn push(&mut self, session_id: &str, delta: &str) {
        let text = self.tails.entry(session_id.to_string()).or_default();
        text.push_str(delta);
        if text.chars().count() > TAIL_LIMIT + TAIL_LIMIT / 2 {
            *text = tail(text, TAIL_LIMIT);
        }
    }

    pub fn text(&self, session_id: &str) -> Option<&str> {
        self.tails.get(session_id).map(String::as_str)
    }

    pub fn clear(&mut self) {
        self.tails.clear();
    }
}

/// Build a group from the compare form: one prompt, a distinct model per run.
/// Every error is user-facing.
pub fn build_group(
    prompt: &str,
    models: &[ModelRef],
    created: i64,
    nonce: u32,
) -> Result<RunGroup, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("A compare run needs a prompt.".to_string());
    }
    if models.is_empty() {
        return Err("Pick at least one model to compare.".to_string());
    }
    if models.len() > MAX_RUNS_PER_GROUP {
        return Err(format!(
            "At most {MAX_RUNS_PER_GROUP} models per comparison."
        ));
    }
    let mut seen = Vec::new();
    for model in models {
        let key = (model.provider_id.clone(), model.id.clone());
        if seen.contains(&key) {
            return Err("Each model can appear once per comparison.".to_string());
        }
        seen.push(key);
    }
    let group_id = format!("runs-{created}-{nonce:08x}");
    let runs = models
        .iter()
        .enumerate()
        .map(|(index, model)| Run {
            id: format!("run-{}", index + 1),
            model: model.clone(),
            session_id: None,
            worktree: None,
            state: RunState::Pending,
        })
        .collect();
    Ok(RunGroup {
        id: group_id,
        prompt: prompt.to_string(),
        created,
        runs,
        kept: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> ModelRef {
        ModelRef {
            id: id.to_string(),
            provider_id: Some("mock".to_string()),
            variant: None,
        }
    }

    #[test]
    fn run_compare_falls_back_to_tabs_when_narrow() {
        assert_eq!(layout_for(1200.0, 3), CompareLayout::Columns(3));
        assert_eq!(layout_for(1200.0, 5), CompareLayout::Columns(3));
        assert_eq!(layout_for(1200.0, 1), CompareLayout::Columns(2));
        assert_eq!(layout_for(700.0, 3), CompareLayout::Tabs);
    }

    #[test]
    fn run_compare_state_labels_and_tones_are_stable() {
        assert_eq!(state_label(&RunState::Pending), "Queued");
        assert_eq!(state_label(&RunState::Running), "Running");
        assert_eq!(state_label(&RunState::Done), "Done");
        assert_eq!(
            state_label(&RunState::Failed {
                message: "no".into()
            }),
            "Failed"
        );
        assert_eq!(state_tone(&RunState::Running), StateTone::Live);
        assert_eq!(state_tone(&RunState::Done), StateTone::Success);
        assert_eq!(state_tone(&RunState::Discarded), StateTone::Muted);
        assert_eq!(
            state_tone(&RunState::Failed {
                message: "no".into()
            }),
            StateTone::Danger
        );
    }

    #[test]
    fn run_compare_tail_keeps_the_end_and_respects_boundaries() {
        assert_eq!(tail("short", 10), "short");
        assert_eq!(tail("abcdef", 3), "…def");
        assert_eq!(tail("aé日", 2), "…é日");
    }

    #[test]
    fn run_compare_streaming_tail_is_bounded() {
        let mut tails = RunTails::default();
        for _ in 0..(TAIL_LIMIT * 3) {
            tails.push("ses-1", "x");
        }
        let text = tails.text("ses-1").expect("tail");
        assert!(text.chars().count() <= TAIL_LIMIT + TAIL_LIMIT / 2);
        assert!(text.starts_with('…'));
        assert!(tails.text("missing").is_none());

        // A single oversized delta is cut down to the render limit plus the
        // ellipsis instead of growing without bound.
        let mut oversized = RunTails::default();
        oversized.push("ses-2", &"y".repeat(TAIL_LIMIT * 4));
        let text = oversized.text("ses-2").expect("tail");
        assert_eq!(text.chars().count(), TAIL_LIMIT + 1);
        assert!(text.starts_with('…'));
    }

    #[test]
    fn run_compare_group_requires_distinct_models_and_a_prompt() {
        assert!(build_group("  ", &[model("a")], 1, 2).is_err());
        assert!(build_group("go", &[], 1, 2).is_err());
        assert!(build_group("go", &[model("a"), model("a")], 1, 2).is_err());
        let group = build_group("go", &[model("a"), model("b")], 1, 2).expect("group");
        assert_eq!(group.runs.len(), 2);
        assert_eq!(group.runs[1].id, "run-2");
        assert_eq!(group.runs[0].state, RunState::Pending);
        assert_eq!(group.id, "runs-1-00000002");
        assert_eq!(group.prompt, "go");
    }
}
