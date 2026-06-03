//! Round-trip tests: hydrated schema → automerge document → hydrated schema.
//!
//! These are the Phase 0 spike tests that validate the document schema
//! design before any CLI exists.

use std::collections::{BTreeMap, BTreeSet};

use automerge::{Automerge, ReadDoc};
use braid_core::amdoc::{
    delete_issue, hydrate, init_tracker, reconcile_issue, reconcile_tracker,
};
use braid_core::schema::*;

fn meta() -> TrackerMetadata {
    TrackerMetadata {
        schema_version: SCHEMA_VERSION,
        name: "test-tracker".into(),
        id_prefix: "br".into(),
        created_at: "2026-06-03T10:00:00.000000Z".into(),
    }
}

fn minimal_issue(id: &str) -> Issue {
    Issue {
        id: id.into(),
        title: format!("issue {id}"),
        description: None,
        design: None,
        acceptance_criteria: None,
        notes: None,
        status: Status::Open,
        priority: 2,
        issue_type: IssueType::Task,
        assignee: None,
        created_at: "2026-06-03T10:00:00.000000Z".into(),
        created_by: "cscheid".into(),
        updated_at: "2026-06-03T10:00:00.000000Z".into(),
        closed_at: None,
        close_reason: None,
        external_ref: None,
        labels: BTreeSet::new(),
        dependencies: BTreeMap::new(),
        comments: BTreeMap::new(),
    }
}

fn full_issue(id: &str) -> Issue {
    let dep = Dependency {
        depends_on_id: "br-parent1".into(),
        dep_type: DependencyType::ParentChild,
        created_at: "2026-06-03T10:01:00.000000Z".into(),
        created_by: "cscheid".into(),
    };
    let dep2 = Dependency {
        depends_on_id: "br-origin9".into(),
        dep_type: DependencyType::DiscoveredFrom,
        created_at: "2026-06-03T10:02:00.000000Z".into(),
        created_by: "agent-1".into(),
    };
    let comment = Comment {
        id: "c-9f3k2a".into(),
        author: "agent-1".into(),
        created_at: "2026-06-03T10:03:00.000000Z".into(),
        text: "Multi-line comment.\nWith unicode: héllo → wörld 🎉".into(),
    };
    Issue {
        id: id.into(),
        title: "Full issue with every field populated".into(),
        description: Some("A description.\n\nWith paragraphs and `code`.".into()),
        design: Some("Design notes here.".into()),
        acceptance_criteria: Some("- [ ] it works\n- [ ] tests pass".into()),
        notes: Some("Some notes.".into()),
        status: Status::InProgress,
        priority: 1,
        issue_type: IssueType::Feature,
        assignee: Some("agent-2".into()),
        created_at: "2026-06-03T10:00:00.000000Z".into(),
        created_by: "cscheid".into(),
        updated_at: "2026-06-03T11:00:00.000000Z".into(),
        closed_at: Some("2026-06-03T12:00:00.000000Z".into()),
        close_reason: Some("done, verified".into()),
        external_ref: Some("https://github.com/example/repo/issues/42".into()),
        labels: BTreeSet::from(["cargo".to_string(), "deps".to_string()]),
        dependencies: BTreeMap::from([(dep.key(), dep), (dep2.key(), dep2)]),
        comments: BTreeMap::from([(comment.id.clone(), comment)]),
    }
}

fn tracker_with(issues: Vec<Issue>) -> TrackerDoc {
    TrackerDoc {
        metadata: meta(),
        issues: issues.into_iter().map(|i| (i.id.clone(), i)).collect(),
    }
}

/// Write a TrackerDoc into a fresh automerge document.
fn materialize(tracker: &TrackerDoc) -> Automerge {
    let mut doc = Automerge::new();
    doc.transact(|tx| {
        init_tracker(tx, &tracker.metadata)?;
        for issue in tracker.issues.values() {
            reconcile_issue(tx, issue)?;
        }
        Ok::<_, braid_core::amdoc::ReconcileError>(())
    })
    .map_err(|f| f.error)
    .unwrap();
    doc
}

#[test]
fn empty_tracker_round_trips() {
    let tracker = tracker_with(vec![]);
    let doc = materialize(&tracker);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back, tracker);
}

#[test]
fn minimal_issue_round_trips() {
    let tracker = tracker_with(vec![minimal_issue("br-min001")]);
    let doc = materialize(&tracker);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back, tracker);
}

#[test]
fn full_issue_round_trips() {
    let tracker = tracker_with(vec![full_issue("br-full01"), minimal_issue("br-min001")]);
    let doc = materialize(&tracker);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back, tracker);
}

#[test]
fn unknown_enum_values_round_trip() {
    let mut issue = minimal_issue("br-odd001");
    issue.status = Status::Other("paused".into());
    issue.issue_type = IssueType::Other("rfc".into());
    let dep = Dependency {
        depends_on_id: "br-x".into(),
        dep_type: DependencyType::Other("mystery-edge".into()),
        created_at: "2026-06-03T10:00:00.000000Z".into(),
        created_by: "cscheid".into(),
    };
    issue.dependencies.insert(dep.key(), dep);

    let tracker = tracker_with(vec![issue]);
    let doc = materialize(&tracker);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back, tracker);
}

#[test]
fn reconcile_is_idempotent() {
    let tracker = tracker_with(vec![full_issue("br-full01")]);
    let mut doc = materialize(&tracker);
    let heads_before = doc.get_heads();

    // Reconciling identical state must generate no operations at all.
    doc.transact(|tx| {
        init_tracker(tx, &tracker.metadata)?;
        for issue in tracker.issues.values() {
            reconcile_issue(tx, issue)?;
        }
        Ok::<_, braid_core::amdoc::ReconcileError>(())
    })
    .map_err(|f| f.error)
    .unwrap();

    assert_eq!(doc.get_heads(), heads_before, "idempotent reconcile must not create a change");
}

