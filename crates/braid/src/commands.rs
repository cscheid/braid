//! CLI command implementations (Phase 1: local-only).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use automerge::Automerge;
use braid_core::amdoc::{hydrate, init_tracker, reconcile_issue};
use braid_core::id::new_issue_id;
use braid_core::schema::{
    Issue, IssueType, SCHEMA_VERSION, Status, TrackerDoc, TrackerMetadata,
};
use braid_core::time::now_rfc3339;
use samod::DocumentId;

use crate::config::{DEFAULT_SYNC_SERVER, REPO_FILE_NAME};
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
    opened.repo.stop().await;

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
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.repo.stop().await;

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
// list
// ---------------------------------------------------------------------------

pub async fn list(cwd: &Path, status: Option<String>, json: bool) -> Result<()> {
    let opened = open_tracker(cwd).await?;
    let tracker = opened.doc.with_document(|d| hydrate(d))?;
    opened.repo.stop().await;

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
