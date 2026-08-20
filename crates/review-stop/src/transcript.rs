use crate::decision::{
  CLIPPY_BLOCK_MARKER, REVIEW_BLOCK_MARKER, TASK_NOTIFICATION_MARKER,
};
use crate::error::AppError;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use tracing::debug;

/// The `subagent_type` of the review whose verdict releases the gate.
const REVIEW_AGENT: &str = "template-compliance";

/// The line the reviewer closes its report with; the verdict is read from it.
const VERDICT_MARKER: &str = "COMPLIANCE:";

/// The most recent verdict the review delivered this turn.
pub enum Verdict {
  Pass,
  Findings {
    text: String,
  },
  /// No review has reported since the last prompt.
  Absent,
}

pub struct Analysis {
  /// Index, into the parsed entries, of the last real user prompt — the start
  /// of "this turn".  `None` when the transcript holds no prompt at all.
  pub last_prompt_idx: Option<usize>,
  pub verdict: Verdict,
}

impl Analysis {
  /// The key the per-turn round counter is filed under: the prompt index as
  /// text, `-1` when there is none.
  pub fn turn_key(&self) -> String {
    self
      .last_prompt_idx
      .map_or_else(|| "-1".to_string(), |idx| idx.to_string())
  }
}

/// Read the transcript and work out where this turn starts and what the
/// review has said since.  One parse serves both questions.
pub fn analyze(path: &Path) -> Result<Analysis, AppError> {
  load(path).and_then(|entries| analyze_entries(&entries))
}

fn analyze_entries(entries: &[Value]) -> Result<Analysis, AppError> {
  let last_prompt_idx = entries.iter().rposition(is_prompt);
  Ok(Analysis {
    last_prompt_idx,
    verdict: verdict(
      entries
        .get(last_prompt_idx.map_or(0, |idx| idx + 1)..)
        .unwrap_or_default(),
    )?,
  })
}

/// The transcript's entries, one per parsable JSONL line.  A line that does
/// not parse — a partially written tail, say — is skipped, so the index of
/// every later entry is its position among the parsed ones.
fn load(path: &Path) -> Result<Vec<Value>, AppError> {
  fs::read_to_string(path)
    .map_err(|source| AppError::TranscriptRead {
      path: path.to_path_buf(),
      source,
    })
    .map(|raw| {
      raw
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(parse_entry)
        .collect()
    })
}

fn parse_entry(line: &str) -> Option<Value> {
  serde_json::from_str(line).map_or_else(
    |error| {
      debug!(%error, "skipping an unparsable transcript line");
      None
    },
    Some,
  )
}

/// A "user" entry that carries a text prompt — string content, or a content
/// array holding a text block — and that a human wrote.  A user entry whose
/// content is only tool_results is a tool response, not a prompt.
///
/// Two kinds of user text turn are machine-authored rather than prompts: this
/// gate's own block reason, re-injected after a block, and the notification a
/// background subagent posts when it finishes.  Counting either as a fresh
/// prompt moves the turn boundary past the review that just ran, hiding its
/// verdict, and resets the round counter — so the gate can neither converge
/// nor give up, and spins without bound.  Both are recognised by their marker
/// text and skipped.
fn is_prompt(entry: &Value) -> bool {
  entry_type(entry) == Some("user")
    && entry.pointer("/message/content").is_some_and(|content| {
      content.is_string()
        || content.as_array().is_some_and(|blocks| {
          blocks
            .iter()
            .any(|block| str_at(block, "/type") == Some("text"))
        })
    })
    && !machine_authored(entry)
}

fn machine_authored(entry: &Value) -> bool {
  hook_authored(entry) || contains_text(entry, TASK_NOTIFICATION_MARKER)
}

/// Whether the entry carries one of this gate's own block reasons.
fn hook_authored(entry: &Value) -> bool {
  contains_text(entry, REVIEW_BLOCK_MARKER)
    || contains_text(entry, CLIPPY_BLOCK_MARKER)
}