#[test]
fn text_field_is_updated_in_place_not_replaced() {
    let mut issue = full_issue("br-full01");
    let tracker = tracker_with(vec![issue.clone()]);
    let mut doc = materialize(&tracker);

    // Locate the automerge Text object backing `description`.
    let issues_obj = doc.get(automerge::ROOT, "issues").unwrap().unwrap().1;
    let issue_obj = doc.get(&issues_obj, "br-full01").unwrap().unwrap().1;
    let desc_obj_before = doc.get(&issue_obj, "description").unwrap().unwrap().1;

    // Mutate the description and reconcile.
    issue.description = Some("A description.\n\nWith paragraphs, edits, and `code`.".into());
    doc.transact(|tx| reconcile_issue(tx, &issue))
        .map_err(|f| f.error)
        .unwrap();

    let desc_obj_after = doc.get(&issue_obj, "description").unwrap().unwrap().1;
    assert_eq!(
        desc_obj_before, desc_obj_after,
        "description must be spliced in place, not deleted and recreated"
    );
    assert_eq!(
        doc.text(&desc_obj_after).unwrap(),
        "A description.\n\nWith paragraphs, edits, and `code`."
    );
}

#[test]
fn optional_text_field_can_be_added_and_removed() {
    let mut issue = minimal_issue("br-min001");
    let mut doc = materialize(&tracker_with(vec![issue.clone()]));

    // Add notes.
    issue.notes = Some("now with notes".into());
    doc.transact(|tx| reconcile_issue(tx, &issue))
        .map_err(|f| f.error)
        .unwrap();
    let back = hydrate(&doc).unwrap();
    assert_eq!(back.issues["br-min001"].notes.as_deref(), Some("now with notes"));

    // Remove them again.
    issue.notes = None;
    doc.transact(|tx| reconcile_issue(tx, &issue))
        .map_err(|f| f.error)
        .unwrap();
    let back = hydrate(&doc).unwrap();
    assert_eq!(back.issues["br-min001"].notes, None);
}

#[test]
fn labels_and_collections_reconcile_to_match() {
    let mut issue = full_issue("br-full01");
    let mut doc = materialize(&tracker_with(vec![issue.clone()]));

    // Change labels (add one, drop one), drop a dependency, drop the comment.
    issue.labels = BTreeSet::from(["deps".to_string(), "urgent".to_string()]);
    let kept_dep_key = "br-parent1:parent-child".to_string();
    let kept = issue.dependencies[&kept_dep_key].clone();
    issue.dependencies = BTreeMap::from([(kept_dep_key, kept)]);
    issue.comments.clear();

    doc.transact(|tx| reconcile_issue(tx, &issue))
        .map_err(|f| f.error)
        .unwrap();

    let back = hydrate(&doc).unwrap();
    assert_eq!(back.issues["br-full01"], issue);
}

#[test]
fn delete_issue_removes_it() {
    let tracker = tracker_with(vec![minimal_issue("br-a"), minimal_issue("br-b")]);
    let mut doc = materialize(&tracker);

    let was_present = doc
        .transact(|tx| delete_issue(tx, "br-a"))
        .map_err(|f| f.error)
        .unwrap()
        .result;
    assert!(was_present);

    let back = hydrate(&doc).unwrap();
    assert!(!back.issues.contains_key("br-a"));
    assert!(back.issues.contains_key("br-b"));

    let was_present = doc
        .transact(|tx| delete_issue(tx, "br-a"))
        .map_err(|f| f.error)
        .unwrap()
        .result;
    assert!(!was_present, "second delete reports absence");
}

#[test]
fn reconcile_tracker_is_full_state_sync() {
    let tracker = tracker_with(vec![minimal_issue("br-a"), minimal_issue("br-b")]);
    let mut doc = materialize(&tracker);

    // Desired state drops br-b and adds br-c.
    let desired = tracker_with(vec![minimal_issue("br-a"), minimal_issue("br-c")]);
    doc.transact(|tx| reconcile_tracker(tx, &desired))
        .map_err(|f| f.error)
        .unwrap();

    let back = hydrate(&doc).unwrap();
    assert_eq!(back, desired);
}

#[test]
fn hydrate_uninitialized_doc_fails() {
    let doc = Automerge::new();
    assert!(hydrate(&doc).is_err());
}

#[test]
fn hydrate_rejects_future_schema_version() {
    let mut bad_meta = meta();
    bad_meta.schema_version = SCHEMA_VERSION + 1;
    let mut doc = Automerge::new();
    doc.transact(|tx| init_tracker(tx, &bad_meta))
        .map_err(|f| f.error)
        .unwrap();
    let err = hydrate(&doc).unwrap_err();
    assert!(
        matches!(
            err,
            braid_core::amdoc::HydrateError::UnsupportedSchemaVersion { found, .. } if found == SCHEMA_VERSION + 1
        ),
        "got: {err:?}"
    );
}

#[test]
fn save_load_preserves_tracker() {
    // The document survives automerge's binary serialization (what samod
    // storage persists).
    let tracker = tracker_with(vec![full_issue("br-full01")]);
    let doc = materialize(&tracker);
    let bytes = doc.save();
    let loaded = Automerge::load(&bytes).unwrap();
    assert_eq!(hydrate(&loaded).unwrap(), tracker);
}
