//! Ready/blocked computation and dependency-cycle detection.
//!
//! All computed at read time over the hydrated [`TrackerDoc`] — at braid's
//! scale (10³ issues) nothing needs materializing.
//!
//! Semantics (carried over from beads):
//! - an edge *blocks* iff its type [`is_blocking`](crate::schema::DependencyType::is_blocking)
//!   (`blocks`, `parent-child`, `conditional-blocks`, `waits-for`) **and**
//!   its target exists **and** the target's status is not terminal
//! - dangling edges (target id absent from the tracker) never block
//! - **ready** = active status (`open` / `in_progress`) and no active
//!   blockers; blocking is one-step, not transitive, so cycles cannot hang
//!   the computation (members of a blocking cycle are simply all blocked)

use std::collections::BTreeSet;

use crate::schema::{Issue, TrackerDoc};

/// The issues actively blocking `issue`, in dependency-key order.
pub fn active_blockers<'t>(tracker: &'t TrackerDoc, issue: &Issue) -> Vec<&'t Issue> {
    issue
        .dependencies
        .values()
        .filter(|dep| dep.dep_type.is_blocking())
        .filter_map(|dep| tracker.issues.get(&dep.depends_on_id))
        .filter(|target| !target.status.is_terminal())
        .collect()
}

/// Issues that can be worked on now: active status, no active blockers.
/// Sorted by (priority, created_at, id).
pub fn ready_issues(tracker: &TrackerDoc) -> Vec<&Issue> {
    let mut out: Vec<&Issue> = tracker
        .issues
        .values()
        .filter(|i| i.status.is_active())
        .filter(|i| active_blockers(tracker, i).is_empty())
        .collect();
    sort_for_listing(&mut out);
    out
}

/// Issues with active status that are blocked, each with its blockers.
/// Sorted by (priority, created_at, id).
pub fn blocked_issues(tracker: &TrackerDoc) -> Vec<(&Issue, Vec<&Issue>)> {
    let mut with_blockers: Vec<(&Issue, Vec<&Issue>)> = tracker
        .issues
        .values()
        .filter(|i| i.status.is_active())
        .map(|i| (i, active_blockers(tracker, i)))
        .filter(|(_, blockers)| !blockers.is_empty())
        .collect();
    with_blockers.sort_by(|(a, _), (b, _)| listing_order(a, b));
    with_blockers
}

/// Standard listing order: priority, then created_at, then id.
pub fn listing_order(a: &Issue, b: &Issue) -> std::cmp::Ordering {
    a.priority
        .cmp(&b.priority)
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.id.cmp(&b.id))
}

pub fn sort_for_listing(issues: &mut [&Issue]) {
    issues.sort_by(|a, b| listing_order(a, b));
}

/// Issues with non-terminal status holding a `parent-child` edge to `id` —
/// i.e. this issue's still-open children. A non-empty result should gate
/// closing `id`. Sorted by id.
pub fn open_children<'t>(tracker: &'t TrackerDoc, id: &str) -> Vec<&'t Issue> {
    let mut out: Vec<&Issue> = tracker
        .issues
        .values()
        .filter(|i| !i.status.is_terminal())
        .filter(|i| {
            i.dependencies
                .values()
                .any(|d| d.dep_type.is_hierarchical() && d.depends_on_id == id)
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Detect cycles among *structural* dependency edges (blocking types plus
/// `parent-child`). Returns each cycle as a list of issue ids (rotated so
/// the lexicographically smallest id comes first); the result is
/// deduplicated and sorted. Symmetric `related` edges are legitimate and
/// excluded.
///
/// Edges to non-terminal *and* terminal targets both participate: a cycle
/// through a closed issue is still a structural mistake worth reporting.
pub fn dependency_cycles(tracker: &TrackerDoc) -> Vec<Vec<String>> {
    // Tarjan-free approach suited to small graphs: iterative DFS with an
    // explicit color map; every back edge closes one cycle.
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    use std::collections::BTreeMap;

    let mut color: BTreeMap<&str, Color> =
        tracker.issues.keys().map(|k| (k.as_str(), Color::White)).collect();
    let mut cycles: BTreeSet<Vec<String>> = BTreeSet::new();

    fn dfs<'t>(
        tracker: &'t TrackerDoc,
        node: &'t str,
        color: &mut std::collections::BTreeMap<&'t str, Color>,
        stack: &mut Vec<&'t str>,
        cycles: &mut BTreeSet<Vec<String>>,
    ) {
        color.insert(node, Color::Gray);
        stack.push(node);
        if let Some(issue) = tracker.issues.get(node) {
            for dep in issue.dependencies.values() {
                if !dep.dep_type.is_blocking() && !dep.dep_type.is_hierarchical() {
                    continue;
                }
                let target = dep.depends_on_id.as_str();
                match color.get(target) {
                    Some(Color::White) => dfs(tracker, target, color, stack, cycles),
                    Some(Color::Gray) => {
                        // back edge: the cycle is the stack suffix from target
                        let pos = stack.iter().position(|n| *n == target).unwrap();
                        let mut cycle: Vec<String> =
                            stack[pos..].iter().map(|s| s.to_string()).collect();
                        // canonical rotation: smallest id first
                        let min = cycle.iter().enumerate().min_by(|(_, a), (_, b)| a.cmp(b));
                        if let Some((i, _)) = min {
                            cycle.rotate_left(i);
                        }
                        cycles.insert(cycle);
                    }
                    _ => {} // Black or dangling target
                }
            }
        }
        stack.pop();
        color.insert(node, Color::Black);
    }

    let nodes: Vec<&str> = tracker.issues.keys().map(String::as_str).collect();
    let mut stack = Vec::new();
    for node in nodes {
        if color.get(node) == Some(&Color::White) {
            dfs(tracker, node, &mut color, &mut stack, &mut cycles);
        }
    }
    cycles.into_iter().collect()
}

/// Issues whose blocking edges point at this issue (reverse edges),
/// regardless of either side's status. Sorted by id.
pub fn dependents_of<'t>(tracker: &'t TrackerDoc, id: &str) -> Vec<&'t Issue> {
    let mut out: Vec<&Issue> = tracker
        .issues
        .values()
        .filter(|i| i.dependencies.values().any(|d| d.depends_on_id == id))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}
