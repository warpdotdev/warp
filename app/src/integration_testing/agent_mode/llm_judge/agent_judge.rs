//! Agentic LLM-as-judge for spec-implementation evals.
//!
//! Unlike `LLMJudge` (a single `/llm_generate` call over the transcript), this
//! judge runs as a fresh MAA conversation inside the same eval client: it
//! inherits the full default toolset, inspects the repo under review on disk,
//! and emits a structured rubric JSON in its final assistant message. The
//! harness parses that JSON, records per-item runtime tags, and computes the
//! pass/fail gate itself — the judge never authors `overall_pass`.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use warpui::integration::{AssertionOutcome, TestStep};
use warpui::SingletonEntity;

use crate::ai::llms::{LLMId, LLMPreferences};
use crate::integration_testing::agent_mode::{
    assert_rubric_from_final_message_with_expectations, set_preferred_agent_mode_llm,
    submit_ai_query_and_wait_until_done,
};

/// Overrides where the warp-control-cli-specs checkout is read from when
/// assembling the judge prompt. Defaults to the path the eval container
/// clones the specs repo into.
pub const SPECS_DIR_ENV_VAR: &str = "WARP_CONTROL_CLI_SPECS_DIR";
const DEFAULT_SPECS_DIR: &str = "/warp-control-cli-specs";

/// Configuration for one agentic judge invocation.
pub struct AgentJudgeConfig {
    /// Must be an available agent-mode LLM ID at run time; `judge_steps`
    /// validates it before swapping the model preference.
    pub judge_model: &'static str,
    /// The full user prompt the judge conversation is seeded with.
    pub user_prompt: String,
    /// Wall-clock cap on the judge step.
    pub timeout: Duration,
}

impl AgentJudgeConfig {
    /// Assembles the judge user prompt from the rubric. The model ID cannot be
    /// validated here — the available-LLM list lives in the running app — so
    /// `judge_steps` validates it with a non-panicking step at run time.
    pub fn new(judge_model: &'static str, rubric: &RubricSpec, timeout: Duration) -> Self {
        Self {
            judge_model,
            user_prompt: judge_user_prompt(rubric),
            timeout,
        }
    }
}

/// The structured verdict the judge emits in its final message.
/// `overall_pass` is intentionally absent: the harness computes it from
/// `items` so the gate never depends on judge discretion.
#[derive(Debug, Deserialize)]
pub struct AgentJudgeResult {
    pub items: Vec<RubricItem>,
    /// Supplementary capture; analytical only, never part of the gate.
    pub abstract_dimensions: AbstractDimensions,
    /// Free-form judge summary; not used for pass/fail.
    #[serde(default)]
    pub overall_critique: String,
}

#[derive(Debug, Deserialize)]
pub struct RubricItem {
    pub id: String,
    pub score: RubricScore,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RubricScore {
    Pass,
    Partial,
    Fail,
    NotImplemented,
}

impl RubricScore {
    pub fn as_str(self) -> &'static str {
        match self {
            RubricScore::Pass => "pass",
            RubricScore::Partial => "partial",
            RubricScore::Fail => "fail",
            RubricScore::NotImplemented => "not_implemented",
        }
    }
}

/// Holistic 1-5 scores capturing emergent quality the per-item rubric misses.
#[derive(Debug, Deserialize)]
pub struct AbstractDimensions {
    pub completeness: u8,
    pub correctness: u8,
    pub scope_discipline: u8,
}

/// The input shape that drives the judge prompt and the expected per-item ID
/// set. Rubric literals live in the sibling `rubrics` module.
pub struct RubricSpec {
    pub name: &'static str,
    pub items: &'static [RubricSpecItem],
}

pub struct RubricSpecItem {
    /// Matches `RubricItem::id` in the judge's output.
    pub id: &'static str,
    /// Behavior the judge looks for evidence of.
    pub description: &'static str,
    /// Narrative pointer into the spec (e.g. "TECH.md §0").
    pub spec_section: &'static str,
}

