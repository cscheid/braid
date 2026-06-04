//! CLI command surface.
//!
//! Two kinds of functions live here: **operator commands** (init, secret,
//! rotate, adopt) that manage the skein's lifecycle and secret directly,
//! and **printers** that delegate all domain logic to [`crate::ops`] (the
//! layer shared with the MCP server) and only format output.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use automerge::Automerge;
use braid_core::amdoc::{hydrate, init_skein, reconcile_issue};
use braid_core::schema::{Issue, SCHEMA_VERSION, SkeinMetadata};
use braid_core::time::now_rfc3339;
use samod::DocumentId;

use crate::config::{DEFAULT_SYNC_SERVER, REPO_FILE_NAME, SecretSource};
use crate::ops::{self, Session};
use crate::skein::{PushOutcome, open_repo, open_skein, open_skein_unchecked};
use crate::sync::{Connect, connect, sync_timeout};

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
                .unwrap_or_else(|| "skein".to_string());
            let meta = SkeinMetadata {
                schema_version: SCHEMA_VERSION,
                name,
                id_prefix: opts.prefix,
                created_at: now_rfc3339(),
        rotated_at: None,
        rotated_to: None,
            };
            let mut doc = Automerge::new();
            doc.transact(|tx| init_skein(tx, &meta)).map_err(|f| f.error)?;

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
                        println!("announced new skein to {sync_server}");
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
                         skein will be announced on the first successful `braid sync`"
                    );
                }
            }

            // Wait for the document to be flushed to the cache.
            repo.stop().await;
            id
        }
    };

    if opts.print_only {
        print!("{}", secret_file_contents(&doc_id, &sync_server));
        return Ok(());
    }

    write_secret_file(&secret_path, &doc_id, &sync_server)?;

    // Print only a redacted prefix: full ids in stdout end up in CI logs
    // and agent transcripts, and the id is a bearer capability. `braid
    // secret` is the explicit disclosure path.
    let redacted = crate::docid::DocId::new(doc_id).redacted();
    match opts.join {
        Some(_) => println!("joined skein {redacted}"),
        None => println!("created skein {redacted}"),
    }
    println!("wrote {} (run `braid secret` to display the full doc id)", secret_path.display());
    println!(
        "reminder: add `{REPO_FILE_NAME}` to your .gitignore — the doc id grants \
         read/write access to this skein"
    );
    Ok(())
}

/// Print the skein secret, deliberately. The TOML on stdout is paste-ready
/// for another machine's `.braid.toml`; the warning goes to stderr so
/// piping stays clean.
pub fn secret(cwd: &Path) -> Result<()> {
    let cfg = crate::config::load(cwd)?;
    eprintln!(
        "braid: this output grants read/write access to the skein — share deliberately"
    );
    println!("doc_id = \"{}\"", cfg.doc_id.expose_secret());
    println!("sync_server = \"{}\"", cfg.sync_server);
    Ok(())
}

/// The canonical `.braid.toml` contents.
fn secret_file_contents(doc_id: &str, sync_server: &str) -> String {
    format!(
        "# braid skein secret — do NOT commit this file.\n\
         # The doc_id is a bearer token: anyone holding it can read and write\n\
         # this skein. Ensure `{REPO_FILE_NAME}` is listed in .gitignore.\n\
         doc_id = \"{doc_id}\"\n\
         sync_server = \"{sync_server}\"\n"
    )
}

