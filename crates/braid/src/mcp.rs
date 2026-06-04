//! `braid mcp`: an MCP server over stdio exposing this skein's operations
//! to agent harnesses (plan: claude-notes/plans/2026/06/04/braid-mcp.md).
//!
//! Capability posture:
//!
//! - the server process holds the skein secret; agents call tools and
//!   **never possess the doc id** (and the e2e suite asserts it crosses
//!   the wire in no message);
//! - `secret`, `init`, and `rotate` are *never* tools, under any flag —
//!   they are operator decisions;
//! - `delete`/`import` exist only behind `--enable-destructive`
//!   (launch-time operator opt-in: braid's delete has no undo);
//! - `--read-only` force-disables every non-read-only tool, enforced at
//!   call time as well as in `tools/list` (hidden tools must also refuse);
//! - every tool carries honest MCP annotations (readOnly/destructive/
//!   idempotent hints) for host confirmation UX.
//!
//! The session is long-lived: samod syncs continuously, mutations push
//! with a bounded barrier and report `sync: confirmed|unconfirmed|offline`,
//! and every operation re-checks for a rotation arriving over sync.
//!
//! stdout is the protocol channel; all diagnostics go to stderr.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rmcp::ServiceExt;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ErrorData, Implementation, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ops::{self, Session};

pub struct McpOpts {
    pub project: Option<PathBuf>,
    pub read_only: bool,
    pub enable_destructive: bool,
}

// ---------------------------------------------------------------------------
// tool registry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Tier {
    /// Read-only: available always, even under `--read-only`.
    Query,
    /// Writes, but reversible (close has reopen; update edits in place).
    Mutate,
    /// No undo. Requires `--enable-destructive`.
    Destructive,
}

struct ToolSpec {
    name: &'static str,
    description: &'static str,
    tier: Tier,
    idempotent: bool,
    schema: fn() -> Value,
}

