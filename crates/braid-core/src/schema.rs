//! The braid skein document schema, in hydrated (plain Rust) form.
//!
//! These types mirror the automerge document shape one-to-one (see the
//! design doc's "Document schema" section). Prose fields that are automerge
//! `Text` in the document are plain `String`s here; conversion happens in
//! [`crate::amdoc`].
//!
//! Merge-semantics summary:
//! - scalar fields: last-writer-wins
//! - prose fields (`description`, `design`, `acceptance_criteria`, `notes`,
//!   comment `text`): automerge `Text`, concurrent edits interleave
//! - `labels`, `dependencies`, `comments`, and the top-level `issues`
//!   collection: maps keyed by collision-free ids, so concurrent inserts of
//!   distinct keys both survive and concurrent inserts of the same logical
//!   key converge to one entry

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Current document schema version. Documents with a different
/// `metadata.schema_version` are refused (the version field is the
/// compatibility gate that lets us evolve the shape later).
pub const SCHEMA_VERSION: i64 = 1;

/// A whole skein: one automerge document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skein {
    pub metadata: SkeinMetadata,
    /// Keyed by issue id; each value's `id` field duplicates its key so
    /// issue objects are self-contained.
    pub issues: BTreeMap<String, Issue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkeinMetadata {
    pub schema_version: i64,
    /// Display name for the skein.
    pub name: String,
    /// Prefix for generated issue ids (without the trailing dash), e.g. "br".
    pub id_prefix: String,
    /// RFC 3339; writer-set (see design decision D10).
    pub created_at: String,
    /// When set, this skein has been rotated: a successor document holds
    /// the live state and this one must no longer be written to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotated_at: Option<String>,
    /// Successor document id, present only after a *compact* rotation
    /// (`braid rotate`); a revocation rotation (`--revoke`) deliberately
    /// omits it. **This value is a bearer capability** — code must never
    /// print it; `braid rotate --adopt` moves it directly into config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotated_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_criteria: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub status: Status,
    /// 0 (critical) .. 4 (backlog).
    pub priority: i64,
    pub issue_type: IssueType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub created_at: String,
    pub created_by: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
    /// Map-as-set in the document; a sorted set here.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub labels: BTreeSet<String>,
    /// Keyed by `"<depends_on_id>:<type>"` (see [`Dependency::key`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, Dependency>,
    /// Keyed by comment id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub comments: BTreeMap<String, Comment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub depends_on_id: String,
    #[serde(rename = "type")]
    pub dep_type: DependencyType,
    pub created_at: String,
    pub created_by: String,
}

impl Dependency {
    /// The map key under which this edge lives: `"<depends_on_id>:<type>"`.
    /// Issue ids never contain `:`, so the key is unambiguous.
    pub fn key(&self) -> String {
        format!("{}:{}", self.depends_on_id, self.dep_type.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub created_at: String,
    pub text: String,
}

/// Issue status. Unknown strings hydrate as [`Status::Other`] so an older
/// braid never fails to read (or silently destroys) a value written by a
/// newer one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Open,
    InProgress,
    Blocked,
    Deferred,
    Closed,
    Other(String),
}

impl Status {
    pub fn as_str(&self) -> &str {
        match self {
            Status::Open => "open",
            Status::InProgress => "in_progress",
            Status::Blocked => "blocked",
            Status::Deferred => "deferred",
            Status::Closed => "closed",
            Status::Other(s) => s,
        }
    }

    /// Statuses under which an issue can appear in ready-work listings.
    pub fn is_active(&self) -> bool {
        matches!(self, Status::Open | Status::InProgress)
    }

    /// Statuses that stop an issue from blocking its dependents.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Status::Closed)
    }
}

