//! Value-returning operations over an open skein session.
//!
//! This is the layer both skins consume (plan:
//! claude-notes/plans/2026/06/04/braid-mcp.md):
//!
//! - the **CLI** opens a [`Session`] per invocation, prints the returned
//!   values, and shuts down;
//! - the **MCP server** opens one Session at startup and holds it — samod
//!   keeps syncing continuously, mutations push with a bounded barrier and
//!   report a [`PushOutcome`] instead of blocking, and every operation
//!   re-checks for rotation (which can arrive over sync at any time).
//!
//! Nothing here prints; nothing here exposes the doc id.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Result, bail};
use braid_core::amdoc::{delete_issue, hydrate, hydrate_metadata, reconcile_issue};
use braid_core::domain::{
    ListFilter, blocked_issues, dependency_cycles, dependents_of, listing_order, open_children,
    ready_issues,
};
use braid_core::id::{new_comment_id, new_issue_id};
use braid_core::schema::{Comment, Dependency, DependencyType, Issue, IssueType, Skein, Status};
use braid_core::time::{now_rfc3339, parse_until};
use serde::Serialize;

use crate::skein::{OpenedSkein, PushOutcome, check_rotation, open_skein};

// ---------------------------------------------------------------------------
// result types
// ---------------------------------------------------------------------------

/// A mutation's value plus what happened on the wire.
#[derive(Debug, Serialize)]
pub struct Mutated<T> {
    #[serde(flatten)]
    pub value: T,
    pub sync: PushOutcome,
}

