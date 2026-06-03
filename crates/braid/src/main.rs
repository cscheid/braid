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
    /// Sync with the configured server (fails if unreachable)
    Sync,
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
        Cmd::Sync => commands::sync(&cwd).await,
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