/// Expected per-item scores used to gate a judge run. The production gate is
/// `all_pass`; calibration cases override individual items (and may leave
/// unlisted items ungated by setting `default_score` to `None`).
#[derive(Clone, Copy)]
pub struct RubricExpectations {
    /// Expected score for items without an override. `None` leaves them
    /// ungated (their scores are still recorded as runtime tags).
    pub default_score: Option<RubricScore>,
    pub overrides: &'static [(&'static str, RubricScore)],
}

impl RubricExpectations {
    /// Default production policy: `overall_pass` iff every item is `pass`.
    pub fn all_pass() -> Self {
        Self {
            default_score: Some(RubricScore::Pass),
            overrides: &[],
        }
    }
}

/// The harness-computed gate for one judge run.
#[derive(Debug)]
pub struct RubricGateOutcome {
    pub overall_pass: bool,
    /// Human-readable description of each item that missed its expectation.
    pub failures: Vec<String>,
}

/// Builds the test steps that run the agentic judge against the current repo
/// state and gate on the default all-pass policy.
pub fn judge_steps(config: AgentJudgeConfig, rubric: &'static RubricSpec) -> Vec<TestStep> {
    judge_steps_with_expectations(config, rubric, RubricExpectations::all_pass())
}

/// Like `judge_steps`, but gates on caller-provided per-item expectations.
/// Calibration cases use this to assert known-good/known-bad outcomes.
pub fn judge_steps_with_expectations(
    config: AgentJudgeConfig,
    rubric: &'static RubricSpec,
    expectations: RubricExpectations,
) -> Vec<TestStep> {
    // `/new <prompt>` resets the conversation AND seeds it with the judge
    // prompt in one step: the slash command forwards its trailing argument as
    // the new conversation's initial prompt. The space after `/new` matters
    // for slash-menu disambiguation.
    let query = format!("/new {}", config.user_prompt);
    vec![
        validate_judge_model_step(config.judge_model),
        set_preferred_agent_mode_llm(config.judge_model),
        submit_ai_query_and_wait_until_done(&query, config.timeout).add_named_assertion(
            "Parse rubric JSON from the judge's final message and apply the gate",
            assert_rubric_from_final_message_with_expectations(rubric, expectations),
        ),
    ]
}

/// `set_preferred_agent_mode_llm` panics on an unknown LLM ID, which would
/// abort the app before debug info is exported. Validate the judge model with
/// an immediate failure instead so a bad config stays inspectable.
fn validate_judge_model_step(judge_model: &'static str) -> TestStep {
    let llm_id = LLMId::from(judge_model);
    TestStep::new(&format!("Validate judge model '{judge_model}'")).add_named_assertion(
        "Judge model is an available agent-mode LLM",
        move |app, _window_id| {
            let llm_id = llm_id.clone();
            let is_available = LLMPreferences::handle(app).read(app, |llm_preferences, _| {
                llm_preferences.is_available_agent_mode_llm(&llm_id)
            });
            if is_available {
                AssertionOutcome::Success
            } else {
                AssertionOutcome::immediate_failure(format!(
                    "Judge model '{llm_id}' is not a valid agent mode LLM"
                ))
            }
        },
    )
}

/// Extracts the judge's rubric JSON from its final message text. The prompt
/// asks for exactly one fenced JSON block; we accept the last parseable
/// fenced block (or a bare-JSON message) for robustness. A missing or
/// unparseable payload is an error — callers must hard-fail on it.
pub fn parse_agent_judge_result(message_text: &str) -> Result<AgentJudgeResult, String> {
    let mut parsed = None;
    let mut last_error: Option<String> = None;
    for block in fenced_code_blocks(message_text) {
        match serde_json::from_str::<AgentJudgeResult>(block.trim()) {
            Ok(result) => parsed = Some(result),
            Err(err) => last_error = Some(err.to_string()),
        }
    }
    if let Some(result) = parsed {
        return Ok(result);
    }
    serde_json::from_str::<AgentJudgeResult>(message_text.trim()).map_err(|bare_err| {
        match last_error {
            Some(fence_err) => format!("no fenced code block deserialized ({fence_err})"),
            None => format!("no fenced code block found and message is not bare JSON ({bare_err})"),
        }
    })
}