#[derive(Debug, Serialize)]
pub struct BlockedStrand {
    pub issue: Issue,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DepNeighbor {
    pub id: String,
    pub dep_type: String,
    /// The neighbor's status, or `"missing!"` for a dangling outgoing edge.
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct DepListing {
    pub outgoing: Vec<DepNeighbor>,
    pub incoming: Vec<DepNeighbor>,
}

#[derive(Debug, Serialize)]
pub struct DepAdded {
    pub issue: Issue,
    pub key: String,
    /// Dependency cycles present after the addition (allowed, warned).
    pub cycles: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct CommentAdded {
    pub issue_id: String,
    pub comment: Comment,
}

#[derive(Debug, Serialize)]
pub struct Deleted {
    pub deleted: Vec<String>,
    /// Strands left holding dangling edges, per deleted id (only under
    /// `force`; without it dependents make deletion fail).
    pub dangling: Vec<DanglingNote>,
}

#[derive(Debug, Serialize)]
pub struct DanglingNote {
    pub deleted_id: String,
    pub dependents: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Closed {
    pub closed: Vec<Issue>,
}

#[derive(Debug, Serialize)]
pub struct Reopened {
    pub reopened: Vec<Issue>,
}

#[derive(Debug, Serialize)]
pub struct Imported {
    pub imported: usize,
    /// beads tombstones recognized and skipped during parsing (never
    /// upserted). The session does not compute this — parsing does — so it
    /// is passed in and carried through to the caller's report.
    pub skipped: usize,
}

/// Skein metadata safe for any consumer: excludes the rotation fields
/// (`rotated_to` is a bearer capability).
#[derive(Debug, Serialize)]
pub struct PublicMetadata {
    pub name: String,
    pub id_prefix: String,
    pub created_at: String,
}

/// Connection/convergence status for the skein resource (plan Q6: the
/// status surface lives on `braid://skein`, not in a sync tool).
#[derive(Debug, Serialize)]
pub struct SyncState {
    pub online: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline_reason: Option<String>,
    /// Whether the server has acknowledged everything local; `null` when
    /// offline (unknowable).
    pub in_sync: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct Deferred {
    pub deferred: Vec<Issue>,
}

#[derive(Debug, Serialize)]
pub struct Undeferred {
    pub undeferred: Vec<Issue>,
}

// ---------------------------------------------------------------------------
// option structs (presentation-free: no json flags here)
// ---------------------------------------------------------------------------

pub struct CreateOpts {
    pub title: String,
    pub description: Option<String>,
    pub issue_type: String,
    pub priority: i64,
    pub labels: Vec<String>,
    pub slug: Option<String>,
    pub assignee: Option<String>,
}

#[derive(Default)]
pub struct UpdateOpts {
    pub title: Option<String>,
    pub description: Option<String>,
    pub design: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub notes: Option<String>,
    pub status: Option<String>,
    pub priority: Option<i64>,
    pub issue_type: Option<String>,
    pub assignee: Option<String>,
    pub external_ref: Option<String>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Resolve a user-supplied id query: exact match first, then unique
/// substring match.
pub fn resolve_issue<'t>(skein: &'t Skein, query: &str) -> Result<&'t Issue> {
    if let Some(issue) = skein.issues.get(query) {
        return Ok(issue);
    }
    let matches: Vec<&Issue> = skein.issues.values().filter(|i| i.id.contains(query)).collect();
    match matches.len() {
        0 => bail!("no issue matching {query:?}"),
        1 => Ok(matches[0]),
        _ => {
            let ids: Vec<&str> = matches.iter().map(|i| i.id.as_str()).collect();
            bail!("ambiguous id {query:?}: matches {}", ids.join(", "));
        }
    }
}

/// Empty strings clear optional fields (`--description ""` removes the
/// description); `None` leaves the field untouched.
fn apply_opt(field: &mut Option<String>, flag: Option<String>) {
    if let Some(v) = flag {
        *field = if v.is_empty() { None } else { Some(v) };
    }
}

fn issue_matches(issue: &Issue, needle_lower: &str) -> bool {
    let mut haystacks: Vec<&str> = vec![&issue.id, &issue.title];
    for s in [
        &issue.description,
        &issue.design,
        &issue.acceptance_criteria,
        &issue.notes,
        &issue.assignee,
        &issue.external_ref,
    ]
    .into_iter()
    .flatten()
    {
        haystacks.push(s);
    }
    haystacks.extend(issue.labels.iter().map(String::as_str));
    haystacks.extend(issue.comments.values().map(|c| c.text.as_str()));
    haystacks.iter().any(|h| h.to_lowercase().contains(needle_lower))
}

// ---------------------------------------------------------------------------
// session
// ---------------------------------------------------------------------------

pub struct Session {
    opened: OpenedSkein,
}

impl Session {
    /// Open a session: config discovery, dial (offline tolerated), load,
    /// pull, rotation check.
    pub async fn open(cwd: &Path) -> Result<Session> {
        Ok(Session { opened: open_skein(cwd).await? })
    }

    /// Wrap an already-opened skein (the CLI's `sync` command opens one
    /// itself to inspect connectivity before deciding to proceed).
    pub fn from_opened(opened: OpenedSkein) -> Session {
        Session { opened }
    }

    pub fn author(&self) -> &str {
        &self.opened.cfg.author
    }

    pub fn sync_server(&self) -> &str {
        &self.opened.cfg.sync_server
    }

    pub fn is_online(&self) -> bool {
        self.opened.conn.is_some()
    }

    pub fn offline_reason(&self) -> Option<&str> {
        self.opened.offline_reason.as_deref()
    }

    /// Explicit bounded push barrier (used by `braid sync`; mutations
    /// already push individually).
    pub async fn push(&self) -> PushOutcome {
        self.opened.push().await
    }

    /// Skein metadata (rotation-checked like every read; the rotation
    /// fields and especially `rotated_to` are deliberately not returned).
    pub fn metadata(&self) -> Result<PublicMetadata> {
        self.guard_rotation()?;
        let meta = self.opened.doc.with_document(|d| hydrate_metadata(d))?;
        Ok(PublicMetadata {
            name: meta.name,
            id_prefix: meta.id_prefix,
            created_at: meta.created_at,
        })
    }

    /// Non-blocking connection/convergence snapshot.
    pub fn sync_state(&self) -> SyncState {
        let in_sync = self.opened.conn.map(|conn| {
            let local = self.opened.doc.with_document(|d| d.get_heads());
            let (peers, _changes) = self.opened.doc.peers();
            peers.get(&conn).map(|s| s.shared_heads.as_ref() == Some(&local)).unwrap_or(false)
        });
        SyncState {
            online: self.opened.conn.is_some(),
            offline_reason: self.opened.offline_reason.clone(),
            in_sync,
        }
    }

    /// A stream yielding once per document change (local or remote) —
    /// the MCP server's notification source. samod types stay internal.
    ///
    /// samod's `changes()` borrows its handle, so a forwarding task owns a
    /// cloned `DocHandle` and feeds an owned channel; the task ends when
    /// either side drops.
    pub fn changes_stream(&self) -> impl futures::Stream<Item = ()> + Send + 'static {
        use futures::StreamExt;
        let doc = self.opened.doc.clone();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        tokio::spawn(async move {
            let mut changes = std::pin::pin!(doc.changes());
            while changes.next().await.is_some() {
                if tx.send(()).is_err() {
                    break; // receiver gone
                }
            }
        });
        futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) })
    }

