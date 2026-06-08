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
//!   `estimated_minutes`, ephemeral/pinned/template machinery,
//!   `agent_context`, …) are dropped
//! - beads **tombstones** (soft-deleted records: `status:"tombstone"` or a
//!   `deleted_at`/`delete_reason`/`deleted_by` marker) are recognized and
//!   skipped entirely — never converted — and counted separately (see
//!   [`is_tombstone`] / [`ParseOutcome`])
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
    defer_until: Option<String>,
    #[serde(default)]
    external_ref: Option<String>,
    // beads soft-delete markers. braid has no tombstone concept; their
    // presence means the record is deleted and must be skipped on import
    // (see `is_tombstone`). Captured only for detection, never converted.
    #[serde(default)]
    deleted_at: Option<String>,
    #[serde(default)]
    delete_reason: Option<String>,
    #[serde(default)]
    deleted_by: Option<String>,
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

/// Is this raw record a beads tombstone (soft-deleted) that import should
/// skip entirely? braid has no tombstone status, so a kept tombstone would
/// hydrate as `Status::Other("tombstone")` — neither active nor terminal,
/// and pure noise in `braid list`.
///
/// Detection is deliberately conservative: a record is a tombstone iff its
/// status is literally `"tombstone"`, **or** it carries a non-empty beads
/// deletion marker (`deleted_at` / `delete_reason` / `deleted_by`). beads
/// writes `status:"closed"` *and* `deleted_at` on a delete, so the markers
/// — not the status — are authoritative. An empty-string marker does not
/// count (we only skip unambiguous deletions).
fn is_tombstone(raw: &RawIssue) -> bool {
    let has = |f: &Option<String>| f.as_deref().is_some_and(|s| !s.is_empty());
    raw.status == "tombstone"
        || has(&raw.deleted_at)
        || has(&raw.delete_reason)
        || has(&raw.deleted_by)
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
        defer_until: raw.defer_until,
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

/// Result of parsing JSONL: the issues to upsert, plus the count of beads
/// tombstones that were recognized and skipped (never converted).
#[derive(Debug)]
pub struct ParseOutcome {
    pub issues: Vec<Issue>,
    pub skipped: usize,
}

/// Parse JSONL text (beads or braid format) into issues. Fails on the
/// first malformed line, naming its 1-based line number. beads tombstones
/// are recognized and skipped (see [`is_tombstone`]); their count is
/// reported separately and they are never validated or converted.
pub fn parse_jsonl(text: &str) -> Result<ParseOutcome> {
    let now = now_rfc3339();
    let mut issues = Vec::new();
    let mut skipped = 0usize;
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let context = || format!("line {}: not a valid issue record", idx + 1);
        let raw: RawIssue = serde_json::from_str(line).with_context(context)?;
        if is_tombstone(&raw) {
            skipped += 1;
            continue;
        }
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
    Ok(ParseOutcome { issues, skipped })
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
        assert!(ok.issues.is_empty());
        assert_eq!(ok.skipped, 0);
        let err = parse_jsonl("{\"id\":\"a\",\"title\":\"t\",\"status\":\"open\"}\n\nnope\n")
            .unwrap_err();
        assert!(err.to_string().contains("line 3"), "got: {err}");
    }

    #[test]
    fn tombstone_status_is_skipped() {
        let jsonl = concat!(
            r#"{"id":"bd-live","title":"live","status":"open"}"#,
            "\n",
            r#"{"id":"bd-dead","title":"dead","status":"tombstone","deleted_at":"2026-05-01T00:00:00Z","deleted_by":"x","delete_reason":"dup"}"#,
            "\n",
        );
        let out = parse_jsonl(jsonl).unwrap();
        assert_eq!(out.issues.len(), 1);
        assert_eq!(out.issues[0].id, "bd-live");
        assert_eq!(out.skipped, 1);
    }

    #[test]
    fn closed_with_deleted_at_is_skipped() {
        // beads writes status:closed *and* deleted_at on a delete; treat the
        // presence of the deletion marker as authoritative and skip.
        let out = parse_jsonl(
            r#"{"id":"bd-x","title":"t","status":"closed","deleted_at":"2026-05-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert!(out.issues.is_empty());
        assert_eq!(out.skipped, 1);
    }

    #[test]
    fn delete_reason_or_deleted_by_alone_is_skipped() {
        let by_reason =
            parse_jsonl(r#"{"id":"a","title":"t","status":"open","delete_reason":"obsolete"}"#)
                .unwrap();
        assert!(by_reason.issues.is_empty());
        assert_eq!(by_reason.skipped, 1);

        let by_who =
            parse_jsonl(r#"{"id":"b","title":"t","status":"open","deleted_by":"cscheid"}"#)
                .unwrap();
        assert!(by_who.issues.is_empty());
        assert_eq!(by_who.skipped, 1);
    }

    #[test]
    fn plain_closed_strand_is_imported_not_skipped() {
        // closed but none of the beads deletion fields → a normal terminal
        // strand, conservatively kept.
        let out = parse_jsonl(r#"{"id":"bd-done","title":"t","status":"closed"}"#).unwrap();
        assert_eq!(out.issues.len(), 1);
        assert_eq!(out.skipped, 0);
        assert_eq!(out.issues[0].status, Status::Closed);
    }

    #[test]
    fn empty_deletion_fields_do_not_trigger_skip() {
        // an empty string is not a real deletion marker; be conservative.
        let out = parse_jsonl(
            r#"{"id":"a","title":"t","status":"open","deleted_at":"","delete_reason":"","deleted_by":""}"#,
        )
        .unwrap();
        assert_eq!(out.issues.len(), 1);
        assert_eq!(out.skipped, 0);
    }

    #[test]
    fn clean_file_skips_nothing() {
        let jsonl = concat!(
            r#"{"id":"a","title":"t","status":"open"}"#,
            "\n",
            r#"{"id":"b","title":"t","status":"in_progress"}"#,
            "\n",
        );
        let out = parse_jsonl(jsonl).unwrap();
        assert_eq!(out.issues.len(), 2);
        assert_eq!(out.skipped, 0);
    }
}