/// Returns the contents of all ``` fenced code blocks in `text`, with any
/// `json` language tag on the opening fence stripped.
fn fenced_code_blocks(text: &str) -> Vec<&str> {
    let mut blocks = Vec::new();
    let mut segments = text.split("```");
    // Text before the first fence occupies the first segment.
    let _ = segments.next();
    while let Some(block) = segments.next() {
        blocks.push(block.strip_prefix("json").unwrap_or(block));
        // Skip the prose between this fence's close and the next fence's open.
        if segments.next().is_none() {
            break;
        }
    }
    blocks
}

/// Validates that every rubric ID appears exactly once in the judge's output
/// (an error otherwise) and computes the harness-side gate against
/// `expectations`. Extra unknown IDs in the output are tolerated.
pub fn evaluate_rubric_result(
    rubric: &RubricSpec,
    expectations: RubricExpectations,
    result: &AgentJudgeResult,
) -> Result<RubricGateOutcome, String> {
    let mut failures = Vec::new();
    for spec_item in rubric.items {
        let matching: Vec<&RubricItem> = result
            .items
            .iter()
            .filter(|item| item.id == spec_item.id)
            .collect();
        let item = match matching.as_slice() {
            [item] => *item,
            [] => {
                return Err(format!(
                    "Judge output is missing rubric item '{}'",
                    spec_item.id
                ))
            }
            _ => {
                return Err(format!(
                    "Judge output contains rubric item '{}' more than once",
                    spec_item.id
                ))
            }
        };
        let expected = expectations
            .overrides
            .iter()
            .find(|(id, _)| *id == spec_item.id)
            .map(|(_, score)| *score)
            .or(expectations.default_score);
        if let Some(expected) = expected {
            if item.score != expected {
                failures.push(format!(
                    "{}: expected {}, judge scored {} (evidence: {})",
                    spec_item.id,
                    expected.as_str(),
                    item.score.as_str(),
                    item.evidence
                ));
            }
        }
    }
    Ok(RubricGateOutcome {
        overall_pass: failures.is_empty(),
        failures,
    })
}

/// Assembles the judge user prompt for `rubric`.
///
/// The slice-relevant spec excerpt is loaded from the specs checkout at
/// prompt-build time (rather than embedded as a const) so the inlined text
/// always matches the `SPECS_COMMIT_HASH` the eval container pins — embedding
/// a copy in this crate would silently drift when the pin is bumped. Prompt
/// assembly happens inside the eval container, where the checkout exists at
/// the default path; local runs can point `WARP_CONTROL_CLI_SPECS_DIR` at a
/// checkout. If loading fails, the prompt falls back to directing the judge
/// at the on-disk spec files, which the prompt references anyway.
pub fn judge_user_prompt(rubric: &RubricSpec) -> String {
    JUDGE_PROMPT_TEMPLATE
        .replace("{rubric_items}", &render_rubric_items(rubric))
        .replace("{spec_excerpt}", &load_spec_excerpt())
}

fn render_rubric_items(rubric: &RubricSpec) -> String {
    rubric
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            format!(
                "{}. {}: {} ({})",
                index + 1,
                item.id,
                item.description,
                item.spec_section
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Loads the slice-relevant spec content: TECH.md §0–§3 plus PRODUCT.md's
/// numbered behaviors section. Sections are located by heading markers, not
/// line numbers, so minor edits to the specs don't break extraction.
fn load_spec_excerpt() -> String {
    let specs_dir =
        std::env::var(SPECS_DIR_ENV_VAR).unwrap_or_else(|_| DEFAULT_SPECS_DIR.to_owned());
    let specs_dir = Path::new(&specs_dir);
    let tech = extract_section(&specs_dir.join("TECH.md"), "### 0.", "### 4.");
    let product = extract_section(&specs_dir.join("PRODUCT.md"), "## Behavior", "## ");
    match (tech, product) {
        (Some(tech), Some(product)) => {
            format!("\n\n## TECH.md §0–§3\n{tech}\n\n## PRODUCT.md numbered behaviors\n{product}")
        }
        _ => {
            log::warn!(
                "Could not load spec excerpt from {}; judge prompt will reference on-disk files only",
                specs_dir.display()
            );
            "(excerpt unavailable in this environment — read the spec files listed above directly)"
                .to_owned()
        }
    }
}

/// Returns the lines from the first line starting with `start_marker` up to
/// (exclusive) the next subsequent line starting with `end_marker`.
fn extract_section(path: &Path, start_marker: &str, end_marker: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.starts_with(start_marker))?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.starts_with(end_marker))
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());
    Some(lines[start..end].join("\n"))
}