    /// Shut the session down, flushing the cache.
    pub async fn shutdown(self) {
        self.opened.close().await;
    }

    /// Long-lived sessions may receive a rotation over sync at any time;
    /// every operation re-checks (cheap: metadata only).
    fn guard_rotation(&self) -> Result<()> {
        let meta = self.opened.doc.with_document(|d| hydrate_metadata(d))?;
        check_rotation(&meta)
    }

    fn hydrate(&self) -> Result<Skein> {
        self.guard_rotation()?;
        Ok(self.opened.doc.with_document(|d| hydrate(d))?)
    }

    // -- queries ------------------------------------------------------------

    pub fn ready(&self, filter: &ListFilter) -> Result<Vec<Issue>> {
        let skein = self.hydrate()?;
        Ok(ready_issues(&skein, &now_rfc3339())
            .into_iter()
            .filter(|i| filter.matches(i))
            .cloned()
            .collect())
    }

    pub fn blocked(&self) -> Result<Vec<BlockedStrand>> {
        let skein = self.hydrate()?;
        Ok(blocked_issues(&skein, &now_rfc3339())
            .into_iter()
            .map(|(issue, blockers)| BlockedStrand {
                issue: issue.clone(),
                blocked_by: blockers.iter().map(|b| b.id.clone()).collect(),
            })
            .collect())
    }

    /// List strands: open (non-closed) ones by default, a single status
    /// when `status` is given, everything when `all` is set; `filter`
    /// narrows further by labels/assignee/type.
    pub fn list(&self, status: Option<&str>, all: bool, filter: &ListFilter) -> Result<Vec<Issue>> {
        let skein = self.hydrate()?;
        let mut issues: Vec<&Issue> = skein
            .issues
            .values()
            .filter(|i| match status {
                Some(s) => i.status.as_str() == s,
                None => all || !i.status.is_terminal(),
            })
            .filter(|i| filter.matches(i))
            .collect();
        issues.sort_by(|a, b| listing_order(a, b));
        Ok(issues.into_iter().cloned().collect())
    }

    pub fn show(&self, query: &str) -> Result<Issue> {
        let skein = self.hydrate()?;
        Ok(resolve_issue(&skein, query)?.clone())
    }

    pub fn search(&self, needle: &str) -> Result<Vec<Issue>> {
        let skein = self.hydrate()?;
        let needle_lower = needle.to_lowercase();
        let mut found: Vec<&Issue> =
            skein.issues.values().filter(|i| issue_matches(i, &needle_lower)).collect();
        found.sort_by(|a, b| listing_order(a, b));
        Ok(found.into_iter().cloned().collect())
    }

    pub fn dep_list(&self, query: &str) -> Result<DepListing> {
        let skein = self.hydrate()?;
        let issue = resolve_issue(&skein, query)?;
        let outgoing = issue
            .dependencies
            .values()
            .map(|dep| DepNeighbor {
                id: dep.depends_on_id.clone(),
                dep_type: dep.dep_type.as_str().to_string(),
                status: skein
                    .issues
                    .get(&dep.depends_on_id)
                    .map(|t| t.status.as_str().to_string())
                    .unwrap_or_else(|| "missing!".to_string()),
            })
            .collect();
        let mut incoming = Vec::new();
        for dependent in dependents_of(&skein, &issue.id) {
            for dep in dependent.dependencies.values() {
                if dep.depends_on_id == issue.id {
                    incoming.push(DepNeighbor {
                        id: dependent.id.clone(),
                        dep_type: dep.dep_type.as_str().to_string(),
                        status: dependent.status.as_str().to_string(),
                    });
                }
            }
        }
        Ok(DepListing { outgoing, incoming })
    }

    pub fn dep_cycles(&self) -> Result<Vec<Vec<String>>> {
        let skein = self.hydrate()?;
        Ok(dependency_cycles(&skein))
    }

    /// All strands as JSONL (id-sorted), conforming to
    /// docs/schemas/strand.schema.json.
    pub fn export_jsonl(&self) -> Result<String> {
        let skein = self.hydrate()?;
        let mut out = String::new();
        for issue in skein.issues.values() {
            out.push_str(&serde_json::to_string(issue)?);
            out.push('\n');
        }
        Ok(out)
    }