/// Whether any string anywhere in the value contains `needle`.  Marker tests
/// run against the whole entry rather than a modelled content shape:
/// machine-authored text reaches the transcript by several routes — a text
/// block, a tool_result whose text sits in `.content`, or a `toolUseResult`
/// field alongside `.message` entirely — and a reader that models one of them
/// silently sees nothing for the others.  Only the marker matters, so where it
/// sits does not.
fn contains_text(value: &Value, needle: &str) -> bool {
  match value {
    Value::String(text) => text.contains(needle),
    Value::Array(items) => items.iter().any(|item| contains_text(item, needle)),
    Value::Object(fields) => {
      fields.values().any(|field| contains_text(field, needle))
    }
    Value::Null | Value::Bool(_) | Value::Number(_) => false,
  }
}

/// The verdict is the most recent COMPLIANCE: line the review delivered this
/// turn, gathered in one pass, in transcript order, across every channel a
/// verdict can arrive on.  Transcript order is the point: consecutive verdicts
/// often arrive by different channels, and ranking by channel instead would
/// let a stale verdict outrank a fresher one.
///
/// The channels, and what admits each:
///
/// - A foreground review reports in its own tool_result, admitted by the
///   launch's id.
/// - A background review left to finish posts a task-notification with no
///   tool_use_id to match on; depending on what was in flight it rides inside
///   a user turn, or is queued as a queued_command attachment plus the
///   queue-operation bookkeeping around it.  These are admitted by their
///   text, so only texts carrying a COMPLIANCE: line count — which also keeps
///   launch metadata from reading as findings.
/// - A background review waited on with TaskOutput reports in TaskOutput's
///   own tool_result and writes no notification; the call is joined back to
///   the review through its task_id — the agentId the launch reported — and
///   admitted by that join, so another agent's TaskOutput is not a verdict
///   however its text reads.
fn verdict(turn: &[Value]) -> Result<Verdict, AppError> {
  let review_ids: HashSet<&str> = tool_uses(turn)
    .filter(|block| {
      // The subagent-spawning tool is named "Task" in stock Claude Code but
      // "Agent" in some harnesses; match either so the review is detected
      // regardless of which one recorded the call.
      matches!(str_at(block, "/name"), Some("Task" | "Agent"))
        && str_at(block, "/input/subagent_type") == Some(REVIEW_AGENT)
    })
    .filter_map(|block| str_at(block, "/id"))
    .collect();
  // The agent ids those launches reported.  A foreground review reports none,
  // and neither source is guaranteed present on a background one, so both are
  // read and an absent one simply yields nothing.
  let agent_ids: HashSet<String> = turn
    .iter()
    .filter(|entry| entry_type(entry) == Some("user"))
    .flat_map(|entry| {
      content_blocks(entry)
        .filter(|block| is_tool_result_for(block, &review_ids))
        .flat_map(move |block| {
          str_at(entry, "/toolUseResult/agentId")
            .map(str::to_string)
            .into_iter()
            .chain(agent_id_in(&message_text(block.get("content"))))
        })
    })
    .collect();
  let verdict_ids: HashSet<&str> = review_ids
    .iter()
    .copied()
    .chain(
      tool_uses(turn)
        .filter(|block| {
          str_at(block, "/name") == Some("TaskOutput")
            && str_at(block, "/input/task_id")
              .is_some_and(|task_id| agent_ids.contains(task_id))
        })
        .filter_map(|block| str_at(block, "/id")),
    )
    .collect();
  Ok(
    turn
      .iter()
      .map(|entry| entry_verdict_texts(entry, &verdict_ids))
      .collect::<Result<Vec<_>, _>>()?
      .into_iter()
      .flatten()
      .rev()
      .find(|text| text.contains(VERDICT_MARKER))
      .map_or(Verdict::Absent, |text| {
        if is_pass(&text) {
          Verdict::Pass
        } else {
          Verdict::Findings { text }
        }
      }),
  )
}

/// The verdict-carrying texts one entry contributes, in the order they sit in
/// it.  Only an entry the harness or the human wrote can deliver a verdict —
/// never the assistant, whose prose quotes "COMPLIANCE: PASS" whenever it
/// discusses this gate.  A tool_result is admitted by id, so it needs no
/// further filter.
///
/// The hook_authored filter is load-bearing for one specific shape: a block
/// reason quotes the findings text it is reporting, so a re-injected block
/// carries both a task-notification tag and the words "COMPLIANCE: PASS" from
/// this gate's own explanation of the rule.  Admitting it would release the
/// gate on the strength of the gate's own prose.
fn entry_verdict_texts(
  entry: &Value,
  verdict_ids: &HashSet<&str>,
) -> Result<Vec<String>, AppError> {
  Ok(
    tool_result_texts(entry, verdict_ids)
      .chain(notification_text(entry)?)
      .collect(),
  )
}