fn specs() -> Vec<ToolSpec> {
    fn no_params() -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }
    vec![
        ToolSpec {
            name: "braid_ready",
            description: "List strands that are ready to work on (active status, no \
                          unresolved blocking dependencies). The best starting point \
                          for picking work.",
            tier: Tier::Query,
            idempotent: true,
            schema: no_params,
        },
        ToolSpec {
            name: "braid_blocked",
            description: "List active strands blocked by dependencies, with the ids \
                          blocking each.",
            tier: Tier::Query,
            idempotent: true,
            schema: no_params,
        },
        ToolSpec {
            name: "braid_list",
            description: "List strands, optionally filtered by status.",
            tier: Tier::Query,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "description": "Filter: open|in_progress|blocked|deferred|closed"
                        }
                    },
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_show",
            description: "Show one strand by id (unique id fragments work).",
            tier: Tier::Query,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {"id": {"type": "string"}},
                    "required": ["id"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_search",
            description: "Case-insensitive substring search over strand ids, titles, \
                          prose fields, labels, and comments.",
            tier: Tier::Query,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_dep_list",
            description: "List a strand's dependencies in both directions (outgoing \
                          targets and incoming dependents).",
            tier: Tier::Query,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {"id": {"type": "string"}},
                    "required": ["id"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_dep_cycles",
            description: "Report dependency cycles among blocking and parent-child edges.",
            tier: Tier::Query,
            idempotent: true,
            schema: no_params,
        },
        ToolSpec {
            name: "braid_export",
            description: "All strands as JSONL (one strand per line, id-sorted), \
                          conforming to braid's published strand JSON Schema.",
            tier: Tier::Query,
            idempotent: true,
            schema: no_params,
        },
        ToolSpec {
            name: "braid_create",
            description: "Create a new strand; returns the full strand record plus a \
                          `sync` field (confirmed|unconfirmed|offline) reporting \
                          whether the sync server acknowledged it.",
            tier: Tier::Mutate,
            idempotent: false,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "title": {"type": "string"},
                        "description": {"type": "string", "description": "Markdown body"},
                        "type": {
                            "type": "string",
                            "description": "task|bug|feature|epic|chore|docs|question",
                            "default": "task"
                        },
                        "priority": {
                            "type": "integer",
                            "description": "0 (critical) .. 4 (backlog)",
                            "default": 2
                        },
                        "labels": {"type": "array", "items": {"type": "string"}},
                        "slug": {
                            "type": "string",
                            "description": "Human-readable id segment: br-<slug>-<suffix>"
                        },
                        "assignee": {"type": "string"}
                    },
                    "required": ["title"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_update",
            description: "Update fields of a strand. Empty strings clear optional \
                          fields; omitted fields are untouched.",
            tier: Tier::Mutate,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "title": {"type": "string"},
                        "description": {"type": "string"},
                        "design": {"type": "string"},
                        "acceptance_criteria": {"type": "string"},
                        "notes": {"type": "string"},
                        "status": {
                            "type": "string",
                            "description": "open|in_progress|blocked|deferred|closed"
                        },
                        "priority": {"type": "integer"},
                        "type": {"type": "string"},
                        "assignee": {"type": "string"},
                        "external_ref": {"type": "string"},
                        "add_labels": {"type": "array", "items": {"type": "string"}},
                        "remove_labels": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["id"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_close",
            description: "Close strands (reversible via braid_reopen). Refuses if a \
                          strand has open children unless force is set.",
            tier: Tier::Mutate,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "ids": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "reason": {"type": "string"},
                        "force": {"type": "boolean", "default": false}
                    },
                    "required": ["ids"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_reopen",
            description: "Reopen closed strands.",
            tier: Tier::Mutate,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "ids": {"type": "array", "items": {"type": "string"}, "minItems": 1}
                    },
                    "required": ["ids"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_comment",
            description: "Append a comment to a strand.",
            tier: Tier::Mutate,
            idempotent: false,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "text": {"type": "string"}
                    },
                    "required": ["id", "text"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_dep_add",
            description: "Add a dependency: strand `id` depends on strand `target`. \
                          Types blocks|conditional-blocks|waits-for block ready-work; \
                          parent-child expresses hierarchy; others are informational. \
                          Cycles are allowed but reported in the result.",
            tier: Tier::Mutate,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "target": {"type": "string"},
                        "type": {"type": "string", "default": "blocks"}
                    },
                    "required": ["id", "target"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_dep_remove",
            description: "Remove a dependency from `id` on `target` (all types unless \
                          `type` narrows it).",
            tier: Tier::Mutate,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "target": {"type": "string"},
                        "type": {"type": "string"}
                    },
                    "required": ["id", "target"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_delete",
            description: "Remove strands entirely. NO UNDO: a delete wins over \
                          concurrent edits. Prefer braid_close. Refuses if other \
                          strands reference the target unless force is set.",
            tier: Tier::Destructive,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {
                        "ids": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                        "force": {"type": "boolean", "default": false}
                    },
                    "required": ["ids"],
                    "additionalProperties": false
                })
            },
        },
        ToolSpec {
            name: "braid_import",
            description: "Bulk-import strands from JSONL text (beads or braid format). \
                          Upserts by id: OVERWRITES existing strands with the same id.",
            tier: Tier::Destructive,
            idempotent: true,
            schema: || {
                json!({
                    "type": "object",
                    "properties": {"jsonl": {"type": "string"}},
                    "required": ["jsonl"],
                    "additionalProperties": false
                })
            },
        },
    ]
}

fn to_tool(spec: &ToolSpec) -> Tool {
    let annotations = ToolAnnotations::new()
        .read_only(spec.tier == Tier::Query)
        .destructive(spec.tier == Tier::Destructive)
        .idempotent(spec.idempotent)
        // skein-confined: effects stay within this issue tracker
        .open_world(false);
    let schema = match (spec.schema)() {
        Value::Object(o) => o,
        _ => unreachable!("tool schemas are objects"),
    };
    Tool::new(spec.name, spec.description, Arc::new(schema)).annotate(annotations)
}

// ---------------------------------------------------------------------------
// server
// ---------------------------------------------------------------------------

pub struct BraidServer {
    session: Session,
    read_only: bool,
    enable_destructive: bool,
}

impl BraidServer {
    fn visible(&self, tier: Tier) -> bool {
        match tier {
            Tier::Query => true,
            Tier::Mutate => !self.read_only,
            Tier::Destructive => !self.read_only && self.enable_destructive,
        }
    }

    /// Tool-level error (the agent sees the message and can react);
    /// protocol errors are reserved for malformed requests.
    fn tool_error(msg: impl Into<String>) -> CallToolResult {
        CallToolResult::error(vec![Content::text(msg.into())])
    }