impl From<&str> for Status {
    fn from(s: &str) -> Self {
        match s {
            "open" => Status::Open,
            "in_progress" => Status::InProgress,
            "blocked" => Status::Blocked,
            "deferred" => Status::Deferred,
            "closed" => Status::Closed,
            other => Status::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueType {
    Task,
    Bug,
    Feature,
    Epic,
    Chore,
    Docs,
    Question,
    Other(String),
}

impl IssueType {
    pub fn as_str(&self) -> &str {
        match self {
            IssueType::Task => "task",
            IssueType::Bug => "bug",
            IssueType::Feature => "feature",
            IssueType::Epic => "epic",
            IssueType::Chore => "chore",
            IssueType::Docs => "docs",
            IssueType::Question => "question",
            IssueType::Other(s) => s,
        }
    }
}

impl From<&str> for IssueType {
    fn from(s: &str) -> Self {
        match s {
            "task" => IssueType::Task,
            "bug" => IssueType::Bug,
            "feature" => IssueType::Feature,
            "epic" => IssueType::Epic,
            "chore" => IssueType::Chore,
            "docs" => IssueType::Docs,
            "question" => IssueType::Question,
            other => IssueType::Other(other.to_string()),
        }
    }
}

/// Dependency edge types, carried over from beads. The first four affect
/// ready-work computation ([`DependencyType::is_blocking`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyType {
    Blocks,
    ParentChild,
    ConditionalBlocks,
    WaitsFor,
    Related,
    DiscoveredFrom,
    RepliesTo,
    Duplicates,
    Supersedes,
    CausedBy,
    Other(String),
}

impl DependencyType {
    pub fn as_str(&self) -> &str {
        match self {
            DependencyType::Blocks => "blocks",
            DependencyType::ParentChild => "parent-child",
            DependencyType::ConditionalBlocks => "conditional-blocks",
            DependencyType::WaitsFor => "waits-for",
            DependencyType::Related => "related",
            DependencyType::DiscoveredFrom => "discovered-from",
            DependencyType::RepliesTo => "replies-to",
            DependencyType::Duplicates => "duplicates",
            DependencyType::Supersedes => "supersedes",
            DependencyType::CausedBy => "caused-by",
            DependencyType::Other(s) => s,
        }
    }

    /// Whether an edge of this type makes the issue holding it blocked
    /// while the target is not terminal.
    ///
    /// `parent-child` is deliberately *not* blocking: children are how an
    /// epic progresses, so an open parent must not stop work on them. The
    /// parent-child relationship instead gates the *parent's close* (an
    /// issue with open children should not close) — see
    /// `domain::open_children`.
    pub fn is_blocking(&self) -> bool {
        matches!(
            self,
            DependencyType::Blocks
                | DependencyType::ConditionalBlocks
                | DependencyType::WaitsFor
        )
    }

    /// Whether an edge of this type expresses hierarchy (used for
    /// close-protection and cycle detection, not ready-work).
    pub fn is_hierarchical(&self) -> bool {
        matches!(self, DependencyType::ParentChild)
    }
}

impl From<&str> for DependencyType {
    fn from(s: &str) -> Self {
        match s {
            "blocks" => DependencyType::Blocks,
            "parent-child" => DependencyType::ParentChild,
            "conditional-blocks" => DependencyType::ConditionalBlocks,
            "waits-for" => DependencyType::WaitsFor,
            "related" => DependencyType::Related,
            "discovered-from" => DependencyType::DiscoveredFrom,
            "replies-to" => DependencyType::RepliesTo,
            "duplicates" => DependencyType::Duplicates,
            "supersedes" => DependencyType::Supersedes,
            "caused-by" => DependencyType::CausedBy,
            other => DependencyType::Other(other.to_string()),
        }
    }
}

macro_rules! string_enum_serde {
    ($ty:ty) => {
        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Ok(<$ty>::from(s.as_str()))
            }
        }
    };
}

string_enum_serde!(Status);
string_enum_serde!(IssueType);
string_enum_serde!(DependencyType);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_string_round_trips() {
        for s in ["open", "in_progress", "blocked", "deferred", "closed", "paused"] {
            assert_eq!(Status::from(s).as_str(), s);
        }
        for t in ["task", "bug", "feature", "epic", "chore", "docs", "question", "rfc"] {
            assert_eq!(IssueType::from(t).as_str(), t);
        }
        for d in [
            "blocks",
            "parent-child",
            "conditional-blocks",
            "waits-for",
            "related",
            "discovered-from",
            "replies-to",
            "duplicates",
            "supersedes",
            "caused-by",
            "mystery-edge",
        ] {
            assert_eq!(DependencyType::from(d).as_str(), d);
        }
    }

    #[test]
    fn dependency_key_format() {
        let dep = Dependency {
            depends_on_id: "br-t3ny42".into(),
            dep_type: DependencyType::ParentChild,
            created_at: "2026-06-03T00:00:00Z".into(),
            created_by: "cscheid".into(),
        };
        assert_eq!(dep.key(), "br-t3ny42:parent-child");
    }

    #[test]
    fn blocking_types_are_exactly_the_three() {
        // parent-child is hierarchical, not blocking: an open epic must not
        // stop work on its children. It gates the parent's close instead.
        let blocking: Vec<&str> = [
            "blocks",
            "parent-child",
            "conditional-blocks",
            "waits-for",
            "related",
            "discovered-from",
            "replies-to",
            "duplicates",
            "supersedes",
            "caused-by",
            "mystery-edge",
        ]
        .into_iter()
        .filter(|s| DependencyType::from(*s).is_blocking())
        .collect();
        assert_eq!(blocking, vec!["blocks", "conditional-blocks", "waits-for"]);
        assert!(DependencyType::ParentChild.is_hierarchical());
        assert!(!DependencyType::Blocks.is_hierarchical());
    }

    #[test]
    fn issue_json_shape() {
        let issue = Issue {
            id: "br-abc123".into(),
            title: "A title".into(),
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            status: Status::InProgress,
            priority: 1,
            issue_type: IssueType::Task,
            assignee: None,
            created_at: "2026-06-03T00:00:00Z".into(),
            created_by: "cscheid".into(),
            updated_at: "2026-06-03T00:00:00Z".into(),
            closed_at: None,
            close_reason: None,
            external_ref: None,
            labels: BTreeSet::new(),
            dependencies: BTreeMap::new(),
            comments: BTreeMap::new(),
        };
        let json = serde_json::to_value(&issue).unwrap();
        assert_eq!(json["status"], "in_progress");
        assert_eq!(json["issue_type"], "task");
        // empty collections and None optionals are omitted
        assert!(json.get("labels").is_none());
        assert!(json.get("description").is_none());

        let back: Issue = serde_json::from_value(json).unwrap();
        assert_eq!(back, issue);
    }
}
