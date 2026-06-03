//! CLI command implementations (Phase 1: local-only).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use automerge::Automerge;
use braid_core::amdoc::{hydrate, init_tracker, reconcile_issue};
use braid_core::domain::{
    blocked_issues, dependency_cycles, dependents_of, open_children, ready_issues,
};
use braid_core::id::{new_comment_id, new_issue_id};
use braid_core::schema::{
    Comment, Dependency, DependencyType, Issue, IssueType, SCHEMA_VERSION, Status, TrackerDoc,
    TrackerMetadata,
};
use braid_core::time::now_rfc3339;
use samod::DocumentId;

use crate::config::{DEFAULT_SYNC_SERVER, REPO_FILE_NAME};
use crate::sync::{Connect, connect, sync_timeout};
use crate::tracker::{open_repo, open_tracker};

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

pub struct InitOpts {
    pub name: Option<String>,
    pub prefix: String,
    pub join: Option<String>,
    pub sync_server: Option<String>,
    pub print_only: bool,
}

pub async fn init(cwd: &Path, opts: InitOpts) -> Result<()> {
    let secret_path = cwd.join(REPO_FILE_NAME);
    if !opts.print_only && secret_path.exists() {
        bail!(
            "{} already exists — this directory is already configured.\n\
             Remove it first if you intend to re-initialize.",
            secret_path.display()
        );
    }
    let sync_server = opts.sync_server.unwrap_or_else(|| DEFAULT_SYNC_SERVER.to_string());

    let doc_id = match &opts.join {
        Some(id) => {
            let parsed: DocumentId = id
                .parse()
                .map_err(|e| anyhow!("--join {id:?} is not a valid document id: {e:?}"))?;
            parsed.to_string()
        }
        None => {
            let name = opts
                .name
                .or_else(|| {
                    cwd.file_name().map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "tracker".to_string());
            let meta = TrackerMetadata {
                schema_version: SCHEMA_VERSION,
                name,
                id_prefix: opts.prefix,
                created_at: now_rfc3339(),
            };
            let mut doc = Automerge::new();
            doc.transact(|tx| init_tracker(tx, &meta)).map_err(|f| f.error)?;

            let repo = open_repo().await?;
            let handle = repo
                .create(doc)
                .await
                .map_err(|_| anyhow!("samod repo stopped unexpectedly"))?;
            let id = handle.document_id().to_string();

            // Best-effort announce to the sync server (D13: init works
            // offline; the doc reaches the server on first successful sync).
            match connect(&repo, &sync_server, sync_timeout()).await? {
                Connect::Connected { conn, .. } => {
                    let confirmed =
                        tokio::time::timeout(sync_timeout(), handle.they_have_our_changes(conn))
                            .await
                            .is_ok();
                    if confirmed {
                        println!("announced new tracker to {sync_server}");
                    } else {
                        eprintln!(
                            "braid: created locally; {sync_server} did not confirm receipt \
                             in time — run `braid sync` later"
                        );
                    }
                }
                Connect::Offline(reason) => {
                    eprintln!(
                        "braid: created locally; server unreachable ({reason}) — the \
                         tracker will be announced on the first successful `braid sync`"
                    );
                }
            }

            // Wait for the document to be flushed to the cache.
            repo.stop().await;
            id
        }
    };

    let contents = format!(
        "# braid tracker secret — do NOT commit this file.\n\
         # The doc_id is a bearer token: anyone holding it can read and write\n\
         # this tracker. Ensure `{REPO_FILE_NAME}` is listed in .gitignore.\n\
         doc_id = \"{doc_id}\"\n\
         sync_server = \"{sync_server}\"\n"
    );

    if opts.print_only {
        print!("{contents}");
        return Ok(());
    }

    std::fs::write(&secret_path, &contents)
        .with_context(|| format!("cannot write {}", secret_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o600))?;
    }

    match opts.join {
        Some(_) => println!("joined tracker {doc_id}"),
        None => println!("created tracker {doc_id}"),
    }
    println!("wrote {}", secret_path.display());
    println!(
        "reminder: add `{REPO_FILE_NAME}` to your .gitignore — the doc id grants \
         read/write access to this tracker"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// create
// ---------------------------------------------------------------------------

pub struct CreateOpts {
    pub title: String,
    pub description: Option<String>,
    pub issue_type: String,
    pub priority: i64,
    pub labels: Vec<String>,
    pub slug: Option<String>,
    pub assignee: Option<String>,
    pub json: bool,
}

pub async fn create(cwd: &Path, opts: CreateOpts) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;

    let mut id = new_issue_id(&tracker.metadata.id_prefix, opts.slug.as_deref());
    // Collision with an existing id is astronomically improbable but free
    // to guard against locally:
    while tracker.issues.contains_key(&id) {
        id = new_issue_id(&tracker.metadata.id_prefix, opts.slug.as_deref());
    }

    let now = now_rfc3339();
    let issue = Issue {
        id: id.clone(),
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
        created_by: opened.cfg.author.clone(),
        updated_at: now,
        closed_at: None,
        close_reason: None,
        external_ref: None,
        labels: opts.labels.into_iter().collect::<BTreeSet<_>>(),
        dependencies: BTreeMap::new(),
        comments: BTreeMap::new(),
    };

    opened
        .doc
        .with_document(|d| d.transact(|tx| reconcile_issue(tx, &issue)).map_err(|f| f.error))?;
    opened.push_and_close().await;

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&issue)?);
    } else {
        println!("{id}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

/// Resolve a user-supplied id query: exact match first, then unique
/// substring match.
pub fn resolve_issue<'t>(tracker: &'t TrackerDoc, query: &str) -> Result<&'t Issue> {
    if let Some(issue) = tracker.issues.get(query) {
        return Ok(issue);
    }
    let matches: Vec<&Issue> =
        tracker.issues.values().filter(|i| i.id.contains(query)).collect();
    match matches.len() {
        0 => bail!("no issue matching {query:?}"),
        1 => Ok(matches[0]),
        _ => {
            let ids: Vec<&str> = matches.iter().map(|i| i.id.as_str()).collect();
            bail!("ambiguous id {query:?}: matches {}", ids.join(", "));
        }
    }
}