/// The texts of a user entry's tool_results whose id is one of `ids`.
fn tool_result_texts<'a>(
  entry: &'a Value,
  ids: &'a HashSet<&'a str>,
) -> impl Iterator<Item = String> + 'a {
  (entry_type(entry) == Some("user"))
    .then(|| {
      content_blocks(entry)
        .filter(move |block| is_tool_result_for(block, ids))
        .map(|block| message_text(block.get("content")))
    })
    .into_iter()
    .flatten()
}

/// The entry itself, serialised, when it carries a background notification.
fn notification_text(entry: &Value) -> Result<Option<String>, AppError> {
  (notification_carrier(entry)
    && !hook_authored(entry)
    && contains_text(entry, TASK_NOTIFICATION_MARKER))
  .then(|| {
    serde_json::to_string(entry).map_err(AppError::TranscriptEntrySerialize)
  })
  .transpose()
}

/// The entry types a real transcript records a background notification
/// under.  They are named rather than taken as "anything but assistant"
/// because a compaction summary is assistant-written under yet another type.
fn notification_carrier(entry: &Value) -> bool {
  matches!(entry_type(entry), Some("user" | "queue-operation"))
    || (entry_type(entry) == Some("attachment")
      && str_at(entry, "/attachment/type") == Some("queued_command"))
}

/// Whether the text reports a pass: `COMPLIANCE:` followed by optional
/// whitespace and `PASS`, anywhere in it.
fn is_pass(text: &str) -> bool {
  text.match_indices(VERDICT_MARKER).any(|(start, marker)| {
    text
      .get(start + marker.len()..)
      .is_some_and(|rest| rest.trim_start().starts_with("PASS"))
  })
}

/// The first `agentId: <id>` the text carries: the id is the run of
/// `[A-Za-z0-9_-]` after optional whitespace, and an `agentId:` with nothing
/// id-like after it is passed over in favour of a later one.
fn agent_id_in(text: &str) -> Option<String> {
  text.match_indices("agentId:").find_map(|(start, marker)| {
    let id: String = text
      .get(start + marker.len()..)?
      .trim_start()
      .chars()
      .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
      .collect();
    (!id.is_empty()).then_some(id)
  })
}

/// The text of a tool_result's content: the string itself, or the text blocks
/// of an array joined by newlines.
fn message_text(content: Option<&Value>) -> String {
  match content {
    Some(Value::String(text)) => text.clone(),
    Some(Value::Array(blocks)) => blocks
      .iter()
      .map(|block| str_at(block, "/text").unwrap_or(""))
      .collect::<Vec<_>>()
      .join("\n"),
    _ => String::new(),
  }
}

/// The tool_use blocks of every assistant entry in the turn.
fn tool_uses(turn: &[Value]) -> impl Iterator<Item = &Value> {
  turn
    .iter()
    .filter(|entry| entry_type(entry) == Some("assistant"))
    .flat_map(content_blocks)
    .filter(|block| str_at(block, "/type") == Some("tool_use"))
}

fn is_tool_result_for(block: &Value, ids: &HashSet<&str>) -> bool {
  str_at(block, "/type") == Some("tool_result")
    && str_at(block, "/tool_use_id").is_some_and(|id| ids.contains(id))
}

fn entry_type(entry: &Value) -> Option<&str> {
  str_at(entry, "/type")
}

