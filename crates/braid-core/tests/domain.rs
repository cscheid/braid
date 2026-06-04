//! Tests for ready/blocked computation, close-protection, and cycle
//! detection.

use std::collections::{BTreeMap, BTreeSet};

use braid_core::domain::{
    active_blockers, blocked_issues, dependency_cycles, dependents_of, is_awake, open_children,
    ready_issues,
};
use braid_core::schema::*;

/// The fixed instant "now" for wake computations in these tests.
const NOW: &str = "2026-06-04T12:00:00.000000Z";

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
        created_at: format!("2026-06-03T10:00:00.{:06}Z", id.len()),
        created_by: "t".into(),
        updated_at: "2026-06-03T10:00:00.000000Z".into(),
        closed_at: None,
        close_reason: None,
        defer_until: None,
        external_ref: None,
        labels: BTreeSet::new(),
        dependencies: BTreeMap::new(),
        comments: BTreeMap::new(),
    }
}

fn dep(on: &str, ty: DependencyType) -> Dependency {
    Dependency {
        depends_on_id: on.into(),
        dep_type: ty,
        created_at: "2026-06-03T10:00:00.000000Z".into(),
        created_by: "t".into(),
    }
}

fn with_dep(mut i: Issue, on: &str, ty: DependencyType) -> Issue {
    let d = dep(on, ty);
    i.dependencies.insert(d.key(), d);
    i
}

fn skein(issues: Vec<Issue>) -> Skein {
    Skein {
        metadata: SkeinMetadata {
            schema_version: SCHEMA_VERSION,
            name: "t".into(),
            id_prefix: "br".into(),
            created_at: "2026-06-03T10:00:00.000000Z".into(),
            rotated_at: None,
            rotated_to: None,
        },
        issues: issues.into_iter().map(|i| (i.id.clone(), i)).collect(),
    }
}

fn ids(issues: &[&Issue]) -> Vec<String> {
    issues.iter().map(|i| i.id.clone()).collect()
}

#[test]
fn open_issue_with_no_deps_is_ready() {
    let t = skein(vec![issue("br-a")]);
    assert_eq!(ids(&ready_issues(&t, NOW)), ["br-a"]);
    assert!(blocked_issues(&t, NOW).is_empty());
}

#[test]
fn blocks_edge_blocks_until_target_closes() {
    let blocked = with_dep(issue("br-a"), "br-b", DependencyType::Blocks);
    let t = skein(vec![blocked.clone(), issue("br-b")]);
    assert_eq!(ids(&ready_issues(&t, NOW)), ["br-b"]);
    let blocked_list = blocked_issues(&t, NOW);
    assert_eq!(blocked_list.len(), 1);
    assert_eq!(blocked_list[0].0.id, "br-a");
    assert_eq!(ids(&blocked_list[0].1), ["br-b"]);

    // close the blocker → br-a becomes ready
    let mut closer = issue("br-b");
    closer.status = Status::Closed;
    let t = skein(vec![blocked, closer]);
    assert_eq!(ids(&ready_issues(&t, NOW)), ["br-a"]);
}

#[test]
fn waits_for_and_conditional_blocks_also_block() {
    for ty in [DependencyType::WaitsFor, DependencyType::ConditionalBlocks] {
        let a = with_dep(issue("br-a"), "br-b", ty);
        let t = skein(vec![a, issue("br-b")]);
        assert_eq!(ids(&ready_issues(&t, NOW)), ["br-b"]);
    }
}

#[test]
fn parent_child_does_not_block_the_child() {
    // The child of an open epic must be workable.
    let child = with_dep(issue("br-child"), "br-epic", DependencyType::ParentChild);
    let mut epic = issue("br-epic");
    epic.issue_type = IssueType::Epic;
    let t = skein(vec![child, epic]);
    let mut ready = ids(&ready_issues(&t, NOW));
    ready.sort();
    assert_eq!(ready, ["br-child", "br-epic"], "both child and epic must be ready");
}

#[test]
fn open_children_gate_parent_close() {
    let child1 = with_dep(issue("br-c1"), "br-epic", DependencyType::ParentChild);
    let mut child2 = with_dep(issue("br-c2"), "br-epic", DependencyType::ParentChild);
    child2.status = Status::Closed;
    let t = skein(vec![child1, child2, issue("br-epic")]);

    assert_eq!(ids(&open_children(&t, "br-epic")), ["br-c1"]);
    assert!(open_children(&t, "br-c1").is_empty());
}

