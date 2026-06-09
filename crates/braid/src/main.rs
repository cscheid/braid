use clap::{Parser, Subcommand};

use braid::commands;

#[derive(Parser)]
#[command(name = "braid", version, about = "An automerge-centric issue tracker for LLM agents")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new skein (or join an existing one) and write .braid.toml
    Init {
        /// Display name for the skein (default: current directory name)
        #[arg(long)]
        name: Option<String>,
        /// Prefix for generated issue ids
        #[arg(long, default_value = "br")]
        prefix: String,
        /// Adopt an existing skein by document id instead of creating one
        #[arg(long, value_name = "DOC_ID")]
        join: Option<String>,
        /// Sync server URL to record in .braid.toml
        #[arg(long, value_name = "URL")]
        sync_server: Option<String>,
        /// Print the .braid.toml contents to stdout instead of writing it
        #[arg(long)]
        print_only: bool,
    },
    /// Create a new strand; prints its id
    Create {
        title: String,
        #[arg(short, long)]
        description: Option<String>,
        /// Issue type: task|bug|feature|epic|chore|docs|question
        #[arg(short = 't', long = "type", default_value = "task")]
        issue_type: String,
        /// 0 (critical) .. 4 (backlog)
        #[arg(short, long, default_value_t = 2)]
        priority: i64,
        /// May be repeated
        #[arg(short, long = "label")]
        label: Vec<String>,
        /// Human-readable id segment: br-<slug>-<suffix>
        #[arg(long)]
        slug: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        /// Attach dependencies atomically as <type>:<target-id> (e.g.
        /// discovered-from:br-abc). Repeatable and comma-separated; the new
        /// strand depends on each target. A missing target fails the create.
        #[arg(long = "deps", value_delimiter = ',')]
        deps: Vec<String>,
        /// Print the full issue as JSON instead of just the id
        #[arg(long)]
        json: bool,
    },
    /// Show one strand (by id or unique id fragment)
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// List strands (open ones by default; --all includes closed)
    List {
        /// Filter by status (open|in_progress|blocked|deferred|closed)
        #[arg(long)]
        status: Option<String>,
        /// Include closed strands
        #[arg(long, conflicts_with = "status")]
        all: bool,
        /// Require a label (repeatable: a strand must carry all of them)
        #[arg(short, long = "label")]
        label: Vec<String>,
        /// Filter by exact assignee
        #[arg(long)]
        assignee: Option<String>,
        /// Filter by issue type: task|bug|feature|epic|chore|docs|question
        #[arg(short = 't', long = "type")]
        issue_type: Option<String>,
        /// Require one of these priorities 0..=4 (repeatable: a strand
        /// matches if its priority is any of them)
        #[arg(long = "priority", value_parser = clap::value_parser!(i64).range(0..=4))]
        priority: Vec<i64>,
        #[arg(long)]
        json: bool,
    },
    /// Update fields of a strand (empty string clears optional fields)
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(short, long)]
        description: Option<String>,
        #[arg(long)]
        design: Option<String>,
        #[arg(long)]
        acceptance_criteria: Option<String>,
        #[arg(long)]
        notes: Option<String>,
        /// open|in_progress|blocked|deferred|closed
        #[arg(long)]
        status: Option<String>,
        #[arg(short, long)]
        priority: Option<i64>,
        #[arg(short = 't', long = "type")]
        issue_type: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        external_ref: Option<String>,
        /// May be repeated
        #[arg(long = "add-label")]
        add_label: Vec<String>,
        /// May be repeated
        #[arg(long = "remove-label")]
        remove_label: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Close one or more strands
    Close {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        reason: Option<String>,
        /// Close even if the issue still has open children
        #[arg(long)]
        force: bool,
    },
    /// Reopen closed strands
    Reopen {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Defer strands; once --until passes they count as ready again
    Defer {
        #[arg(required = true)]
        ids: Vec<String>,
        /// Wake time: RFC 3339 (2026-07-01T09:00:00Z), date (2026-07-01),
        /// or duration from now (36h, 7d, 2w). Omitted: sleeps until
        /// `braid undefer`.
        #[arg(long)]
        until: Option<String>,
    },
    /// Wake deferred strands now (status back to open)
    Undefer {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Delete strands entirely (a delete wins over concurrent edits)
    Delete {
        #[arg(required = true)]
        ids: Vec<String>,
        /// Delete even if other strands still reference these (leaves
        /// dangling, non-blocking edges)
        #[arg(long)]
        force: bool,
    },
    /// Add a comment to a strand; prints the comment id
    Comment { id: String, text: String },
    /// Manage dependencies between strands
    Dep {
        #[command(subcommand)]
        cmd: DepCmd,
    },
    /// List strands that are ready to work on (active, unblocked)
    Ready {
        /// Require a label (repeatable: a strand must carry all of them)
        #[arg(short, long = "label")]
        label: Vec<String>,
        /// Filter by exact assignee
        #[arg(long)]
        assignee: Option<String>,
        /// Filter by issue type: task|bug|feature|epic|chore|docs|question
        #[arg(short = 't', long = "type")]
        issue_type: Option<String>,
        /// Require one of these priorities 0..=4 (repeatable: a strand
        /// matches if its priority is any of them)
        #[arg(long = "priority", value_parser = clap::value_parser!(i64).range(0..=4))]
        priority: Vec<i64>,
        #[arg(long)]
        json: bool,
    },
    /// List active strands blocked by dependencies, with their blockers
    Blocked {
        #[arg(long)]
        json: bool,
    },
    /// Search strands (case-insensitive substring over all text)
    Search {
        text: String,
        #[arg(long)]
        json: bool,
    },
    /// Print the agent-facing usage guide (markdown), or install a skill stub
    AgentsInfo {
        /// Instead of printing, write/update a braid skill file (SKILL.md)
        /// in DIR — idempotent, preserves surrounding content. Point it at
        /// e.g. .claude/skills/braid/.
        #[arg(long, value_name = "DIR")]
        install: Option<std::path::PathBuf>,
    },
    /// Print the skein secret (doc id + sync server) — grants read/write access; share deliberately
    Secret,
    /// Show the resolved configuration and where each field came from (doc id redacted) — for debugging which file/layer braid is using
    Config,
    /// Import strands from a JSONL file (beads or braid format); upserts by id
    Import { path: std::path::PathBuf },
    /// Export all strands as JSONL to stdout
    Export,
    /// Rotate the skein into a fresh document (sheds history; with
    /// --revoke, recovers from a leaked doc id)
    Rotate {
        /// The old doc id is presumed leaked: write no forwarding pointer
        /// into the old document; distribute the new secret out-of-band
        #[arg(long, conflicts_with = "adopt")]
        revoke: bool,
        /// Follow a compact rotation's forwarding pointer: switch this
        /// clone to the successor skein
        #[arg(long)]
        adopt: bool,
    },
    /// Sync with the configured server (fails if unreachable)
    Sync,
    /// Serve this skein to MCP hosts over stdio (Claude Desktop, IDEs, ...)
    Mcp {
        /// Project directory (defaults to the current directory)
        #[arg(long)]
        project: Option<std::path::PathBuf>,
        /// Serve only read-only tools
        #[arg(long)]
        read_only: bool,
        /// Expose braid_delete and braid_import (no-undo operations)
        #[arg(long, conflicts_with = "read_only")]
        enable_destructive: bool,
    },
}

#[derive(Subcommand)]
enum DepCmd {
    /// Add a dependency: ISSUE depends on TARGET
    Add {
        issue: String,
        target: String,
        /// blocks|parent-child|conditional-blocks|waits-for|related|...
        #[arg(short = 't', long = "type", default_value = "blocks")]
        dep_type: String,
    },
    /// Remove a dependency (all types unless --type narrows it)
    Remove {
        issue: String,
        target: String,
        #[arg(short = 't', long = "type")]
        dep_type: Option<String>,
    },
    /// List dependencies of an issue, both directions
    List { issue: String },
    /// Recursive parent-child descendant tree of an issue (epic → subtasks)
    Tree {
        issue: String,
        #[arg(long)]
        json: bool,
    },
    /// Report dependency cycles (blocking + parent-child edges)
    Cycles,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot determine current directory: {e}");
            std::process::exit(1);
        }
    };

    let result = match cli.cmd {
        Cmd::Init { name, prefix, join, sync_server, print_only } => {
            commands::init(&cwd, commands::InitOpts { name, prefix, join, sync_server, print_only })
                .await
        }
        Cmd::Create {
            title,
            description,
            issue_type,
            priority,
            label,
            slug,
            assignee,
            deps,
            json,
        } => {
            commands::create(
                &cwd,
                commands::CreateOpts {
                    title,
                    description,
                    issue_type,
                    priority,
                    labels: label,
                    slug,
                    assignee,
                    deps,
                    json,
                },
            )
            .await
        }
        Cmd::Show { id, json } => commands::show(&cwd, &id, json).await,
        Cmd::List { status, all, label, assignee, issue_type, priority, json } => {
            let filter =
                commands::FilterOpts { labels: label, assignee, issue_type, priorities: priority };
            commands::list(&cwd, status, all, filter, json).await
        }
        Cmd::Update {
            id,
            title,
            description,
            design,
            acceptance_criteria,
            notes,
            status,
            priority,
            issue_type,
            assignee,
            external_ref,
            add_label,
            remove_label,
            json,
        } => {
            commands::update(
                &cwd,
                &id,
                commands::UpdateOpts {
                    title,
                    description,
                    design,
                    acceptance_criteria,
                    notes,
                    status,
                    priority,
                    issue_type,
                    assignee,
                    external_ref,
                    add_labels: add_label,
                    remove_labels: remove_label,
                    json,
                },
            )
            .await
        }
        Cmd::Close { ids, reason, force } => commands::close(&cwd, &ids, reason, force).await,
        Cmd::Reopen { ids } => commands::reopen(&cwd, &ids).await,
        Cmd::Defer { ids, until } => commands::defer(&cwd, &ids, until).await,
        Cmd::Undefer { ids } => commands::undefer(&cwd, &ids).await,
        Cmd::Delete { ids, force } => commands::delete(&cwd, &ids, force).await,
        Cmd::Comment { id, text } => commands::comment(&cwd, &id, &text).await,
        Cmd::Dep { cmd } => match cmd {
            DepCmd::Add { issue, target, dep_type } => {
                commands::dep_add(&cwd, &issue, &target, &dep_type).await
            }
            DepCmd::Remove { issue, target, dep_type } => {
                commands::dep_remove(&cwd, &issue, &target, dep_type).await
            }
            DepCmd::List { issue } => commands::dep_list(&cwd, &issue).await,
            DepCmd::Tree { issue, json } => commands::dep_tree(&cwd, &issue, json).await,
            DepCmd::Cycles => commands::dep_cycles(&cwd).await,
        },
        Cmd::Ready { label, assignee, issue_type, priority, json } => {
            let filter =
                commands::FilterOpts { labels: label, assignee, issue_type, priorities: priority };
            commands::ready(&cwd, filter, json).await
        }
        Cmd::Blocked { json } => commands::blocked(&cwd, json).await,
        Cmd::Search { text, json } => commands::search(&cwd, &text, json).await,
        Cmd::AgentsInfo { install } => match install {
            Some(dir) => commands::agents_info_install(&dir),
            None => {
                commands::agents_info();
                Ok(())
            }
        },
        Cmd::Secret => commands::secret(&cwd),
        Cmd::Config => commands::config(&cwd),
        Cmd::Rotate { revoke, adopt } => {
            if adopt {
                commands::rotate_adopt(&cwd).await
            } else {
                commands::rotate(&cwd, revoke).await
            }
        }
        Cmd::Import { path } => commands::import(&cwd, &path).await,
        Cmd::Export => commands::export(&cwd).await,
        Cmd::Sync => commands::sync(&cwd).await,
        Cmd::Mcp { project, read_only, enable_destructive } => {
            braid::mcp::serve(braid::mcp::McpOpts { project, read_only, enable_destructive }).await
        }
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