const JUDGE_PROMPT_TEMPLATE: &str = r#"You are an expert code reviewer evaluating an implementation of the warpctrl first vertical slice.

# Where things are
- Implementation under review: /warp (a checkout of warpdotdev/warp). The agent's diff against the warp base commit can be obtained with `git -C /warp diff`. Start there.
- Spec the agent was asked to implement: /warp-control-cli-specs/{PRODUCT,TECH,SECURITY,README}.md.
- Key sections of the spec, inlined below for convenience: {spec_excerpt}

# Scope
The agent was asked to implement only the first vertical slice:
- TECH.md §0 — security architecture foundations
- TECH.md §1 — shared protocol module
- TECH.md §2 — per-process discovery
- TECH.md §3 — local authentication / safety boundary
- `warpctrl tab create` end-to-end + standalone `warpctrl` binary + `warpctrl instance list` + `warpctrl --version`

Nothing else is in scope. Items the agent shipped that are outside this scope count against `scope_discipline`, not against the per-item rubric.

# Rubric
For each item below, decide a 4-level score and cite the evidence (file path + approximate line range, or a short rationale when the artifact is absent).

Scale:
- `pass` — implemented and matches the cited spec clause
- `partial` — present and partially correct; missing a sub-requirement of the cited spec clause
- `fail` — present but violates the cited spec clause (e.g. `SyncToCloud::Always` where the spec requires `Never`)
- `not_implemented` — the implementation artifact is absent from the diff/source

`not_implemented` is an **expected outcome** on incomplete implementations and is informative signal, not a defect. Score it cleanly without negative framing.

Items:
{rubric_items}

# Abstract dimensions
After scoring the rubric, also produce three holistic 1–5 scores:
- `completeness` — fraction of spec deliverables actually shipped, judged holistically. Can drop even when every named item passes if the agent left obvious adjacent work unfinished.
- `correctness` — where the agent shipped code, does it match what the spec asked for (separate from "does the artifact exist").
- `scope_discipline` — did the agent stay close to the first-slice scope, or sprawl into follow-up work / unrelated cleanup / over-engineering.

These are analytical, not gating. Fold a one-sentence rationale per dimension into `overall_critique`. Do not default to 3-across-the-board; commit to a score.

# How to investigate
1. Start cheaply: `git -C /warp diff --name-only` and `git -C /warp diff --stat`. This is your map.
2. Read the spec excerpt above; refer to the full files in `/warp-control-cli-specs/` only when you need detail not inlined.
3. For each rubric item, locate evidence in the agent's source. Prefer `read_files`, `grep`, `file_glob`, `search_codebase` over wholesale exploration.
4. Score and emit JSON.

# Constraints
- You are reviewing, not modifying. Do not call `edit_files`, `apply_file_diffs`, `request_file_edits`, or any tool that mutates the repo.
- Do not run `git reset`, `git checkout .`, `git stash`, `rm`, or any command that disturbs the working tree.
- Do not spawn child agents with `run_agents`.
- Do not anchor your judgment on specific file paths; the agent's module split is free to differ from what TECH.md sketches, as long as each rubric item's behavior is implemented somewhere.

# Output
Your final message must contain exactly one fenced JSON code block matching this schema:

```json
{
  "items": [
    {"id": "<rubric_id>", "score": "pass|partial|fail|not_implemented", "evidence": "<file:line range or short rationale>"}
  ],
  "abstract_dimensions": {
    "completeness": 1,
    "correctness": 1,
    "scope_discipline": 1
  },
  "overall_critique": "<2-4 sentence summary including the one-sentence rationale per abstract dimension>"
}
```

Every rubric ID listed above must appear in `items` exactly once. Do not author an `overall_pass` field — the harness computes it from per-item scores.
"#;

#[cfg(test)]
#[path = "agent_judge_tests.rs"]
mod tests;