pub async fn show(cwd: &Path, query: &str, json: bool) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.close().await;

    let issue = resolve_issue(&tracker, query)?;
    if json {
        println!("{}", serde_json::to_string_pretty(issue)?);
    } else {
        print!("{}", format_issue(issue));
    }
    Ok(())
}

fn format_issue(issue: &Issue) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "id:        {}", issue.id);
    let _ = writeln!(out, "title:     {}", issue.title);
    let _ = writeln!(out, "status:    {}", issue.status);
    let _ = writeln!(out, "priority:  {}", issue.priority);
    let _ = writeln!(out, "type:      {}", issue.issue_type);
    if let Some(a) = &issue.assignee {
        let _ = writeln!(out, "assignee:  {a}");
    }
    let _ = writeln!(out, "created:   {} by {}", issue.created_at, issue.created_by);
    let _ = writeln!(out, "updated:   {}", issue.updated_at);
    if let Some(t) = &issue.closed_at {
        let reason = issue.close_reason.as_deref().unwrap_or("");
        let _ = writeln!(out, "closed:    {t} {reason}");
    }
    if let Some(r) = &issue.external_ref {
        let _ = writeln!(out, "ref:       {r}");
    }
    if !issue.labels.is_empty() {
        let labels: Vec<&str> = issue.labels.iter().map(String::as_str).collect();
        let _ = writeln!(out, "labels:    {}", labels.join(", "));
    }
    for dep in issue.dependencies.values() {
        let _ = writeln!(out, "dep:       {} ({})", dep.depends_on_id, dep.dep_type);
    }
    if let Some(d) = &issue.description {
        let _ = writeln!(out, "\n{d}");
    }
    for c in issue.comments.values() {
        let _ = writeln!(out, "\n--- comment {} by {} at {}\n{}", c.id, c.author, c.created_at, c.text);
    }
    out
}

// ---------------------------------------------------------------------------
// mutation commands: update, close, reopen, comment
// ---------------------------------------------------------------------------

/// Empty strings clear optional fields (`--description "" ` removes the
/// description); `None` leaves the field untouched.
fn apply_opt(field: &mut Option<String>, flag: Option<String>) {
    if let Some(v) = flag {
        *field = if v.is_empty() { None } else { Some(v) };
    }
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
    pub json: bool,
}

pub async fn update(cwd: &Path, query: &str, opts: UpdateOpts) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    let mut issue = resolve_issue(&tracker, query)?.clone();

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

    opened
        .doc
        .with_document(|d| d.transact(|tx| reconcile_issue(tx, &issue)).map_err(|f| f.error))?;
    opened.push_and_close().await;

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&issue)?);
    } else {
        println!("{}", issue.id);
    }
    Ok(())
}