fn content_blocks(entry: &Value) -> impl Iterator<Item = &Value> {
  entry
    .pointer("/message/content")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
  value.pointer(pointer).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn entries(jsonl: &str) -> Vec<Value> {
    jsonl
      .lines()
      .filter(|line| !line.is_empty())
      .filter_map(parse_entry)
      .collect()
  }

  const PROMPT: &str =
    r#"{"type":"user","message":{"content":"please change the code"}}"#;
  const LAUNCH: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","id":"rev1","input":{"subagent_type":"template-compliance"}}]}}"#;

  #[test]
  fn pass_is_read_anywhere_after_the_marker() {
    assert!(is_pass("No findings.\n\nCOMPLIANCE:   PASS"));
    assert!(is_pass("COMPLIANCE: FINDINGS\n...\nCOMPLIANCE: PASS"));
    assert!(!is_pass("COMPLIANCE: FINDINGS"));
    assert!(!is_pass("COMPLIANCE:X, and PASS elsewhere"));
  }

  #[test]
  fn agent_id_skips_a_bare_marker_for_a_later_one() {
    assert_eq!(
      agent_id_in("agentId: abc-1_2 (internal)"),
      Some("abc-1_2".to_string())
    );
    assert_eq!(
      agent_id_in("agentId: (none) then agentId:xyz"),
      Some("xyz".to_string())
    );
    assert_eq!(agent_id_in("no id here"), None);
  }

  #[test]
  fn machine_authored_turns_do_not_move_the_turn_boundary() {
    let block = format!(
      r#"{{"type":"user","message":{{"content":[{{"type":"text","text":"... {REVIEW_BLOCK_MARKER} COMPLIANCE: PASS."}}]}}}}"#
    );
    let jsonl = format!("{PROMPT}\n{LAUNCH}\n{block}\n");
    let parsed = entries(&jsonl);
    assert_eq!(parsed.iter().rposition(is_prompt), Some(0));
  }

  #[test]
  fn tool_responses_are_not_prompts() {
    let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"ok"}]}}"#;
    let parsed = entries(&format!("{PROMPT}\n{LAUNCH}\n{result}\n"));
    assert_eq!(parsed.iter().rposition(is_prompt), Some(0));
  }

  #[test]
  fn the_latest_verdict_wins_across_channels() {
    let launch_result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"Async agent launched successfully.\nagentId: abc123"}]}}"#;
    let queued_pass = r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<result>COMPLIANCE: PASS</result>\n</task-notification>"}}"#;
    let relaunch = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Task","id":"rev2","input":{"subagent_type":"template-compliance"}}]}}"#;
    let findings = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"rev2","content":[{"type":"text","text":"COMPLIANCE: FINDINGS\nsomething is off"}]}]}}"#;
    let parsed = entries(&format!(
      "{PROMPT}\n{LAUNCH}\n{launch_result}\n{queued_pass}\n{relaunch}\n{findings}\n"
    ));
    let analysis = analyze_entries(&parsed).unwrap();
    assert_eq!(analysis.turn_key(), "0");
    assert!(matches!(
      analysis.verdict,
      Verdict::Findings { ref text } if text.contains("something is off")
    ));
  }

  #[test]
  fn a_task_output_counts_only_when_it_waited_on_the_review() {
    let launch_result = r#"{"type":"user","toolUseResult":{"agentId":"abc123"},"message":{"content":[{"type":"tool_result","tool_use_id":"rev1","content":"launched"}]}}"#;
    let wait_other = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TaskOutput","id":"out1","input":{"task_id":"zzz999"}}]}}"#;
    let other_pass = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"out1","content":"COMPLIANCE: PASS"}]}}"#;
    let parsed = entries(&format!(
      "{PROMPT}\n{LAUNCH}\n{launch_result}\n{wait_other}\n{other_pass}\n"
    ));
    assert!(matches!(
      analyze_entries(&parsed).unwrap().verdict,
      Verdict::Absent
    ));
    let wait_review = wait_other.replace("zzz999", "abc123");
    let parsed = entries(&format!(
      "{PROMPT}\n{LAUNCH}\n{launch_result}\n{wait_review}\n{other_pass}\n"
    ));
    assert!(matches!(analyze_entries(&parsed).unwrap().verdict, Verdict::Pass));
  }

  #[test]
  fn an_unparsable_line_is_skipped_and_does_not_shift_indices() {
    let parsed = entries(&format!("{PROMPT}\nnot json at all\n{LAUNCH}\n"));
    assert_eq!(parsed.len(), 2);
    assert_eq!(analyze_entries(&parsed).unwrap().turn_key(), "0");
  }
}