    pub fn strand_count(&self) -> Result<usize> {
        Ok(self.hydrate()?.issues.len())
    }

    // -- mutations ----------------------------------------------------------

    async fn commit_one(&self, issue: &Issue) -> Result<PushOutcome> {
        self.opened
            .doc
            .with_document(|d| d.transact(|tx| reconcile_issue(tx, issue)).map_err(|f| f.error))?;
        Ok(self.opened.push().await)
    }

    async fn commit_many(&self, issues: &[Issue]) -> Result<PushOutcome> {
        self.opened.doc.with_document(|d| {
            d.transact(|tx| {
                for issue in issues {
                    reconcile_issue(tx, issue)?;
                }
                Ok::<_, braid_core::amdoc::ReconcileError>(())
            })
            .map_err(|f| f.error)
        })?;
        Ok(self.opened.push().await)
    }

    pub async fn create(&self, opts: CreateOpts) -> Result<Mutated<Issue>> {
        let skein = self.hydrate()?;

        let mut id = new_issue_id(&skein.metadata.id_prefix, opts.slug.as_deref());
        // Collision with an existing id is astronomically improbable but
        // free to guard against locally:
        while skein.issues.contains_key(&id) {
            id = new_issue_id(&skein.metadata.id_prefix, opts.slug.as_deref());
        }

        let now = now_rfc3339();
        let issue = Issue {
            id,
            title: opts.title,
            description: opts.description,
            design: None,
            acceptance_criteria: None,
            notes: None,
            status: Status::Open,
            priority: opts.priority,
            issue_type: IssueType::from(opts.issue_type.as_str()),
            assignee: opts.assignee,
            created_at: now.clone(),
            created_by: self.opened.cfg.author.clone(),
            updated_at: now,
            closed_at: None,
            close_reason: None,
            defer_until: None,
            external_ref: None,
            labels: opts.labels.into_iter().collect::<BTreeSet<_>>(),
            dependencies: BTreeMap::new(),
            comments: BTreeMap::new(),
        };

        let sync = self.commit_one(&issue).await?;
        Ok(Mutated { value: issue, sync })
    }

    pub async fn update(&self, query: &str, opts: UpdateOpts) -> Result<Mutated<Issue>> {
        let skein = self.hydrate()?;
        let mut issue = resolve_issue(&skein, query)?.clone();

        if let Some(t) = opts.title {
            issue.title = t;
        }
        apply_opt(&mut issue.description, opts.description);
        apply_opt(&mut issue.design, opts.design);
        apply_opt(&mut issue.acceptance_criteria, opts.acceptance_criteria);
        apply_opt(&mut issue.notes, opts.notes);
        apply_opt(&mut issue.assignee, opts.assignee);
        apply_opt(&mut issue.external_ref, opts.external_ref);
        if let Some(s) = opts.status {
            issue.status = Status::from(s.as_str());
            // leaving `deferred` by any path clears the wake time
            if issue.status != Status::Deferred {
                issue.defer_until = None;
            }
        }
        if let Some(p) = opts.priority {
            issue.priority = p;
        }
        if let Some(t) = opts.issue_type {
            issue.issue_type = IssueType::from(t.as_str());
        }
        for l in opts.add_labels {
            issue.labels.insert(l);
        }
        for l in &opts.remove_labels {
            issue.labels.remove(l);
        }
        issue.updated_at = now_rfc3339();

        let sync = self.commit_one(&issue).await?;
        Ok(Mutated { value: issue, sync })
    }

    pub async fn close_strands(
        &self,
        queries: &[String],
        reason: Option<String>,
        force: bool,
    ) -> Result<Mutated<Closed>> {
        let skein = self.hydrate()?;

        // Resolve and validate everything before mutating anything.
        let mut to_close: Vec<Issue> = Vec::new();
        let closing_now: Vec<String> = queries
            .iter()
            .map(|q| resolve_issue(&skein, q).map(|i| i.id.clone()))
            .collect::<Result<_>>()?;
        for query in queries {
            let issue = resolve_issue(&skein, query)?;
            let open_kids: Vec<&Issue> = open_children(&skein, &issue.id)
                .into_iter()
                // children being closed in the same invocation don't count
                .filter(|c| !closing_now.contains(&c.id))
                .collect();
            if !open_kids.is_empty() && !force {
                let ids: Vec<&str> = open_kids.iter().map(|i| i.id.as_str()).collect();
                bail!(
                    "{} has open children ({}); close them first or pass --force",
                    issue.id,
                    ids.join(", ")
                );
            }
            to_close.push(issue.clone());
        }

        let now = now_rfc3339();
        for issue in &mut to_close {
            issue.status = Status::Closed;
            issue.closed_at = Some(now.clone());
            issue.close_reason = reason.clone();
            issue.defer_until = None;
            issue.updated_at = now.clone();
        }

        let sync = self.commit_many(&to_close).await?;
        Ok(Mutated { value: Closed { closed: to_close }, sync })
    }