/// Write a `.braid.toml` (mode 600) — used by init, rotate, and adopt.
fn write_secret_file(path: &Path, doc_id: &str, sync_server: &str) -> Result<()> {
    let contents = secret_file_contents(doc_id, sync_server);
    std::fs::write(path, &contents)
        .with_context(|| format!("cannot write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// rotate / rotate --adopt
// ---------------------------------------------------------------------------

/// Point this clone's configuration at `new_doc_id`. Rewrites the
/// `.braid.toml` that supplied the old id when possible; otherwise prints
/// the paste-ready TOML (the operator needs the new secret, so this is a
/// deliberate disclosure, flagged on stderr — same contract as
/// `braid secret`).
fn switch_config_to(source: &SecretSource, new_doc_id: &str, sync_server: &str) -> Result<()> {
    match source {
        SecretSource::RepoFile(path) => {
            write_secret_file(path, new_doc_id, sync_server)?;
            println!("updated {}", path.display());
        }
        other => {
            let what = match other {
                SecretSource::Env => "the BRAID_DOC_ID environment variable",
                SecretSource::UserConfig { project } => {
                    eprintln!(
                        "braid: update [projects.{project}] in ~/.config/braid/projects.toml"
                    );
                    "your user-level config"
                }
                SecretSource::RepoFile(_) => unreachable!(),
            };
            eprintln!(
                "braid: this clone's doc id comes from {what}, which braid cannot \
                 rewrite. New secret follows on stdout — it grants read/write \
                 access; share deliberately."
            );
            println!("doc_id = \"{new_doc_id}\"");
            println!("sync_server = \"{sync_server}\"");
        }
    }
    Ok(())
}

/// Rotate the skein: export current state into a fresh document, mark the
/// old one rotated, and switch this clone over. Compact mode (default)
/// records a forwarding pointer in the old document so stale clones can
/// `--adopt`; `--revoke` deliberately does not (the old id is presumed
/// leaked, and a pointer would hand the attacker the new capability).
///
/// See claude-notes/plans/2026/06/04/braid-rotate.md for the design.
pub async fn rotate(cwd: &Path, revoke: bool) -> Result<()> {
    // The rotation check in open_skein also protects us: rotating an
    // already-rotated skein is refused there.
    let opened = open_skein(cwd).await?;
    if opened.conn.is_none() {
        let reason = opened.offline_reason.clone().unwrap_or_default();
        opened.close().await;
        bail!(
            "rotation requires a confirmed connection to the sync server \
             ({reason}).\nA rotation cut over while offline would fork from \
             stale state; retry when connected."
        );
    }
    let conn = opened.conn.expect("checked above");
    let old_state = opened.doc.with_document(|d| hydrate(d))?;
    let strand_count = old_state.issues.len();
    let now = now_rfc3339();

    // 1. Build the successor document: same identity, fresh history.
    let new_meta = SkeinMetadata {
        schema_version: SCHEMA_VERSION,
        name: old_state.metadata.name.clone(),
        id_prefix: old_state.metadata.id_prefix.clone(),
        created_at: now.clone(),
        rotated_at: None,
        rotated_to: None,
    };
    let mut new_doc = Automerge::new();
    new_doc.transact(|tx| init_skein(tx, &new_meta)).map_err(|f| f.error)?;
    for issue in old_state.issues.values() {
        // per-issue transactions: reads inside one giant automerge
        // transaction are superlinear (same lesson as import)
        new_doc.transact(|tx| reconcile_issue(tx, issue)).map_err(|f| f.error)?;
    }

    // 2. Create it in the repo and wait until the server confirms receipt.
    let new_handle = opened
        .repo
        .create(new_doc)
        .await
        .map_err(|_| anyhow!("samod repo stopped unexpectedly"))?;
    let new_doc_id = new_handle.document_id().to_string();
    let confirmed = tokio::time::timeout(sync_timeout(), new_handle.they_have_our_changes(conn))
        .await
        .is_ok();
    if !confirmed {
        opened.close().await;
        bail!(
            "the server did not confirm receipt of the new skein in time; \
             rotation aborted — nothing was changed (an unused document may \
             remain on the server)."
        );
    }

    // 3. Mark the old document rotated. Compact mode records the successor
    //    id; revoke mode must not.
    let mut rotated_meta = old_state.metadata.clone();
    rotated_meta.rotated_at = Some(now.clone());
    rotated_meta.rotated_to = if revoke { None } else { Some(new_doc_id.clone()) };
    opened
        .doc
        .with_document(|d| d.transact(|tx| init_skein(tx, &rotated_meta)).map_err(|f| f.error))?;
    let marker_confirmed =
        tokio::time::timeout(sync_timeout(), opened.doc.they_have_our_changes(conn))
            .await
            .is_ok();
    if !marker_confirmed {
        opened.close().await;
        bail!(
            "the new skein is on the server, but the rotation marker on the old \
             skein was not confirmed; other clones may keep writing to the old \
             document. Re-run `braid rotate` (this will create another fresh \
             document) or retry when the connection is stable."
        );
    }

    // 4. Cut this clone over.
    switch_config_to(&opened.cfg.doc_id_source, &new_doc_id, &opened.cfg.sync_server)?;
    let old_redacted = opened.cfg.doc_id.redacted();
    let new_redacted = crate::docid::DocId::new(new_doc_id).redacted();
    opened.close().await;

    println!(
        "rotated skein {old_redacted} -> {new_redacted} ({strand_count} strand{} carried over)",
        if strand_count == 1 { "" } else { "s" }
    );
    if revoke {
        println!(
            "revoke mode: no forwarding pointer was written. Distribute the new \
             secret out-of-band (`braid secret`) to every participant; stale \
             clones will see a rotation error until they are reconfigured."
        );
    } else {
        println!(
            "stale clones will be prompted to run `braid rotate --adopt` on \
             their next command."
        );
    }
    println!(
        "note: rotation does not erase the old document — anyone holding the \
         old id retains read access to its (now frozen) history."
    );
    Ok(())
}

/// Follow a compact rotation's forwarding pointer: switch this clone's
/// configuration to the successor document, surfacing any "straggler"
/// strands that were written to the old skein after the rotation.
pub async fn rotate_adopt(cwd: &Path) -> Result<()> {
    let opened = open_skein_unchecked(cwd).await?;
    let old_state = opened.doc.with_document(|d| hydrate(d))?;

    let Some(rotated_at) = old_state.metadata.rotated_at.clone() else {
        opened.close().await;
        bail!("this skein has not been rotated; nothing to adopt");
    };
    let Some(new_doc_id) = old_state.metadata.rotated_to.clone() else {
        opened.close().await;
        bail!(
            "this skein was rotated with --revoke: the successor id was \
             deliberately not recorded. Obtain the new secret out-of-band \
             (`braid secret` on an up-to-date machine)."
        );
    };

    // Straggler detection (D-R6): strands modified after the rotation
    // instant were written into the dead document. Unparseable timestamps
    // are conservatively included.
    let stragglers: Vec<&Issue> = old_state
        .issues
        .values()
        .filter(|i| braid_core::time::is_after(&i.updated_at, &rotated_at).unwrap_or(true))
        .collect();
    if !stragglers.is_empty() {
        let path = cwd.join(".braid-stragglers.jsonl");
        let mut out = String::new();
        for issue in &stragglers {
            out.push_str(&serde_json::to_string(issue)?);
            out.push('\n');
        }
        std::fs::write(&path, out)
            .with_context(|| format!("cannot write {}", path.display()))?;
        let ids: Vec<&str> = stragglers.iter().map(|i| i.id.as_str()).collect();
        eprintln!(
            "braid: {} straggler strand{} modified in the old skein after \
             rotation: {}\nWritten to {} — review and `braid import` what \
             should carry over (importing overwrites same-id strands in the \
             new skein).",
            stragglers.len(),
            if stragglers.len() == 1 { "" } else { "s" },
            ids.join(", "),
            path.display()
        );
    }

    switch_config_to(&opened.cfg.doc_id_source, &new_doc_id, &opened.cfg.sync_server)?;
    opened.close().await;

    // Verify the successor loads with the new configuration.
    let adopted = open_skein(cwd).await?;
    let skein = adopted.doc.with_document(|d| hydrate(d))?;
    let n = skein.issues.len();
    let redacted = adopted.cfg.doc_id.redacted();
    adopted.close().await;
    println!(
        "adopted rotation: now on skein {redacted} ({n} strand{})",
        if n == 1 { "" } else { "s" }
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI printers over ops::Session
//
// Everything below is presentation: open a Session, call the operation,
// print. Domain logic lives in crate::ops (shared with the MCP server).
// ---------------------------------------------------------------------------

/// CLI replica of the old push_and_close warning: connected but the
/// server didn't confirm receipt within the timeout.
fn warn_unconfirmed(sync: &PushOutcome) {
    if matches!(sync, PushOutcome::Unconfirmed) {
        eprintln!(
            "braid: changes saved locally, but the server did not confirm \
             receipt in time; run `braid sync` later to be sure"
        );
    }
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
    let session = Session::open(cwd).await?;
    let result = session
        .create(ops::CreateOpts {
            title: opts.title,
            description: opts.description,
            issue_type: opts.issue_type,
            priority: opts.priority,
            labels: opts.labels,
            slug: opts.slug,
            assignee: opts.assignee,
        })
        .await?;
    session.shutdown().await;
    warn_unconfirmed(&result.sync);

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&result.value)?);
    } else {
        println!("{}", result.value.id);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

pub async fn show(cwd: &Path, query: &str, json: bool) -> Result<()> {
    let session = Session::open(cwd).await?;
    let issue = session.show(query);
    session.shutdown().await;
    let issue = issue?;

    if json {
        println!("{}", serde_json::to_string_pretty(&issue)?);
    } else {
        print!("{}", format_issue(&issue));
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
    if let Some(t) = &issue.defer_until {
        let _ = writeln!(out, "wakes:     {t}");
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
    let json = opts.json;
    let session = Session::open(cwd).await?;
    let result = session
        .update(
            query,
            ops::UpdateOpts {
                title: opts.title,
                description: opts.description,
                design: opts.design,
                acceptance_criteria: opts.acceptance_criteria,
                notes: opts.notes,
                status: opts.status,
                priority: opts.priority,
                issue_type: opts.issue_type,
                assignee: opts.assignee,
                external_ref: opts.external_ref,
                add_labels: opts.add_labels,
                remove_labels: opts.remove_labels,
            },
        )
        .await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    if json {
        println!("{}", serde_json::to_string_pretty(&result.value)?);
    } else {
        println!("{}", result.value.id);
    }
    Ok(())
}

pub async fn close(cwd: &Path, queries: &[String], reason: Option<String>, force: bool) -> Result<()> {
    let session = Session::open(cwd).await?;
    let result = session.close_strands(queries, reason, force).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    for issue in &result.value.closed {
        println!("{}", issue.id);
    }
    Ok(())
}

pub async fn reopen(cwd: &Path, queries: &[String]) -> Result<()> {
    let session = Session::open(cwd).await?;
    let result = session.reopen(queries).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    for issue in &result.value.reopened {
        println!("{}", issue.id);
    }
    Ok(())
}

pub async fn defer(cwd: &Path, queries: &[String], until: Option<String>) -> Result<()> {
    let session = Session::open(cwd).await?;
    let result = session.defer(queries, until).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    for issue in &result.value.deferred {
        println!("{}", issue.id);
    }
    Ok(())
}

pub async fn undefer(cwd: &Path, queries: &[String]) -> Result<()> {
    let session = Session::open(cwd).await?;
    let result = session.undefer(queries).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    for issue in &result.value.undeferred {
        println!("{}", issue.id);
    }
    Ok(())
}

pub async fn comment(cwd: &Path, query: &str, text: &str) -> Result<()> {
    let session = Session::open(cwd).await?;
    let result = session.comment(query, text).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    println!("{}", result.value.comment.id);
    Ok(())
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

pub async fn delete(cwd: &Path, queries: &[String], force: bool) -> Result<()> {
    let session = Session::open(cwd).await?;
    let result = session.delete(queries, force).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    for note in &result.value.dangling {
        eprintln!(
            "braid: {} deleted; {} now hold{} dangling edges to it \
             (harmless: they never block)",
            note.deleted_id,
            note.dependents.join(", "),
            if note.dependents.len() == 1 { "s" } else { "" }
        );
    }
    for id in &result.value.deleted {
        println!("{id}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// dependencies: dep add / remove / list / cycles
// ---------------------------------------------------------------------------

pub async fn dep_add(cwd: &Path, from: &str, to: &str, dep_type: &str) -> Result<()> {
    let session = Session::open(cwd).await?;
    let result = session.dep_add(from, to, dep_type).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    for cycle in &result.value.cycles {
        eprintln!("braid: warning: dependency cycle: {}", cycle.join(" -> "));
    }
    println!("{}: {}", result.value.issue.id, result.value.key);
    Ok(())
}

pub async fn dep_remove(cwd: &Path, from: &str, to: &str, dep_type: Option<String>) -> Result<()> {
    let session = Session::open(cwd).await?;
    let result = session.dep_remove(from, to, dep_type.as_deref()).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    println!("{}", result.value.id);
    Ok(())
}

pub async fn dep_list(cwd: &Path, query: &str) -> Result<()> {
    let session = Session::open(cwd).await?;
    let listing = session.dep_list(query);
    session.shutdown().await;
    let listing = listing?;

    for n in &listing.outgoing {
        println!("outgoing  {} ({}) [{}]", n.id, n.dep_type, n.status);
    }
    for n in &listing.incoming {
        println!("incoming  {} ({}) [{}]", n.id, n.dep_type, n.status);
    }
    Ok(())
}

pub async fn dep_cycles(cwd: &Path) -> Result<()> {
    let session = Session::open(cwd).await?;
    let cycles = session.dep_cycles();
    session.shutdown().await;
    let cycles = cycles?;

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

/// Width-aligned human listing. Column widths are computed from the data
/// (slugged ids are much longer than bare ones). On a TTY a bold header
/// and a strand-count footer are added; piped output stays data-rows-only
/// so `braid list | wc -l` and grep/awk keep working.
fn print_listing(issues: &[Issue]) {
    use std::io::IsTerminal;

    if issues.is_empty() {
        return;
    }
    let id_w = issues.iter().map(|i| i.id.len()).max().unwrap_or(2).max(2);
    let ty_w = issues.iter().map(|i| i.issue_type.as_str().len()).max().unwrap_or(4).max(4);
    let st_w = issues.iter().map(|i| i.status.as_str().len()).max().unwrap_or(6).max(6);

    let tty = std::io::stdout().is_terminal();
    if tty {
        println!(
            "\x1b[1m{:<id_w$}  {:<4} {:<ty_w$}  {:<st_w$}  TITLE\x1b[0m",
            "ID", "PRI", "TYPE", "STATUS"
        );
    }
    for i in issues {
        let wake = match &i.defer_until {
            Some(t) => format!("  [wakes {t}]"),
            None => String::new(),
        };
        println!(
            "{:<id_w$}  P{:<3} {:<ty_w$}  {:<st_w$}  {}{}",
            i.id,
            i.priority,
            i.issue_type.as_str(),
            i.status.as_str(),
            i.title,
            wake
        );
    }
    if tty {
        let n = issues.len();
        println!("\n{n} strand{}", if n == 1 { "" } else { "s" });
    }
}

pub async fn ready(cwd: &Path, json: bool) -> Result<()> {
    let session = Session::open(cwd).await?;
    let ready = session.ready();
    session.shutdown().await;
    let ready = ready?;

    if json {
        println!("{}", serde_json::to_string_pretty(&ready)?);
    } else {
        print_listing(&ready);
    }
    Ok(())
}

pub async fn blocked(cwd: &Path, json: bool) -> Result<()> {
    let session = Session::open(cwd).await?;
    let blocked = session.blocked();
    session.shutdown().await;
    let blocked = blocked?;

    if json {
        println!("{}", serde_json::to_string_pretty(&blocked)?);
    } else if !blocked.is_empty() {
        let id_w = blocked.iter().map(|b| b.issue.id.len()).max().unwrap_or(2);
        let st_w =
            blocked.iter().map(|b| b.issue.status.as_str().len()).max().unwrap_or(6).max(6);
        let title_w = blocked.iter().map(|b| b.issue.title.len()).max().unwrap_or(0);
        for b in blocked {
            println!(
                "{:<id_w$}  P{:<3} {:<st_w$}  {:<title_w$}  [blocked by {}]",
                b.issue.id,
                b.issue.priority,
                b.issue.status.as_str(),
                b.issue.title,
                b.blocked_by.join(", ")
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// import / export
// ---------------------------------------------------------------------------

pub async fn import(cwd: &Path, path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    // Parse everything before touching the document: imports are atomic.
    let issues = crate::import::parse_jsonl(&text)?;

    let session = Session::open(cwd).await?;
    let result = session.import(&issues).await;
    session.shutdown().await;
    let result = result?;
    warn_unconfirmed(&result.sync);

    println!("imported {} strands from {}", result.value.imported, path.display());
    Ok(())
}

pub async fn export(cwd: &Path) -> Result<()> {
    let session = Session::open(cwd).await?;
    let jsonl = session.export_jsonl();
    session.shutdown().await;
    print!("{}", jsonl?);
    Ok(())
}

// ---------------------------------------------------------------------------
// search
// ---------------------------------------------------------------------------

pub async fn search(cwd: &Path, needle: &str, json: bool) -> Result<()> {
    let session = Session::open(cwd).await?;
    let found = session.search(needle);
    session.shutdown().await;
    let found = found?;

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
    let session = Session::open(cwd).await?;
    if !session.is_online() {
        let reason = session.offline_reason().unwrap_or_default().to_string();
        session.shutdown().await;
        bail!("offline: {reason}");
    }
    let count = session.strand_count();
    let server = session.sync_server().to_string();
    // an explicit sync wants the push barrier even with no local changes
    let outcome = session.push().await;
    session.shutdown().await;
    let count = count?;
    warn_unconfirmed(&outcome);
    println!("synced with {server} ({count} strands)");
    Ok(())
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

pub async fn list(cwd: &Path, status: Option<String>, json: bool) -> Result<()> {
    let session = Session::open(cwd).await?;
    let issues = session.list(status.as_deref());
    session.shutdown().await;
    let issues = issues?;

    if json {
        println!("{}", serde_json::to_string_pretty(&issues)?);
    } else {
        print_listing(&issues);
    }
    Ok(())
}
