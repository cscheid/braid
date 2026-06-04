//! JSONL import: accepts both beads `issues.jsonl` lines and braid's own
//! export format, converting either into schema [`Issue`]s.
//!
//! Mapping notes (beads → braid):
//! - `"completed"` status is an alias for `closed`; other unknown statuses
//!   round-trip via `Status::Other`
//! - `dependencies` may be a beads array (with redundant `issue_id`,
//!   `metadata`, `thread_id` — dropped) or a braid map; either becomes the
//!   keyed map
//! - `comments` may be a beads array with **integer ids** (a CRDT hazard —
//!   replaced with fresh `c-` ids) or a braid map with string ids (kept,
//!   so braid → braid round-trips exactly)
//! - beads-only fields (`source_repo`, `compaction_level`, `owner`,
//!   `estimated_minutes`, tombstone/ephemeral/pinned/template machinery,
//!   `agent_context`, …) are dropped
//! - missing timestamps default to import time; missing `created_by`
//!   defaults to "unknown"

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use braid_core::id::new_comment_id;
use braid_core::schema::{Comment, Dependency, DependencyType, Issue, IssueType, Status};
use braid_core::time::now_rfc3339;
use serde::Deserialize;

#[derive(Deserialize)]
struct RawIssue {
    id: String,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    design: Option<String>,
    #[serde(default)]
    acceptance_criteria: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    status: String,
    #[serde(default = "default_priority")]
    priority: i64,
    #[serde(default = "default_issue_type")]
    issue_type: String,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    closed_at: Option<String>,
    #[serde(default)]
    close_reason: Option<String>,
    #[serde(default)]
    external_ref: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    dependencies: Option<RawDeps>,
    #[serde(default)]
    comments: Option<RawComments>,
    // every other field (source_repo, compaction_level, owner, ...) is
    // silently ignored
}

