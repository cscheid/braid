//! Concurrent-merge semantics tests: the heart of the Phase 0 spike.
//!
//! Each test forks a document into two replicas ("alice" and "bob"),
//! makes concurrent edits through the same reconcile API the CLI will use,
//! merges in both directions, and asserts (a) convergence and (b) the
//! specific semantics the design doc promises:
//!
//! - prose (Text) fields: concurrent edits interleave (D5)
//! - scalar fields: last-writer-wins, one side's value atomically (D5)
//! - same logical dependency edge added twice: converges to one entry (D7)
//! - distinct issues / labels / comments added concurrently: both survive
//!
//! It also *pins* automerge's delete-vs-concurrent-edit behavior so we
//! document reality rather than assumption.

use std::collections::{BTreeMap, BTreeSet};

use automerge::Automerge;
use braid_core::amdoc::{delete_issue, hydrate, init_skein, reconcile_issue};
use braid_core::schema::*;

fn meta() -> SkeinMetadata {
    SkeinMetadata {
        schema_version: SCHEMA_VERSION,
        name: "merge-tests".into(),
        id_prefix: "br".into(),
        created_at: "2026-06-03T10:00:00.000000Z".into(),
            rotated_at: None,
            rotated_to: None,
    }
}

fn issue(id: &str) -> Issue {
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

/// Build a base doc containing `issues`, then fork it into two replicas.
fn base_and_forks(issues: Vec<Issue>) -> (Automerge, Automerge) {
    let mut doc = Automerge::new();
    doc.transact(|tx| {
        init_skein(tx, &meta())?;
        for i in &issues {
            reconcile_issue(tx, i)?;
        }
        Ok::<_, braid_core::amdoc::ReconcileError>(())
    })
    .map_err(|f| f.error)
    .unwrap();
    let alice = doc.fork();
    let bob = doc.fork();
    (alice, bob)
}

fn apply(doc: &mut Automerge, issue: &Issue) {
    doc.transact(|tx| reconcile_issue(tx, issue))
        .map_err(|f| f.error)
        .unwrap();
}

/// Merge in both directions and assert both replicas hydrate identically.
/// Returns the converged state.
fn converge(alice: &mut Automerge, bob: &mut Automerge) -> Skein {
    alice.merge(bob).unwrap();
    bob.merge(alice).unwrap();
    let a = hydrate(alice).unwrap();
    let b = hydrate(bob).unwrap();
    assert_eq!(a, b, "replicas must converge to identical state");
    a
}

#[test]
fn concurrent_text_edits_to_same_description_interleave() {
    let mut base = issue("br-x");
    base.description = Some("The quick brown fox jumps over the lazy dog.".into());
    let (mut alice, mut bob) = base_and_forks(vec![base.clone()]);

    // Alice edits the start; Bob edits the end. Disjoint regions.
    let mut a = base.clone();
    a.description = Some("The slow brown fox jumps over the lazy dog.".into());
    apply(&mut alice, &a);

    let mut b = base.clone();
    b.description = Some("The quick brown fox jumps over the energetic dog.".into());
    apply(&mut bob, &b);

    let merged = converge(&mut alice, &mut bob);
    assert_eq!(
        merged.issues["br-x"].description.as_deref(),
        Some("The slow brown fox jumps over the energetic dog."),
        "both edits must survive the merge (Text interleaving, design D5)"
    );
}

#[test]
fn concurrent_scalar_edits_are_lww_one_side_wins_atomically() {
    let base = issue("br-x");
    let (mut alice, mut bob) = base_and_forks(vec![base.clone()]);

    let mut a = base.clone();
    a.status = Status::InProgress;
    apply(&mut alice, &a);

    let mut b = base.clone();
    b.status = Status::Closed;
    b.closed_at = Some("2026-06-03T12:00:00.000000Z".into());
    apply(&mut bob, &b);

    let merged = converge(&mut alice, &mut bob);
    let status = &merged.issues["br-x"].status;
    assert!(
        *status == Status::InProgress || *status == Status::Closed,
        "LWW must pick one of the written values, got {status:?}"
    );
}

#[test]
fn same_dependency_edge_added_concurrently_converges_to_one_entry() {
    let base = issue("br-x");
    let (mut alice, mut bob) = base_and_forks(vec![base.clone()]);

    let edge = |who: &str, at: &str| Dependency {
        depends_on_id: "br-parent".into(),
        dep_type: DependencyType::ParentChild,
        created_at: at.into(),
        created_by: who.into(),
    };

    let mut a = base.clone();
    let ea = edge("alice", "2026-06-03T11:00:00.000000Z");
    a.dependencies.insert(ea.key(), ea);
    apply(&mut alice, &a);

    let mut b = base.clone();
    let eb = edge("bob", "2026-06-03T11:00:01.000000Z");
    b.dependencies.insert(eb.key(), eb);
    apply(&mut bob, &b);

    let merged = converge(&mut alice, &mut bob);
    let deps = &merged.issues["br-x"].dependencies;
    assert_eq!(deps.len(), 1, "same logical edge must converge to one entry (design D7)");
    let dep = &deps["br-parent:parent-child"];
    assert_eq!(dep.depends_on_id, "br-parent");
    assert_eq!(dep.dep_type, DependencyType::ParentChild);
    // Field values settle by LWW; either writer's metadata is acceptable.
    assert!(dep.created_by == "alice" || dep.created_by == "bob");
}

#[test]
fn distinct_issues_created_concurrently_both_survive() {
    let (mut alice, mut bob) = base_and_forks(vec![]);

    apply(&mut alice, &issue("br-from-alice"));
    apply(&mut bob, &issue("br-from-bob"));

    let merged = converge(&mut alice, &mut bob);
    assert!(merged.issues.contains_key("br-from-alice"));
    assert!(merged.issues.contains_key("br-from-bob"));
}

#[test]
fn concurrent_label_adds_both_survive() {
    let base = issue("br-x");
    let (mut alice, mut bob) = base_and_forks(vec![base.clone()]);

    let mut a = base.clone();
    a.labels.insert("from-alice".into());
    apply(&mut alice, &a);

    let mut b = base.clone();
    b.labels.insert("from-bob".into());
    apply(&mut bob, &b);

    let merged = converge(&mut alice, &mut bob);
    assert_eq!(
        merged.issues["br-x"].labels,
        BTreeSet::from(["from-alice".to_string(), "from-bob".to_string()]),
        "concurrent label adds must both survive (map-as-set, design D7)"
    );
}

#[test]
fn concurrent_comments_both_survive() {
    let base = issue("br-x");
    let (mut alice, mut bob) = base_and_forks(vec![base.clone()]);

    let comment = |id: &str, author: &str| Comment {
        id: id.into(),
        author: author.into(),
        created_at: "2026-06-03T11:00:00.000000Z".into(),
        text: format!("comment from {author}"),
    };

    let mut a = base.clone();
    let ca = comment("c-alice1", "alice");
    a.comments.insert(ca.id.clone(), ca);
    apply(&mut alice, &a);

    let mut b = base.clone();
    let cb = comment("c-bob111", "bob");
    b.comments.insert(cb.id.clone(), cb);
    apply(&mut bob, &b);

    let merged = converge(&mut alice, &mut bob);
    let comments = &merged.issues["br-x"].comments;
    assert_eq!(comments.len(), 2, "concurrent comments must both survive");
    assert_eq!(comments["c-alice1"].author, "alice");
    assert_eq!(comments["c-bob111"].author, "bob");
}

#[test]
fn concurrent_edits_to_different_fields_of_same_issue_both_survive() {
    let base = issue("br-x");
    let (mut alice, mut bob) = base_and_forks(vec![base.clone()]);

    let mut a = base.clone();
    a.title = "retitled by alice".into();
    apply(&mut alice, &a);

    let mut b = base.clone();
    b.priority = 0;
    b.assignee = Some("bob".into());
    apply(&mut bob, &b);

    let merged = converge(&mut alice, &mut bob);
    let got = &merged.issues["br-x"];
    assert_eq!(got.title, "retitled by alice");
    assert_eq!(got.priority, 0);
    assert_eq!(got.assignee.as_deref(), Some("bob"));
}

/// Pin automerge's delete-vs-concurrent-edit semantics.
///
/// Alice deletes the issue (removes the key from the `issues` map); Bob
/// concurrently edits a field *inside* the issue object. In automerge, the
/// map-key deletion wins unless someone concurrently re-puts the key
/// itself; edits inside the removed object do not resurrect it.
///
/// This is the behavior braid documents: **delete wins over concurrent
/// edit**. If this test ever fails after an automerge upgrade, the
/// documented semantics changed and we need to revisit.
#[test]
fn delete_vs_concurrent_edit_pins_automerge_semantics() {
    let base = issue("br-x");
    let (mut alice, mut bob) = base_and_forks(vec![base.clone()]);

    alice
        .transact(|tx| delete_issue(tx, "br-x"))
        .map_err(|f| f.error)
        .unwrap();

    let mut b = base.clone();
    b.title = "edited after alice deleted".into();
    apply(&mut bob, &b);

    let merged = converge(&mut alice, &mut bob);
    assert!(
        !merged.issues.contains_key("br-x"),
        "documented semantics: delete wins over concurrent edit; \
         automerge changed behavior if this fails"
    );
}

/// Two replicas concurrently create *the same* issue id (possible only if
/// id generation collides or an id is chosen manually). Pin the semantics:
/// automerge picks one replica's issue object; the other's fields are lost,
/// but the document still converges and hydrates cleanly. This is why
/// braid's random ids must be long enough to make collision negligible.
#[test]
fn same_issue_id_created_concurrently_converges_one_object_wins() {
    let (mut alice, mut bob) = base_and_forks(vec![]);

    let mut a = issue("br-same");
    a.title = "alice's version".into();
    a.labels.insert("from-alice".into());
    apply(&mut alice, &a);

    let mut b = issue("br-same");
    b.title = "bob's version".into();
    b.labels.insert("from-bob".into());
    apply(&mut bob, &b);

    let merged = converge(&mut alice, &mut bob);
    let got = &merged.issues["br-same"];
    assert!(
        got.title == "alice's version" || got.title == "bob's version",
        "one replica's object wins wholesale"
    );
}

/// Convergence holds through a save/load cycle (i.e. across what samod
/// storage and sync actually transport).
#[test]
fn convergence_survives_save_load() {
    let base = issue("br-x");
    let (mut alice, mut bob) = base_and_forks(vec![base.clone()]);

    let mut a = base.clone();
    a.status = Status::InProgress;
    apply(&mut alice, &a);

    let mut b = base.clone();
    b.notes = Some("bob's notes".into());
    apply(&mut bob, &b);

    // Simulate transport: bob's full doc bytes merged into alice and back.
    let mut bob_loaded = Automerge::load(&bob.save()).unwrap();
    alice.merge(&mut bob_loaded).unwrap();
    let mut alice_loaded = Automerge::load(&alice.save()).unwrap();
    bob.merge(&mut alice_loaded).unwrap();

    let a = hydrate(&alice).unwrap();
    let b = hydrate(&bob).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.issues["br-x"].status, Status::InProgress);
    assert_eq!(a.issues["br-x"].notes.as_deref(), Some("bob's notes"));
}