pub async fn close(cwd: &Path, queries: &[String], reason: Option<String>, force: bool) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;

    // Resolve and validate everything before mutating anything.
    let mut to_close: Vec<Issue> = Vec::new();
    let closing_now: Vec<String> = queries
        .iter()
        .map(|q| resolve_issue(&tracker, q).map(|i| i.id.clone()))
        .collect::<Result<_>>()?;
    for query in queries {
        let issue = resolve_issue(&tracker, query)?;
        let open_kids: Vec<&Issue> = open_children(&tracker, &issue.id)
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
        issue.updated_at = now.clone();
    }

    opened.doc.with_document(|d| {
        d.transact(|tx| {
            for issue in &to_close {
                reconcile_issue(tx, issue)?;
            }
            Ok::<_, braid_core::amdoc::ReconcileError>(())
        })
        .map_err(|f| f.error)
    })?;
    opened.push_and_close().await;

    for issue in &to_close {
        println!("{}", issue.id);
    }
    Ok(())
}

pub async fn reopen(cwd: &Path, queries: &[String]) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;

    let mut to_reopen: Vec<Issue> = queries
        .iter()
        .map(|q| resolve_issue(&tracker, q).cloned())
        .collect::<Result<_>>()?;
    let now = now_rfc3339();
    for issue in &mut to_reopen {
        issue.status = Status::Open;
        issue.closed_at = None;
        issue.close_reason = None;
        issue.updated_at = now.clone();
    }

    opened.doc.with_document(|d| {
        d.transact(|tx| {
            for issue in &to_reopen {
                reconcile_issue(tx, issue)?;
            }
            Ok::<_, braid_core::amdoc::ReconcileError>(())
        })
        .map_err(|f| f.error)
    })?;
    opened.push_and_close().await;

    for issue in &to_reopen {
        println!("{}", issue.id);
    }
    Ok(())
}

pub async fn comment(cwd: &Path, query: &str, text: &str) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    let mut issue = resolve_issue(&tracker, query)?.clone();

    let now = now_rfc3339();
    let mut comment = Comment {
        id: new_comment_id(),
        author: opened.cfg.author.clone(),
        created_at: now.clone(),
        text: text.to_string(),
    };
    while issue.comments.contains_key(&comment.id) {
        comment.id = new_comment_id();
    }
    let comment_id = comment.id.clone();
    issue.comments.insert(comment.id.clone(), comment);
    issue.updated_at = now;

    opened
        .doc
        .with_document(|d| d.transact(|tx| reconcile_issue(tx, &issue)).map_err(|f| f.error))?;
    opened.push_and_close().await;

    println!("{comment_id}");
    Ok(())
}

// ---------------------------------------------------------------------------
// dependencies: dep add / remove / list / cycles
// ---------------------------------------------------------------------------

pub async fn dep_add(cwd: &Path, from: &str, to: &str, dep_type: &str) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let mut tracker = opened.doc.with_document(|d| hydrate(d))?;

    let mut issue = resolve_issue(&tracker, from)?.clone();
    let target_id = resolve_issue(&tracker, to)?.id.clone();
    if issue.id == target_id {
        bail!("{} cannot depend on itself", issue.id);
    }

    let dep = Dependency {
        depends_on_id: target_id,
        dep_type: DependencyType::from(dep_type),
        created_at: now_rfc3339(),
        created_by: opened.cfg.author.clone(),
    };
    let key = dep.key();
    issue.dependencies.insert(key.clone(), dep);
    issue.updated_at = now_rfc3339();

    // Cycle check against the would-be state: allowed (concurrent merges
    // can create cycles regardless), but loudly warned about.
    tracker.issues.insert(issue.id.clone(), issue.clone());
    let cycles = dependency_cycles(&tracker);
    if !cycles.is_empty() {
        for cycle in &cycles {
            eprintln!("braid: warning: dependency cycle: {}", cycle.join(" -> "));
        }
    }

    opened
        .doc
        .with_document(|d| d.transact(|tx| reconcile_issue(tx, &issue)).map_err(|f| f.error))?;
    opened.push_and_close().await;
    println!("{}: {key}", issue.id);
    Ok(())
}

pub async fn dep_remove(cwd: &Path, from: &str, to: &str, dep_type: Option<String>) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;

    let mut issue = resolve_issue(&tracker, from)?.clone();
    let target_id = resolve_issue(&tracker, to)?.id.clone();

    let before = issue.dependencies.len();
    issue.dependencies.retain(|_, d| {
        let type_matches =
            dep_type.as_ref().is_none_or(|t| d.dep_type.as_str() == t.as_str());
        !(d.depends_on_id == target_id && type_matches)
    });
    if issue.dependencies.len() == before {
        bail!("{} has no dependency on {target_id}", issue.id);
    }
    issue.updated_at = now_rfc3339();

    opened
        .doc
        .with_document(|d| d.transact(|tx| reconcile_issue(tx, &issue)).map_err(|f| f.error))?;
    opened.push_and_close().await;
    println!("{}", issue.id);
    Ok(())
}