fn default_priority() -> i64 {
    2
}
fn default_issue_type() -> String {
    "task".to_string()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawDeps {
    List(Vec<RawDep>),
    Map(BTreeMap<String, RawDep>),
}

#[derive(Deserialize)]
struct RawDep {
    depends_on_id: String,
    #[serde(rename = "type")]
    dep_type: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawComments {
    List(Vec<RawComment>),
    Map(BTreeMap<String, RawComment>),
}

#[derive(Deserialize)]
struct RawComment {
    /// beads uses integers, braid uses `c-` strings
    #[serde(default)]
    id: Option<serde_json::Value>,
    #[serde(default)]
    author: Option<String>,
    text: String,
    #[serde(default)]
    created_at: Option<String>,
}

fn convert_status(s: &str) -> Status {
    match s {
        // beads alias seen in the wild
        "completed" => Status::Closed,
        other => Status::from(other),
    }
}

fn convert_comment(raw: RawComment, now: &str) -> Comment {
    let id = match &raw.id {
        // braid-style string id: keep it (exact round-trips)
        Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
        // beads-style integer (or missing): fresh collision-free id
        _ => new_comment_id(),
    };
    Comment {
        id,
        author: raw.author.unwrap_or_else(|| "unknown".to_string()),
        created_at: raw.created_at.unwrap_or_else(|| now.to_string()),
        text: raw.text,
    }
}

fn convert(raw: RawIssue, now: &str) -> Issue {
    let dependencies: BTreeMap<String, Dependency> = match raw.dependencies {
        None => BTreeMap::new(),
        Some(RawDeps::List(list)) => list
            .into_iter()
            .map(|d| {
                let dep = Dependency {
                    depends_on_id: d.depends_on_id,
                    dep_type: DependencyType::from(d.dep_type.as_str()),
                    created_at: d.created_at.unwrap_or_else(|| now.to_string()),
                    created_by: d.created_by.unwrap_or_else(|| "unknown".to_string()),
                };
                (dep.key(), dep)
            })
            .collect(),
        Some(RawDeps::Map(map)) => map
            .into_values()
            .map(|d| {
                let dep = Dependency {
                    depends_on_id: d.depends_on_id,
                    dep_type: DependencyType::from(d.dep_type.as_str()),
                    created_at: d.created_at.unwrap_or_else(|| now.to_string()),
                    created_by: d.created_by.unwrap_or_else(|| "unknown".to_string()),
                };
                // re-key canonically rather than trusting the input keys
                (dep.key(), dep)
            })
            .collect(),
    };

    let comments: BTreeMap<String, Comment> = match raw.comments {
        None => BTreeMap::new(),
        Some(RawComments::List(list)) => list
            .into_iter()
            .map(|c| {
                let c = convert_comment(c, now);
                (c.id.clone(), c)
            })
            .collect(),
        Some(RawComments::Map(map)) => map
            .into_values()
            .map(|c| {
                let c = convert_comment(c, now);
                (c.id.clone(), c)
            })
            .collect(),
    };

    Issue {
        id: raw.id,
        title: raw.title,
        description: raw.description,
        design: raw.design,
        acceptance_criteria: raw.acceptance_criteria,
        notes: raw.notes,
        status: convert_status(&raw.status),
        priority: raw.priority,
        issue_type: IssueType::from(raw.issue_type.as_str()),
        assignee: raw.assignee,
        created_at: raw.created_at.unwrap_or_else(|| now.to_string()),
        created_by: raw.created_by.unwrap_or_else(|| "unknown".to_string()),
        updated_at: raw.updated_at.unwrap_or_else(|| now.to_string()),
        closed_at: raw.closed_at,
        close_reason: raw.close_reason,
        defer_until: None, // wired to RawIssue in the import phase (plan phase 3)
        external_ref: raw.external_ref,
        labels: raw.labels.into_iter().collect(),
        dependencies,
        comments,
    }
}

/// Strand ids appear as map keys and inside dependency keys
/// (`<id>:<type>`), so the export contract (docs/schemas/strand.schema.json)
/// forbids colons and whitespace. Import enforces this: anything import
/// accepts, export will later emit.
fn validate_id(id: &str, what: &str) -> Result<()> {
    if id.is_empty() {
        anyhow::bail!("{what} id is empty");
    }
    if id.contains(':') || id.contains(char::is_whitespace) {
        anyhow::bail!(
            "{what} id {id:?} contains a colon or whitespace, which the braid \
             JSONL contract forbids (docs/schemas/strand.schema.json)"
        );
    }
    Ok(())
}

/// Parse JSONL text (beads or braid format) into issues. Fails on the
/// first malformed line, naming its 1-based line number.
pub fn parse_jsonl(text: &str) -> Result<Vec<Issue>> {
    let now = now_rfc3339();
    let mut issues = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let context = || format!("line {}: not a valid issue record", idx + 1);
        let raw: RawIssue = serde_json::from_str(line).with_context(context)?;
        let issue = convert(raw, &now);
        (|| -> Result<()> {
            validate_id(&issue.id, "strand")?;
            for dep in issue.dependencies.values() {
                validate_id(&dep.depends_on_id, "dependency target")?;
            }
            for comment in issue.comments.values() {
                validate_id(&comment.id, "comment")?;
            }
            Ok(())
        })()
        .with_context(|| format!("line {}", idx + 1))?;
        issues.push(issue);
    }
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_maps_to_closed_and_unknown_round_trips() {
        assert_eq!(convert_status("completed"), Status::Closed);
        assert_eq!(convert_status("closed"), Status::Closed);
        assert_eq!(convert_status("paused"), Status::Other("paused".into()));
    }

    #[test]
    fn integer_comment_ids_are_replaced_string_ids_kept() {
        let now = "2026-06-03T00:00:00.000000Z";
        let from_int = convert_comment(
            RawComment {
                id: Some(serde_json::json!(15)),
                author: Some("x".into()),
                text: "t".into(),
                created_at: None,
            },
            now,
        );
        assert!(from_int.id.starts_with("c-"));

        let from_str = convert_comment(
            RawComment {
                id: Some(serde_json::json!("c-abc12345")),
                author: None,
                text: "t".into(),
                created_at: None,
            },
            now,
        );
        assert_eq!(from_str.id, "c-abc12345");
        assert_eq!(from_str.author, "unknown");
    }

    #[test]
    fn blank_lines_are_skipped_and_errors_name_the_line() {
        let ok = parse_jsonl("\n\n").unwrap();
        assert!(ok.is_empty());
        let err = parse_jsonl("{\"id\":\"a\",\"title\":\"t\",\"status\":\"open\"}\n\nnope\n")
            .unwrap_err();
        assert!(err.to_string().contains("line 3"), "got: {err}");
    }
}
