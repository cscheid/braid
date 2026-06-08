//! Ready/blocked computation and dependency-cycle detection.
//!
//! All computed at read time over the hydrated [`Skein`] — at braid's
//! scale (10³ issues) nothing needs materializing.
//!
//! Semantics (carried over from beads):
//! - an edge *blocks* iff its type [`is_blocking`](crate::schema::DependencyType::is_blocking)
//!   (`blocks`, `parent-child`, `conditional-blocks`, `waits-for`) **and**
//!   its target exists **and** the target's status is not terminal
//! - dangling edges (target id absent from the skein) never block
//! - **ready** = awake (see [`is_awake`]) and no active blockers; blocking
//!   is one-step, not transitive, so cycles cannot hang the computation
//!   (members of a blocking cycle are simply all blocked)
//! - **wake** is read-time: a `deferred` strand whose `defer_until` has
//!   passed counts as awake without anything rewriting the document — no
//!   scheduler, no write-on-read (design decision D2: braid never manages
//!   a daemon)

use std::collections::BTreeSet;

use crate::schema::{Issue, IssueType, Skein, Status};
use crate::time::is_after;

/// Field filters shared by the `list` and `ready` listings, so the two
/// commands cannot drift apart in semantics. Every populated field must
/// match (AND across fields); an empty filter matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilter {
    /// Required labels: a strand must carry *all* of them (AND).
    pub labels: Vec<String>,
    /// Exact assignee match; strands with no assignee never match.
    pub assignee: Option<String>,
    /// Issue type; arbitrary strings match `Other(...)` strands.
    pub issue_type: Option<IssueType>,
}

impl ListFilter {
    pub fn matches(&self, issue: &Issue) -> bool {
        self.labels.iter().all(|l| issue.labels.contains(l))
            && self.assignee.as_deref().is_none_or(|a| issue.assignee.as_deref() == Some(a))
            && self.issue_type.as_ref().is_none_or(|t| issue.issue_type == *t)
    }
}

/// The issues actively blocking `issue`, in dependency-key order.
pub fn active_blockers<'t>(skein: &'t Skein, issue: &Issue) -> Vec<&'t Issue> {
    issue
        .dependencies
        .values()
        .filter(|dep| dep.dep_type.is_blocking())
        .filter_map(|dep| skein.issues.get(&dep.depends_on_id))
        .filter(|target| !target.status.is_terminal())
        .collect()
}

/// Whether `issue` counts as workable at instant `now` (RFC 3339): an
/// active status, or `deferred` with a `defer_until` that has passed
/// (inclusive: awake from the wake instant on). A dateless deferred
/// strand sleeps until an explicit undefer; an unparseable `defer_until`
/// is conservative and never wakes.
pub fn is_awake(issue: &Issue, now: &str) -> bool {
    if issue.status.is_active() {
        return true;
    }
    if issue.status != Status::Deferred {
        return false;
    }
    match issue.defer_until.as_deref() {
        // awake iff until <= now; is_after returns None on parse failure
        Some(until) => matches!(is_after(until, now), Some(false)),
        None => false,
    }
}

/// Issues that can be worked on at `now`: awake, no active blockers.
/// Sorted by (priority, created_at, id).
pub fn ready_issues<'t>(skein: &'t Skein, now: &str) -> Vec<&'t Issue> {
    let mut out: Vec<&Issue> = skein
        .issues
        .values()
        .filter(|i| is_awake(i, now))
        .filter(|i| active_blockers(skein, i).is_empty())
        .collect();
    sort_for_listing(&mut out);
    out
}