#[test]
fn non_blocking_edge_types_never_block() {
    for ty in [
        DependencyType::Related,
        DependencyType::DiscoveredFrom,
        DependencyType::RepliesTo,
        DependencyType::Duplicates,
        DependencyType::Supersedes,
        DependencyType::CausedBy,
        DependencyType::Other("mystery-edge".into()),
    ] {
        let a = with_dep(issue("br-a"), "br-b", ty.clone());
        let t = skein(vec![a, issue("br-b")]);
        assert_eq!(ids(&ready_issues(&t, NOW)), ["br-a", "br-b"], "type {ty:?} must not block");
    }
}

#[test]
fn dangling_edges_do_not_block() {
    let a = with_dep(issue("br-a"), "br-ghost", DependencyType::Blocks);
    let t = skein(vec![a]);
    assert_eq!(ids(&ready_issues(&t, NOW)), ["br-a"]);
}

#[test]
fn only_active_statuses_appear_in_ready_or_blocked() {
    let mut closed = issue("br-closed");
    closed.status = Status::Closed;
    let mut deferred = issue("br-deferred");
    deferred.status = Status::Deferred;
    let mut in_progress = issue("br-inprog");
    in_progress.status = Status::InProgress;
    let mut manually_blocked = with_dep(issue("br-manual"), "br-inprog", DependencyType::Blocks);
    manually_blocked.status = Status::Blocked;

    let t = skein(vec![closed, deferred, in_progress, manually_blocked]);
    assert_eq!(ids(&ready_issues(&t, NOW)), ["br-inprog"]);
    // status "blocked" (manual) is not active, so it doesn't show in
    // blocked_issues either — that listing is for dependency-blocked work.
    assert!(blocked_issues(&t, NOW).is_empty());
}

#[test]
fn ready_sorts_by_priority_then_created_at() {
    let mut low = issue("br-low");
    low.priority = 3;
    let mut high = issue("br-high");
    high.priority = 0;
    let mut mid_old = issue("br-mid-old");
    mid_old.priority = 2;
    mid_old.created_at = "2026-01-01T00:00:00.000000Z".into();
    let mut mid_new = issue("br-mid-new");
    mid_new.priority = 2;
    mid_new.created_at = "2026-06-01T00:00:00.000000Z".into();

    let t = skein(vec![low, high, mid_new, mid_old]);
    assert_eq!(ids(&ready_issues(&t, NOW)), ["br-high", "br-mid-old", "br-mid-new", "br-low"]);
}

#[test]
fn simple_cycle_is_detected() {
    let a = with_dep(issue("br-a"), "br-b", DependencyType::Blocks);
    let b = with_dep(issue("br-b"), "br-a", DependencyType::Blocks);
    let t = skein(vec![a, b]);
    assert_eq!(dependency_cycles(&t), vec![vec!["br-a".to_string(), "br-b".to_string()]]);
}

#[test]
fn self_cycle_is_detected() {
    let a = with_dep(issue("br-a"), "br-a", DependencyType::Blocks);
    let t = skein(vec![a]);
    assert_eq!(dependency_cycles(&t), vec![vec!["br-a".to_string()]]);
}

#[test]
fn longer_cycle_through_parent_child_is_detected() {
    // a blocks-> b parent-child-> c blocks-> a
    let a = with_dep(issue("br-a"), "br-b", DependencyType::Blocks);
    let b = with_dep(issue("br-b"), "br-c", DependencyType::ParentChild);
    let c = with_dep(issue("br-c"), "br-a", DependencyType::Blocks);
    let t = skein(vec![a, b, c]);
    assert_eq!(
        dependency_cycles(&t),
        vec![vec!["br-a".to_string(), "br-b".to_string(), "br-c".to_string()]]
    );
}

#[test]
fn related_edges_do_not_form_cycles() {
    // symmetric related edges are legitimate
    let a = with_dep(issue("br-a"), "br-b", DependencyType::Related);
    let b = with_dep(issue("br-b"), "br-a", DependencyType::Related);
    let t = skein(vec![a, b]);
    assert!(dependency_cycles(&t).is_empty());
}

#[test]
fn acyclic_graph_has_no_cycles() {
    let a = with_dep(issue("br-a"), "br-b", DependencyType::Blocks);
    let b = with_dep(issue("br-b"), "br-c", DependencyType::Blocks);
    let c = issue("br-c");
    let d = with_dep(issue("br-d"), "br-b", DependencyType::Blocks); // diamond-ish
    let t = skein(vec![a, b, c, d]);
    assert!(dependency_cycles(&t).is_empty());
}

#[test]
fn cycle_members_are_all_blocked_not_hanging() {
    // a blocks-> b blocks-> a: both blocked, computation terminates.
    let a = with_dep(issue("br-a"), "br-b", DependencyType::Blocks);
    let b = with_dep(issue("br-b"), "br-a", DependencyType::Blocks);
    let t = skein(vec![a, b]);
    assert!(ready_issues(&t, NOW).is_empty());
    assert_eq!(blocked_issues(&t, NOW).len(), 2);
}