pub async fn dep_list(cwd: &Path, query: &str) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.close().await;

    let issue = resolve_issue(&tracker, query)?;
    for dep in issue.dependencies.values() {
        let status = tracker
            .issues
            .get(&dep.depends_on_id)
            .map(|t| t.status.as_str())
            .unwrap_or("missing!");
        println!("outgoing  {} ({}) [{status}]", dep.depends_on_id, dep.dep_type);
    }
    for dependent in dependents_of(&tracker, &issue.id) {
        for dep in dependent.dependencies.values() {
            if dep.depends_on_id == issue.id {
                println!(
                    "incoming  {} ({}) [{}]",
                    dependent.id,
                    dep.dep_type,
                    dependent.status.as_str()
                );
            }
        }
    }
    Ok(())
}

pub async fn dep_cycles(cwd: &Path) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.close().await;

    let cycles = dependency_cycles(&tracker);
    if cycles.is_empty() {
        println!("no cycles");
    } else {
        for cycle in cycles {
            println!("{}", cycle.join(" -> "));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ready / blocked
// ---------------------------------------------------------------------------

fn print_listing(issues: &[&Issue]) {
    for i in issues {
        println!(
            "{}  P{} {:8} {:12} {}",
            i.id,
            i.priority,
            i.issue_type.as_str(),
            i.status.as_str(),
            i.title
        );
    }
}

pub async fn ready(cwd: &Path, json: bool) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.close().await;

    let ready = ready_issues(&tracker);
    if json {
        println!("{}", serde_json::to_string_pretty(&ready)?);
    } else {
        print_listing(&ready);
    }
    Ok(())
}

pub async fn blocked(cwd: &Path, json: bool) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.close().await;

    let blocked = blocked_issues(&tracker);
    if json {
        let rows: Vec<serde_json::Value> = blocked
            .iter()
            .map(|(issue, blockers)| {
                serde_json::json!({
                    "issue": issue,
                    "blocked_by": blockers.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        for (issue, blockers) in blocked {
            let ids: Vec<&str> = blockers.iter().map(|b| b.id.as_str()).collect();
            println!(
                "{}  P{} {:12} {}  [blocked by {}]",
                issue.id,
                issue.priority,
                issue.status.as_str(),
                issue.title,
                ids.join(", ")
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// search
// ---------------------------------------------------------------------------

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

pub async fn search(cwd: &Path, needle: &str, json: bool) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.close().await;

    let needle_lower = needle.to_lowercase();
    let mut found: Vec<&Issue> =
        tracker.issues.values().filter(|i| issue_matches(i, &needle_lower)).collect();
    found.sort_by(|a, b| braid_core::domain::listing_order(a, b));

    if json {
        println!("{}", serde_json::to_string_pretty(&found)?);
    } else {
        print_listing(&found);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// agents-info
// ---------------------------------------------------------------------------

/// The agent-facing usage guide, embedded so it is always version-matched
/// (design decision D11).
pub fn agents_info() {
    print!("{}", include_str!("agents-info.md"));
}

// ---------------------------------------------------------------------------
// sync
// ---------------------------------------------------------------------------

/// Explicit bidirectional sync. Unlike other commands, being offline here
/// is a hard failure: syncing is the entire point.
pub async fn sync(cwd: &Path) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    if opened.conn.is_none() {
        let reason = opened.offline_reason.clone().unwrap_or_default();
        opened.close().await;
        bail!("offline: {reason}");
    }
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    let server = opened.cfg.sync_server.clone();
    opened.push_and_close().await;
    println!("synced with {server} ({} issues)", tracker.issues.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

pub async fn list(cwd: &Path, status: Option<String>, json: bool) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    opened.pull().await;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.close().await;

    let mut issues: Vec<&Issue> = tracker
        .issues
        .values()
        .filter(|i| match &status {
            Some(s) => i.status.as_str() == s,
            None => true,
        })
        .collect();
    issues.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });

    if json {
        println!("{}", serde_json::to_string_pretty(&issues)?);
    } else {
        for i in issues {
            println!(
                "{}  P{} {:8} {:12} {}",
                i.id,
                i.priority,
                i.issue_type.as_str(),
                i.status.as_str(),
                i.title
            );
        }
    }
    Ok(())
}