    pub async fn reopen(&self, queries: &[String]) -> Result<Mutated<Reopened>> {
        let skein = self.hydrate()?;

        let mut to_reopen: Vec<Issue> =
            queries.iter().map(|q| resolve_issue(&skein, q).cloned()).collect::<Result<_>>()?;
        let now = now_rfc3339();
        for issue in &mut to_reopen {
            issue.status = Status::Open;
            issue.closed_at = None;
            issue.close_reason = None;
            issue.defer_until = None;
            issue.updated_at = now.clone();
        }

        let sync = self.commit_many(&to_reopen).await?;
        Ok(Mutated { value: Reopened { reopened: to_reopen }, sync })
    }

    /// Defer strands: status → `deferred`, with an optional wake time. The
    /// wake is read-time (see `domain::is_awake`): once `defer_until`
    /// passes, `ready` surfaces the strand again without anything
    /// rewriting it. Re-deferring updates the wake time; a `None` wake
    /// sleeps until an explicit undefer.
    pub async fn defer(
        &self,
        queries: &[String],
        until: Option<String>,
    ) -> Result<Mutated<Deferred>> {
        let skein = self.hydrate()?;

        let now = now_rfc3339();
        let wake = match until.as_deref() {
            Some(input) => Some(parse_until(input, &now).ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot parse --until {input:?}: accepted forms are an RFC 3339 \
                     timestamp (2026-07-01T09:00:00Z), a date (2026-07-01), or a \
                     duration from now (36h, 7d, 2w)"
                )
            })?),
            None => None,
        };

        // Resolve and validate everything before mutating anything.
        let mut to_defer: Vec<Issue> = Vec::new();
        for query in queries {
            let issue = resolve_issue(&skein, query)?;
            if issue.status == Status::Closed {
                bail!("{} is closed; reopen it before deferring", issue.id);
            }
            to_defer.push(issue.clone());
        }

        for issue in &mut to_defer {
            issue.status = Status::Deferred;
            issue.defer_until = wake.clone();
            issue.updated_at = now.clone();
        }

        let sync = self.commit_many(&to_defer).await?;
        Ok(Mutated { value: Deferred { deferred: to_defer }, sync })
    }

    /// Wake deferred strands explicitly: status → `open`, wake time cleared.
    pub async fn undefer(&self, queries: &[String]) -> Result<Mutated<Undeferred>> {
        let skein = self.hydrate()?;

        let mut to_wake: Vec<Issue> = Vec::new();
        for query in queries {
            let issue = resolve_issue(&skein, query)?;
            if issue.status != Status::Deferred {
                bail!("{} is not deferred (status: {})", issue.id, issue.status);
            }
            to_wake.push(issue.clone());
        }

        let now = now_rfc3339();
        for issue in &mut to_wake {
            issue.status = Status::Open;
            issue.defer_until = None;
            issue.updated_at = now.clone();
        }

        let sync = self.commit_many(&to_wake).await?;
        Ok(Mutated { value: Undeferred { undeferred: to_wake }, sync })
    }

    pub async fn comment(&self, query: &str, text: &str) -> Result<Mutated<CommentAdded>> {
        let skein = self.hydrate()?;
        let mut issue = resolve_issue(&skein, query)?.clone();

        let now = now_rfc3339();
        let mut comment = Comment {
            id: new_comment_id(),
            author: self.opened.cfg.author.clone(),
            created_at: now.clone(),
            text: text.to_string(),
        };
        while issue.comments.contains_key(&comment.id) {
            comment.id = new_comment_id();
        }
        issue.comments.insert(comment.id.clone(), comment.clone());
        issue.updated_at = now;

        let issue_id = issue.id.clone();
        let sync = self.commit_one(&issue).await?;
        Ok(Mutated { value: CommentAdded { issue_id, comment }, sync })
    }

    /// Remove strands entirely. Deletion is the sharpest mutation braid
    /// has — the pinned merge semantics say a delete wins over concurrent
    /// edits — so strands that other strands still reference are guarded
    /// behind `force` (mirroring close's open-children protection).
    pub async fn delete(&self, queries: &[String], force: bool) -> Result<Mutated<Deleted>> {
        let skein = self.hydrate()?;

        // Resolve and validate everything before mutating anything (atomic
        // on bad input, same discipline as close/import).
        let doomed: Vec<String> = queries
            .iter()
            .map(|q| resolve_issue(&skein, q).map(|i| i.id.clone()))
            .collect::<Result<_>>()?;

        let mut dangling = Vec::new();
        for id in &doomed {
            let dependents: Vec<&Issue> = dependents_of(&skein, id)
                .into_iter()
                // strands deleted in the same invocation don't count
                .filter(|d| !doomed.contains(&d.id))
                .collect();
            if !dependents.is_empty() && !force {
                let ids: Vec<&str> = dependents.iter().map(|i| i.id.as_str()).collect();
                bail!(
                    "{id} is referenced by {}; deleting it would leave dangling \
                     edges. Remove the dependencies first (`braid dep remove`) or \
                     pass --force.",
                    ids.join(", ")
                );
            }
            if !dependents.is_empty() {
                dangling.push(DanglingNote {
                    deleted_id: id.clone(),
                    dependents: dependents.iter().map(|i| i.id.clone()).collect(),
                });
            }
        }

        self.opened.doc.with_document(|d| {
            d.transact(|tx| {
                for id in &doomed {
                    delete_issue(tx, id)?;
                }
                Ok::<_, braid_core::amdoc::ReconcileError>(())
            })
            .map_err(|f| f.error)
        })?;
        let sync = self.opened.push().await;
        Ok(Mutated { value: Deleted { deleted: doomed, dangling }, sync })
    }

    pub async fn dep_add(&self, from: &str, to: &str, dep_type: &str) -> Result<Mutated<DepAdded>> {
        let mut skein = self.hydrate()?;

        let mut issue = resolve_issue(&skein, from)?.clone();
        let target_id = resolve_issue(&skein, to)?.id.clone();
        if issue.id == target_id {
            bail!("{} cannot depend on itself", issue.id);
        }

        let dep = Dependency {
            depends_on_id: target_id,
            dep_type: DependencyType::from(dep_type),
            created_at: now_rfc3339(),
            created_by: self.opened.cfg.author.clone(),
        };
        let key = dep.key();
        issue.dependencies.insert(key.clone(), dep);
        issue.updated_at = now_rfc3339();

        // Cycle check against the would-be state: allowed (concurrent
        // merges can create cycles regardless), but loudly surfaced.
        skein.issues.insert(issue.id.clone(), issue.clone());
        let cycles = dependency_cycles(&skein);

        let sync = self.commit_one(&issue).await?;
        Ok(Mutated { value: DepAdded { issue, key, cycles }, sync })
    }

    pub async fn dep_remove(
        &self,
        from: &str,
        to: &str,
        dep_type: Option<&str>,
    ) -> Result<Mutated<Issue>> {
        let skein = self.hydrate()?;

        let mut issue = resolve_issue(&skein, from)?.clone();
        let target_id = resolve_issue(&skein, to)?.id.clone();

        let before = issue.dependencies.len();
        issue.dependencies.retain(|_, d| {
            let type_matches = dep_type.is_none_or(|t| d.dep_type.as_str() == t);
            !(d.depends_on_id == target_id && type_matches)
        });
        if issue.dependencies.len() == before {
            bail!("{} has no dependency on {target_id}", issue.id);
        }
        issue.updated_at = now_rfc3339();

        let sync = self.commit_one(&issue).await?;
        Ok(Mutated { value: issue, sync })
    }

    /// Upsert pre-parsed strands (per-issue transactions: reads inside one
    /// giant automerge transaction are severely superlinear). `skipped` is
    /// the count of beads tombstones the parse step dropped; it is reported
    /// back verbatim, not recomputed here.
    pub async fn import(&self, issues: &[Issue], skipped: usize) -> Result<Mutated<Imported>> {
        self.guard_rotation()?;
        self.opened.doc.with_document(|d| {
            for issue in issues {
                d.transact(|tx| reconcile_issue(tx, issue)).map_err(|f| f.error)?;
            }
            Ok::<_, braid_core::amdoc::ReconcileError>(())
        })?;
        let sync = self.opened.push().await;
        Ok(Mutated { value: Imported { imported: issues.len(), skipped }, sync })
    }
}
