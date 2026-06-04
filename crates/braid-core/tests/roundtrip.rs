//! Round-trip tests: hydrated schema → automerge document → hydrated schema.
//!
//! These are the Phase 0 spike tests that validate the document schema
//! design before any CLI exists.

use std::collections::{BTreeMap, BTreeSet};

use automerge::{Automerge, ReadDoc};
use braid_core::amdoc::{
    delete_issue, hydrate, init_skein, reconcile_issue, reconcile_skein,
};
use braid_core::schema::*;

fn meta() -> SkeinMetadata {
    SkeinMetadata {
        schema_version: SCHEMA_VERSION,
        name: "test-skein".into(),
        id_prefix: "br".into(),
        created_at: "2026-06-03T10:00:00.000000Z".into(),
        rotated_at: None,
        rotated_to: None,
    }
}

#[test]
fn rotation_metadata_round_trips() {
    let mut m = meta();
    m.rotated_at = Some("2026-06-04T12:00:00.000000Z".into());
    m.rotated_to = Some("4UfaPGzzySmw7Y1MR1VVXbfp4fgx".into());
    let skein = Skein { metadata: m.clone(), issues: Default::default() };
    let doc = materialize(&skein);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back.metadata, m);

    // revoke-style marker: rotated_at without rotated_to
    let mut m2 = meta();
    m2.rotated_at = Some("2026-06-04T12:00:00.000000Z".into());
    let skein2 = Skein { metadata: m2.clone(), issues: Default::default() };
    let back2 = hydrate(&materialize(&skein2)).unwrap();
    assert_eq!(back2.metadata, m2);
}

#[test]
fn marking_rotation_on_existing_doc_is_an_update_and_idempotent() {
    let skein = skein_with(vec![minimal_issue("br-a")]);
    let mut doc = materialize(&skein);

    let mut rotated = skein.metadata.clone();
    rotated.rotated_at = Some("2026-06-04T12:00:00.000000Z".into());
    rotated.rotated_to = Some("4UfaPGzzySmw7Y1MR1VVXbfp4fgx".into());

    doc.transact(|tx| init_skein(tx, &rotated)).map_err(|f| f.error).unwrap();
    let back = hydrate(&doc).unwrap();
    assert_eq!(back.metadata, rotated);
    assert!(back.issues.contains_key("br-a"), "marking rotation must not touch strands");

    // idempotent: re-asserting identical metadata creates no change
    let heads = doc.get_heads();
    doc.transact(|tx| init_skein(tx, &rotated)).map_err(|f| f.error).unwrap();
    assert_eq!(doc.get_heads(), heads);
}

#[test]
fn hydrate_metadata_reads_only_metadata() {
    let mut m = meta();
    m.rotated_at = Some("2026-06-04T12:00:00.000000Z".into());
    let skein = Skein { metadata: m.clone(), issues: Default::default() };
    let doc = materialize(&skein);
    let got = braid_core::amdoc::hydrate_metadata(&doc).unwrap();
    assert_eq!(got, m);
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

fn skein_with(issues: Vec<Issue>) -> Skein {
    Skein {
        metadata: meta(),
        issues: issues.into_iter().map(|i| (i.id.clone(), i)).collect(),
    }
}

/// Write a Skein into a fresh automerge document.
fn materialize(skein: &Skein) -> Automerge {
    let mut doc = Automerge::new();
    doc.transact(|tx| {
        init_skein(tx, &skein.metadata)?;
        for issue in skein.issues.values() {
            reconcile_issue(tx, issue)?;
        }
        Ok::<_, braid_core::amdoc::ReconcileError>(())
    })
    .map_err(|f| f.error)
    .unwrap();
    doc
}

#[test]
fn empty_skein_round_trips() {
    let skein = skein_with(vec![]);
    let doc = materialize(&skein);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back, skein);
}

#[test]
fn minimal_issue_round_trips() {
    let skein = skein_with(vec![minimal_issue("br-min001")]);
    let doc = materialize(&skein);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back, skein);
}

#[test]
fn full_issue_round_trips() {
    let skein = skein_with(vec![full_issue("br-full01"), minimal_issue("br-min001")]);
    let doc = materialize(&skein);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back, skein);
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

    let skein = skein_with(vec![issue]);
    let doc = materialize(&skein);
    let back = hydrate(&doc).unwrap();
    assert_eq!(back, skein);
}

#[test]
fn reconcile_is_idempotent() {
    let skein = skein_with(vec![full_issue("br-full01")]);
    let mut doc = materialize(&skein);
    let heads_before = doc.get_heads();

    // Reconciling identical state must generate no operations at all.
    doc.transact(|tx| {
        init_skein(tx, &skein.metadata)?;
        for issue in skein.issues.values() {
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
    let skein = skein_with(vec![issue.clone()]);
    let mut doc = materialize(&skein);

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
    let mut doc = materialize(&skein_with(vec![issue.clone()]));

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
    let mut doc = materialize(&skein_with(vec![issue.clone()]));

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
    let skein = skein_with(vec![minimal_issue("br-a"), minimal_issue("br-b")]);
    let mut doc = materialize(&skein);

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
fn reconcile_skein_is_full_state_sync() {
    let skein = skein_with(vec![minimal_issue("br-a"), minimal_issue("br-b")]);
    let mut doc = materialize(&skein);

    // Desired state drops br-b and adds br-c.
    let desired = skein_with(vec![minimal_issue("br-a"), minimal_issue("br-c")]);
    doc.transact(|tx| reconcile_skein(tx, &desired))
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
    doc.transact(|tx| init_skein(tx, &bad_meta))
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
fn save_load_preserves_skein() {
    // The document survives automerge's binary serialization (what samod
    // storage persists).
    let skein = skein_with(vec![full_issue("br-full01")]);
    let doc = materialize(&skein);
    let bytes = doc.save();
    let loaded = Automerge::load(&bytes).unwrap();
    assert_eq!(hydrate(&loaded).unwrap(), skein);
}