/// Awake issues that are blocked, each with its blockers.
/// Sorted by (priority, created_at, id).
pub fn blocked_issues<'t>(skein: &'t Skein, now: &str) -> Vec<(&'t Issue, Vec<&'t Issue>)> {
    let mut with_blockers: Vec<(&Issue, Vec<&Issue>)> = skein
        .issues
        .values()
        .filter(|i| is_awake(i, now))
        .map(|i| (i, active_blockers(skein, i)))
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
pub fn open_children<'t>(skein: &'t Skein, id: &str) -> Vec<&'t Issue> {
    let mut out: Vec<&Issue> = skein
        .issues
        .values()
        .filter(|i| !i.status.is_terminal())
        .filter(|i| {
            i.dependencies.values().any(|d| d.dep_type.is_hierarchical() && d.depends_on_id == id)
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// All hierarchical children of `id` (issues holding a `parent-child` edge
/// to `id`), **terminal or not**, sorted by id. Like [`open_children`] but
/// it keeps closed children: a dependency *tree* shows the whole shape, not
/// just the open frontier. The recursion in [`dep_tree`] is built on this.
pub fn child_strands<'t>(skein: &'t Skein, id: &str) -> Vec<&'t Issue> {
    let mut out: Vec<&Issue> = skein
        .issues
        .values()
        .filter(|i| {
            i.dependencies.values().any(|d| d.dep_type.is_hierarchical() && d.depends_on_id == id)
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// A node in the recursive dependency tree produced by [`dep_tree`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DepTreeNode {
    pub id: String,
    pub title: String,
    /// Status as a string (`open`, `closed`, …), or `missing!` for an id
    /// absent from the skein (only reachable via a defensive path).
    pub status: String,
    /// The dependency type linking this node to its parent in the tree;
    /// `None` for the root. Always `parent-child` today — the tree follows
    /// only hierarchical edges (decided 2026-06-08) — but carried per node
    /// so a future multi-edge mode stays additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dep_type: Option<String>,
    /// `true` when this id already appears among its own ancestors: the
    /// node is emitted to mark the cycle, but not expanded.
    pub cycle: bool,
    pub children: Vec<DepTreeNode>,
}

/// Build the recursive `parent-child` descendant tree rooted at `root_id`
/// (children, grandchildren, …). Children come from [`child_strands`], so
/// closed strands are included and siblings are ordered by id.
///
/// Cycles are broken safely: an id that reappears among its own ancestors
/// is emitted with `cycle: true` and not re-expanded, so a `parent-child`
/// loop (which CRDT merges can introduce) terminates instead of recursing
/// forever. A strand reachable by two distinct paths (a diamond) is shown
/// — and expanded — in each location; only true ancestor cycles are cut.
pub fn dep_tree(skein: &Skein, root_id: &str) -> DepTreeNode {
    let mut path: Vec<String> = Vec::new();
    build_dep_tree_node(skein, root_id, None, &mut path)
}

fn build_dep_tree_node(
    skein: &Skein,
    id: &str,
    dep_type: Option<String>,
    path: &mut Vec<String>,
) -> DepTreeNode {
    let (title, status) = match skein.issues.get(id) {
        Some(i) => (i.title.clone(), i.status.as_str().to_string()),
        None => (String::new(), "missing!".to_string()),
    };

    if path.iter().any(|p| p == id) {
        return DepTreeNode {
            id: id.to_string(),
            title,
            status,
            dep_type,
            cycle: true,
            children: Vec::new(),
        };
    }

    path.push(id.to_string());
    let children = child_strands(skein, id)
        .into_iter()
        .map(|c| build_dep_tree_node(skein, &c.id, Some("parent-child".to_string()), path))
        .collect();
    path.pop();

    DepTreeNode { id: id.to_string(), title, status, dep_type, cycle: false, children }
}

/// Detect cycles among *structural* dependency edges (blocking types plus
/// `parent-child`). Returns each cycle as a list of issue ids (rotated so
/// the lexicographically smallest id comes first); the result is
/// deduplicated and sorted. Symmetric `related` edges are legitimate and
/// excluded.
///
/// Edges to non-terminal *and* terminal targets both participate: a cycle
/// through a closed issue is still a structural mistake worth reporting.
pub fn dependency_cycles(skein: &Skein) -> Vec<Vec<String>> {
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
        skein.issues.keys().map(|k| (k.as_str(), Color::White)).collect();
    let mut cycles: BTreeSet<Vec<String>> = BTreeSet::new();

    fn dfs<'t>(
        skein: &'t Skein,
        node: &'t str,
        color: &mut std::collections::BTreeMap<&'t str, Color>,
        stack: &mut Vec<&'t str>,
        cycles: &mut BTreeSet<Vec<String>>,
    ) {
        color.insert(node, Color::Gray);
        stack.push(node);
        if let Some(issue) = skein.issues.get(node) {
            for dep in issue.dependencies.values() {
                if !dep.dep_type.is_blocking() && !dep.dep_type.is_hierarchical() {
                    continue;
                }
                let target = dep.depends_on_id.as_str();
                match color.get(target) {
                    Some(Color::White) => dfs(skein, target, color, stack, cycles),
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

    let nodes: Vec<&str> = skein.issues.keys().map(String::as_str).collect();
    let mut stack = Vec::new();
    for node in nodes {
        if color.get(node) == Some(&Color::White) {
            dfs(skein, node, &mut color, &mut stack, &mut cycles);
        }
    }
    cycles.into_iter().collect()
}

/// Issues whose blocking edges point at this issue (reverse edges),
/// regardless of either side's status. Sorted by id.
pub fn dependents_of<'t>(skein: &'t Skein, id: &str) -> Vec<&'t Issue> {
    let mut out: Vec<&Issue> = skein
        .issues
        .values()
        .filter(|i| i.dependencies.values().any(|d| d.depends_on_id == id))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::IssueType;
    use std::collections::{BTreeMap, BTreeSet};

    fn issue(labels: &[&str], assignee: Option<&str>, issue_type: IssueType) -> Issue {
        Issue {
            id: "br-test01".into(),
            title: "test".into(),
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            status: Status::Open,
            priority: 2,
            issue_type,
            assignee: assignee.map(String::from),
            created_at: "2026-06-04T00:00:00Z".into(),
            created_by: "test".into(),
            updated_at: "2026-06-04T00:00:00Z".into(),
            closed_at: None,
            close_reason: None,
            defer_until: None,
            external_ref: None,
            labels: labels.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
            dependencies: BTreeMap::new(),
            comments: BTreeMap::new(),
        }
    }

    fn labels(labels: &[&str]) -> ListFilter {
        ListFilter { labels: labels.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    fn assignee(a: &str) -> ListFilter {
        ListFilter { assignee: Some(a.to_string()), ..Default::default() }
    }

    fn issue_type(t: &str) -> ListFilter {
        ListFilter { issue_type: Some(IssueType::from(t)), ..Default::default() }
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert!(ListFilter::default().matches(&issue(&[], None, IssueType::Task)));
        assert!(ListFilter::default().matches(&issue(&["x"], Some("alice"), IssueType::Bug)));
    }

    #[test]
    fn label_filter_requires_all_labels() {
        let i = issue(&["x", "y"], None, IssueType::Task);
        assert!(labels(&["x"]).matches(&i));
        assert!(labels(&["y"]).matches(&i));
        assert!(labels(&["x", "y"]).matches(&i));
        assert!(!labels(&["x", "z"]).matches(&i), "AND semantics: every label must be present");
        assert!(!labels(&["z"]).matches(&i));
        assert!(!labels(&["x"]).matches(&issue(&[], None, IssueType::Task)));
    }

    #[test]
    fn assignee_filter_is_exact_and_skips_unassigned() {
        let i = issue(&[], Some("alice"), IssueType::Task);
        assert!(assignee("alice").matches(&i));
        assert!(!assignee("bob").matches(&i));
        assert!(!assignee("ali").matches(&i), "exact match, not substring");
        assert!(!assignee("alice").matches(&issue(&[], None, IssueType::Task)));
    }

    #[test]
    fn type_filter_matches_known_and_custom_types() {
        let bug = issue(&[], None, IssueType::Bug);
        assert!(issue_type("bug").matches(&bug));
        assert!(!issue_type("task").matches(&bug));
        // arbitrary strings match Other(...) strands, consistent with the
        // schema's tolerance for unknown types
        let rfc = issue(&[], None, IssueType::from("rfc"));
        assert!(issue_type("rfc").matches(&rfc));
        assert!(!issue_type("bug").matches(&rfc));
    }

    #[test]
    fn filters_compose_with_and() {
        let i = issue(&["x"], Some("alice"), IssueType::Bug);
        let f = ListFilter {
            labels: vec!["x".into()],
            assignee: Some("alice".into()),
            issue_type: Some(IssueType::Bug),
        };
        assert!(f.matches(&i));
        let f = ListFilter { assignee: Some("bob".into()), ..f };
        assert!(!f.matches(&i), "one failing field fails the whole filter");
    }

    // ----- dep_tree -----

    use crate::schema::{Dependency, DependencyType, SkeinMetadata};

    /// A strand with the given id/status and no dependencies.
    fn strand(id: &str, status: Status) -> Issue {
        let mut i = issue(&[], None, IssueType::Task);
        i.id = id.into();
        i.title = format!("title-{id}");
        i.status = status;
        i
    }

    /// Add a `parent-child` edge from `child` to `parent` (child depends on
    /// parent — the direction `dep add <child> <parent> --type parent-child`
    /// records).
    fn link_parent(child: &mut Issue, parent: &str) {
        let dep = Dependency {
            depends_on_id: parent.to_string(),
            dep_type: DependencyType::ParentChild,
            created_at: "2026-06-04T00:00:00Z".into(),
            created_by: "test".into(),
        };
        child.dependencies.insert(dep.key(), dep);
    }

    fn skein_of(issues: Vec<Issue>) -> Skein {
        Skein {
            metadata: SkeinMetadata {
                schema_version: 1,
                name: "t".into(),
                id_prefix: "br".into(),
                created_at: "2026-06-04T00:00:00Z".into(),
                rotated_at: None,
                rotated_to: None,
            },
            issues: issues.into_iter().map(|i| (i.id.clone(), i)).collect(),
        }
    }

    /// child ids of a node, in order
    fn child_ids(node: &DepTreeNode) -> Vec<&str> {
        node.children.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn dep_tree_builds_nested_descendants() {
        // epic A; B and C are children of A; D is a child of C.
        let a = strand("br-a", Status::Open);
        let mut b = strand("br-b", Status::Open);
        link_parent(&mut b, "br-a");
        let mut c = strand("br-c", Status::Open);
        link_parent(&mut c, "br-a");
        let mut d = strand("br-d", Status::Open);
        link_parent(&mut d, "br-c");
        let skein = skein_of(vec![a, b, c, d]);

        let tree = dep_tree(&skein, "br-a");
        assert_eq!(tree.id, "br-a");
        assert_eq!(tree.dep_type, None, "root has no parent edge");
        assert!(!tree.cycle);
        assert_eq!(child_ids(&tree), vec!["br-b", "br-c"], "siblings sorted by id");

        let c_node = tree.children.iter().find(|n| n.id == "br-c").unwrap();
        assert_eq!(c_node.dep_type.as_deref(), Some("parent-child"));
        assert_eq!(child_ids(c_node), vec!["br-d"]);
        assert!(c_node.children[0].children.is_empty(), "D is a leaf");
    }

    #[test]
    fn dep_tree_includes_closed_children() {
        let a = strand("br-a", Status::Open);
        let mut done = strand("br-done", Status::Closed);
        link_parent(&mut done, "br-a");
        let skein = skein_of(vec![a, done]);

        let tree = dep_tree(&skein, "br-a");
        assert_eq!(child_ids(&tree), vec!["br-done"]);
        assert_eq!(tree.children[0].status, "closed");
    }

    #[test]
    fn dep_tree_breaks_parent_child_cycle() {
        // A is a child of B and B is a child of A (a parent-child loop).
        let mut a = strand("br-a", Status::Open);
        link_parent(&mut a, "br-b");
        let mut b = strand("br-b", Status::Open);
        link_parent(&mut b, "br-a");
        let skein = skein_of(vec![a, b]);

        let tree = dep_tree(&skein, "br-a");
        assert!(!tree.cycle);
        assert_eq!(child_ids(&tree), vec!["br-b"]);
        let b_node = &tree.children[0];
        // B's child is A again — already an ancestor, so marked and cut.
        assert_eq!(child_ids(b_node), vec!["br-a"]);
        let a_again = &b_node.children[0];
        assert!(a_again.cycle, "the repeated ancestor is flagged as a cycle");
        assert!(a_again.children.is_empty(), "cycle node is not expanded");
    }

    #[test]
    fn dep_tree_leaf_has_no_children() {
        let skein = skein_of(vec![strand("br-solo", Status::Open)]);
        let tree = dep_tree(&skein, "br-solo");
        assert!(tree.children.is_empty());
        assert!(!tree.cycle);
        assert_eq!(tree.status, "open");
    }

    #[test]
    fn dep_tree_expands_shared_node_in_each_location() {
        // diamond: B and C are children of A; D is a child of BOTH B and C.
        // D is not an ancestor of itself, so it is shown under each parent.
        let a = strand("br-a", Status::Open);
        let mut b = strand("br-b", Status::Open);
        link_parent(&mut b, "br-a");
        let mut c = strand("br-c", Status::Open);
        link_parent(&mut c, "br-a");
        let mut d = strand("br-d", Status::Open);
        link_parent(&mut d, "br-b");
        link_parent(&mut d, "br-c");
        let skein = skein_of(vec![a, b, c, d]);

        let tree = dep_tree(&skein, "br-a");
        let b_node = tree.children.iter().find(|n| n.id == "br-b").unwrap();
        let c_node = tree.children.iter().find(|n| n.id == "br-c").unwrap();
        assert_eq!(child_ids(b_node), vec!["br-d"]);
        assert_eq!(child_ids(c_node), vec!["br-d"]);
        assert!(!b_node.children[0].cycle, "a shared (non-ancestor) node is not a cycle");
    }
}