    async fn dispatch(&self, name: &str, args: Value) -> Result<Value> {
        match name {
            "braid_ready" => {
                let strands = self.session.ready()?;
                Ok(json!({"strands": strands, "count": strands_len(&strands)}))
            }
            "braid_blocked" => {
                let blocked = self.session.blocked()?;
                Ok(json!({"blocked": blocked}))
            }
            "braid_list" => {
                #[derive(Deserialize)]
                struct P {
                    status: Option<String>,
                }
                let p: P = serde_json::from_value(args)?;
                let strands = self.session.list(p.status.as_deref())?;
                Ok(json!({"strands": strands, "count": strands_len(&strands)}))
            }
            "braid_show" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                }
                let p: P = serde_json::from_value(args)?;
                Ok(serde_json::to_value(self.session.show(&p.id)?)?)
            }
            "braid_search" => {
                #[derive(Deserialize)]
                struct P {
                    text: String,
                }
                let p: P = serde_json::from_value(args)?;
                let strands = self.session.search(&p.text)?;
                Ok(json!({"strands": strands, "count": strands_len(&strands)}))
            }
            "braid_dep_list" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                }
                let p: P = serde_json::from_value(args)?;
                Ok(serde_json::to_value(self.session.dep_list(&p.id)?)?)
            }
            "braid_dep_cycles" => {
                Ok(json!({"cycles": self.session.dep_cycles()?}))
            }
            "braid_export" => {
                Ok(json!({"jsonl": self.session.export_jsonl()?}))
            }
            "braid_create" => {
                #[derive(Deserialize)]
                struct P {
                    title: String,
                    description: Option<String>,
                    #[serde(rename = "type", default = "default_type")]
                    issue_type: String,
                    #[serde(default = "default_priority")]
                    priority: i64,
                    #[serde(default)]
                    labels: Vec<String>,
                    slug: Option<String>,
                    assignee: Option<String>,
                }
                let p: P = serde_json::from_value(args)?;
                let result = self
                    .session
                    .create(ops::CreateOpts {
                        title: p.title,
                        description: p.description,
                        issue_type: p.issue_type,
                        priority: p.priority,
                        labels: p.labels,
                        slug: p.slug,
                        assignee: p.assignee,
                    })
                    .await?;
                Ok(serde_json::to_value(result)?)
            }
            "braid_update" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    title: Option<String>,
                    description: Option<String>,
                    design: Option<String>,
                    acceptance_criteria: Option<String>,
                    notes: Option<String>,
                    status: Option<String>,
                    priority: Option<i64>,
                    #[serde(rename = "type")]
                    issue_type: Option<String>,
                    assignee: Option<String>,
                    external_ref: Option<String>,
                    #[serde(default)]
                    add_labels: Vec<String>,
                    #[serde(default)]
                    remove_labels: Vec<String>,
                }
                let p: P = serde_json::from_value(args)?;
                let result = self
                    .session
                    .update(
                        &p.id,
                        ops::UpdateOpts {
                            title: p.title,
                            description: p.description,
                            design: p.design,
                            acceptance_criteria: p.acceptance_criteria,
                            notes: p.notes,
                            status: p.status,
                            priority: p.priority,
                            issue_type: p.issue_type,
                            assignee: p.assignee,
                            external_ref: p.external_ref,
                            add_labels: p.add_labels,
                            remove_labels: p.remove_labels,
                        },
                    )
                    .await?;
                Ok(serde_json::to_value(result)?)
            }
            "braid_close" => {
                #[derive(Deserialize)]
                struct P {
                    ids: Vec<String>,
                    reason: Option<String>,
                    #[serde(default)]
                    force: bool,
                }
                let p: P = serde_json::from_value(args)?;
                let result = self.session.close_strands(&p.ids, p.reason, p.force).await?;
                Ok(serde_json::to_value(result)?)
            }
            "braid_reopen" => {
                #[derive(Deserialize)]
                struct P {
                    ids: Vec<String>,
                }
                let p: P = serde_json::from_value(args)?;
                Ok(serde_json::to_value(self.session.reopen(&p.ids).await?)?)
            }
            "braid_comment" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    text: String,
                }
                let p: P = serde_json::from_value(args)?;
                Ok(serde_json::to_value(self.session.comment(&p.id, &p.text).await?)?)
            }
            "braid_dep_add" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    target: String,
                    #[serde(rename = "type", default = "default_dep_type")]
                    dep_type: String,
                }
                let p: P = serde_json::from_value(args)?;
                Ok(serde_json::to_value(
                    self.session.dep_add(&p.id, &p.target, &p.dep_type).await?,
                )?)
            }
            "braid_dep_remove" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    target: String,
                    #[serde(rename = "type")]
                    dep_type: Option<String>,
                }
                let p: P = serde_json::from_value(args)?;
                Ok(serde_json::to_value(
                    self.session.dep_remove(&p.id, &p.target, p.dep_type.as_deref()).await?,
                )?)
            }
            "braid_delete" => {
                #[derive(Deserialize)]
                struct P {
                    ids: Vec<String>,
                    #[serde(default)]
                    force: bool,
                }
                let p: P = serde_json::from_value(args)?;
                Ok(serde_json::to_value(self.session.delete(&p.ids, p.force).await?)?)
            }
            "braid_import" => {
                #[derive(Deserialize)]
                struct P {
                    jsonl: String,
                }
                let p: P = serde_json::from_value(args)?;
                let issues = crate::import::parse_jsonl(&p.jsonl)?;
                Ok(serde_json::to_value(self.session.import(&issues).await?)?)
            }
            other => anyhow::bail!("unknown tool: {other}"),
        }
    }
}

