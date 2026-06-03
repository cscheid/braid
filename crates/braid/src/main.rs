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
    /// Create a new tracker (or join an existing one) and write .braid.toml
    Init {
        /// Display name for the tracker (default: current directory name)
        #[arg(long)]
        name: Option<String>,
        /// Prefix for generated issue ids
        #[arg(long, default_value = "br")]
        prefix: String,
        /// Adopt an existing tracker by document id instead of creating one
        #[arg(long, value_name = "DOC_ID")]
        join: Option<String>,
        /// Sync server URL to record in .braid.toml
        #[arg(long, value_name = "URL")]
        sync_server: Option<String>,
        /// Print the .braid.toml contents to stdout instead of writing it
        #[arg(long)]
        print_only: bool,
    },
    /// Create a new issue; prints its id
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
        /// Print the full issue as JSON instead of just the id
        #[arg(long)]
        json: bool,
    },
    /// Show one issue (by id or unique id fragment)
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// List issues
    List {
        /// Filter by status (open|in_progress|blocked|deferred|closed)
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Update fields of an issue (empty string clears optional fields)
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
    /// Close one or more issues
    Close {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        reason: Option<String>,
        /// Close even if the issue still has open children
        #[arg(long)]
        force: bool,
    },
    /// Reopen closed issues
    Reopen {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Add a comment to an issue; prints the comment id
    Comment { id: String, text: String },
    /// Manage dependencies between issues
    Dep {
        #[command(subcommand)]
        cmd: DepCmd,
    },
    /// List issues that are ready to work on (active, unblocked)
    Ready {
        #[arg(long)]
        json: bool,
    },
    /// List active issues blocked by dependencies, with their blockers
    Blocked {
        #[arg(long)]
        json: bool,
    },
    /// Search issues (case-insensitive substring over all text)
    Search {
        text: String,
        #[arg(long)]
        json: bool,
    },
    /// Print the agent-facing usage guide (markdown)
    AgentsInfo,
    /// Sync with the configured server (fails if unreachable)
    Sync,
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
        Cmd::Create { title, description, issue_type, priority, label, slug, assignee, json } => {
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
                    json,
                },
            )
            .await
        }
        Cmd::Show { id, json } => commands::show(&cwd, &id, json).await,
        Cmd::List { status, json } => commands::list(&cwd, status, json).await,
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
        Cmd::Comment { id, text } => commands::comment(&cwd, &id, &text).await,
        Cmd::Dep { cmd } => match cmd {
            DepCmd::Add { issue, target, dep_type } => {
                commands::dep_add(&cwd, &issue, &target, &dep_type).await
            }
            DepCmd::Remove { issue, target, dep_type } => {
                commands::dep_remove(&cwd, &issue, &target, dep_type).await
            }
            DepCmd::List { issue } => commands::dep_list(&cwd, &issue).await,
            DepCmd::Cycles => commands::dep_cycles(&cwd).await,
        },
        Cmd::Ready { json } => commands::ready(&cwd, json).await,
        Cmd::Blocked { json } => commands::blocked(&cwd, json).await,
        Cmd::Search { text, json } => commands::search(&cwd, &text, json).await,
        Cmd::AgentsInfo => {
            commands::agents_info();
            Ok(())
        }
        Cmd::Sync => commands::sync(&cwd).await,
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