#[test]
fn dependents_lists_reverse_edges() {
    let a = with_dep(issue("br-a"), "br-c", DependencyType::Blocks);
    let b = with_dep(issue("br-b"), "br-c", DependencyType::Related);
    let t = skein(vec![a, b, issue("br-c")]);
    assert_eq!(ids(&dependents_of(&t, "br-c")), ["br-a", "br-b"]);
    assert!(dependents_of(&t, "br-a").is_empty());
}

// ---------------------------------------------------------------------------
// defer / read-time wake
// ---------------------------------------------------------------------------

fn deferred(id: &str, until: Option<&str>) -> Issue {
    let mut i = issue(id);
    i.status = Status::Deferred;
    i.defer_until = until.map(String::from);
    i
}

#[test]
fn deferred_with_future_date_stays_excluded() {
    let t = skein(vec![deferred("br-later", Some("2026-07-01T00:00:00.000000Z"))]);
    assert!(ready_issues(&t, NOW).is_empty());
    assert!(blocked_issues(&t, NOW).is_empty());
}

#[test]
fn deferred_with_past_date_wakes_into_ready() {
    let t = skein(vec![deferred("br-due", Some("2026-06-01T00:00:00.000000Z"))]);
    let ready = ready_issues(&t, NOW);
    assert_eq!(ids(&ready), ["br-due"]);
    // wake is computed, not written: the strand still reads as deferred
    assert_eq!(ready[0].status, Status::Deferred);
}

#[test]
fn defer_until_equal_to_now_is_awake() {
    // "wakes at T" means awake from T on, inclusive
    let t = skein(vec![deferred("br-exact", Some(NOW))]);
    assert_eq!(ids(&ready_issues(&t, NOW)), ["br-exact"]);
}

#[test]
fn deferred_without_date_never_wakes() {
    let t = skein(vec![deferred("br-parked", None)]);
    assert!(ready_issues(&t, NOW).is_empty());
}

#[test]
fn deferred_with_unparseable_date_never_wakes() {
    // conservative: a garbled timestamp keeps the strand asleep rather
    // than surfacing it
    let t = skein(vec![deferred("br-garbled", Some("not a timestamp"))]);
    assert!(ready_issues(&t, NOW).is_empty());
}

#[test]
fn awake_deferred_with_active_blocker_is_blocked_not_ready() {
    let woken = with_dep(
        deferred("br-due", Some("2026-06-01T00:00:00.000000Z")),
        "br-blocker",
        DependencyType::Blocks,
    );
    let t = skein(vec![woken, issue("br-blocker")]);
    assert_eq!(ids(&ready_issues(&t, NOW)), ["br-blocker"]);
    let blocked = blocked_issues(&t, NOW);
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].0.id, "br-due");
    assert_eq!(ids(&blocked[0].1), ["br-blocker"]);
}

#[test]
fn deferred_still_blocks_its_dependents() {
    // deferral pauses the strand, not its obligations: only `closed`
    // releases dependents (is_terminal is unchanged)
    let a = with_dep(issue("br-a"), "br-asleep", DependencyType::Blocks);
    let t = skein(vec![a, deferred("br-asleep", Some("2026-07-01T00:00:00.000000Z"))]);
    assert!(ready_issues(&t, NOW).is_empty());
    let blocked = blocked_issues(&t, NOW);
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].0.id, "br-a");
}

#[test]
fn is_awake_covers_all_statuses() {
    assert!(is_awake(&issue("br-open"), NOW));
    let mut inprog = issue("br-p");
    inprog.status = Status::InProgress;
    assert!(is_awake(&inprog, NOW));
    for status in [Status::Blocked, Status::Closed, Status::Other("paused".into())] {
        let mut i = issue("br-x");
        i.status = status.clone();
        // defer_until only means something on a deferred strand
        i.defer_until = Some("2026-06-01T00:00:00.000000Z".into());
        assert!(!is_awake(&i, NOW), "status {status:?} must not be awake");
    }
}

#[test]
fn active_blockers_ignores_terminal_and_dangling() {
    let mut i = issue("br-a");
    for (target, ty) in [
        ("br-open", DependencyType::Blocks),
        ("br-closed", DependencyType::Blocks),
        ("br-ghost", DependencyType::Blocks),
        ("br-rel", DependencyType::Related),
    ] {
        let d = dep(target, ty);
        i.dependencies.insert(d.key(), d);
    }
    let mut closed = issue("br-closed");
    closed.status = Status::Closed;
    let t = skein(vec![i, issue("br-open"), closed, issue("br-rel")]);

    let blockers = active_blockers(&t, &t.issues["br-a"]);
    assert_eq!(ids(&blockers), ["br-open"]);
}