fn strands_len(v: &[braid_core::schema::Issue]) -> usize {
    v.len()
}

fn default_type() -> String {
    "task".to_string()
}
fn default_priority() -> i64 {
    2
}
fn default_dep_type() -> String {
    "blocks".to_string()
}

impl ServerHandler for BraidServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("braid", env!("CARGO_PKG_VERSION"))
                    .with_title("braid issue tracking"),
            )
            .with_instructions(
                "braid tracks work in a skein (a CRDT-synced collection of issue \
                 strands). Start with braid_ready to find workable strands; \
                 braid_create files new work; braid_close completes it. Mutation \
                 results carry a `sync` field reporting server acknowledgement.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = specs()
            .iter()
            .filter(|s| self.visible(s.tier))
            .map(to_tool)
            .collect();
        Ok(ListToolsResult { tools, next_cursor: None, meta: None })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let name = request.name.as_ref();
        // Gating is enforced at call time, not just in tools/list: a host
        // (or a confused agent) may call tools it was never shown.
        match specs().iter().find(|s| s.name == name) {
            None => return Ok(Self::tool_error(format!("unknown tool: {name}"))),
            Some(spec) if !self.visible(spec.tier) => {
                let why = match spec.tier {
                    Tier::Destructive if !self.enable_destructive => {
                        "not available: this server was started without \
                         --enable-destructive"
                    }
                    _ => "not available: this server is read-only",
                };
                return Ok(Self::tool_error(format!("{name} is {why}")));
            }
            Some(_) => {}
        }

        let args = Value::Object(request.arguments.unwrap_or_default());
        match self.dispatch(name, args).await {
            Ok(value) => Ok(CallToolResult::structured(value)),
            // Domain failures (unknown id, open children, rotation, parse
            // errors) are tool-level errors the agent can read and react to.
            Err(e) => Ok(Self::tool_error(format!("{e:#}"))),
        }
    }
}

/// Run the server over stdio until the host closes the transport.
pub async fn serve(opts: McpOpts) -> Result<()> {
    let cwd = match opts.project {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    let session = Session::open(&cwd).await?;
    eprintln!(
        "braid mcp: serving skein ({} strands) as {}{}{}",
        session.strand_count().map(|n| n.to_string()).unwrap_or_else(|_| "?".into()),
        session.author(),
        if opts.read_only { ", read-only" } else { "" },
        if opts.enable_destructive { ", destructive tools enabled" } else { "" },
    );

    let server = BraidServer {
        session,
        read_only: opts.read_only,
        enable_destructive: opts.enable_destructive,
    };
    let service = server
        .serve(rmcp::transport::io::stdio())
        .await
        .map_err(|e| anyhow::anyhow!("mcp transport failed to start: {e}"))?;
    service.waiting().await?;
    Ok(())
}
