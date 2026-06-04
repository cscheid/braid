//! e2e tests for `braid mcp` (plan: claude-notes/plans/2026/06/04/braid-mcp.md).
//!
//! A minimal JSON-RPC client speaks newline-delimited MCP over the spawned
//! server's stdio: initialize → tools/list → tools/call. Tests cover the
//! three gating layers (annotations, --read-only, --enable-destructive),
//! the schema conformance of tool outputs, doc-id hygiene, and the
//! long-lived-session rotation guard.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

const DEAD_SERVER: &str = "tcp://127.0.0.1:1";

struct Skein {
    home: PathBuf,
    work: PathBuf,
}

impl Skein {
    fn new(tmp: &Path, name: &str) -> Skein {
        let home = tmp.join(format!("{name}-home"));
        let work = tmp.join(format!("{name}-work"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        Skein { home, work }
    }

    fn init(&self, server_url: &str) -> String {
        let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
        c.current_dir(&self.work)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", &self.home)
            .env("BRAID_SYNC_TIMEOUT", "0.3")
            .args(["init", "--name", "mcp-test", "--sync-server", server_url])
            .assert()
            .success();
        let secret = std::fs::read_to_string(self.work.join(".braid.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&secret).unwrap();
        parsed["doc_id"].as_str().unwrap().to_string()
    }
}

struct McpClient {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    /// every protocol byte the server sent us (for hygiene assertions)
    transcript: String,
}

impl McpClient {
    async fn spawn(skein: &Skein, extra_args: &[&str]) -> McpClient {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_braid"))
            .arg("mcp")
            .args(extra_args)
            .current_dir(&skein.work)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", &skein.home)
            .env("BRAID_SYNC_TIMEOUT", "0.5")
            .env("BRAID_AUTHOR", "mcp-agent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn braid mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client =
            McpClient { _child: child, stdin, stdout, next_id: 0, transcript: String::new() };
        client.initialize().await;
        client
    }

    async fn send(&mut self, msg: Value) {
        let line = format!("{msg}\n");
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                self.stdout.read_line(&mut line),
            )
            .await
            .expect("server response timed out")
            .expect("read from server");
            assert!(n > 0, "server closed stdout");
            self.transcript.push_str(&line);
            let v: Value = serde_json::from_str(line.trim()).expect("server sent valid JSON");
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    async fn initialize(&mut self) {
        let resp = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "braid-e2e", "version": "0"}
                }),
            )
            .await;
        assert!(resp.get("result").is_some(), "initialize failed: {resp}");
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    }

    async fn tools(&mut self) -> Vec<Value> {
        let resp = self.request("tools/list", json!({})).await;
        resp["result"]["tools"].as_array().expect("tools array").clone()
    }

    async fn tool_names(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tools()
            .await
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        names
    }

    /// Call a tool, expecting success; returns structuredContent.
    async fn call(&mut self, name: &str, args: Value) -> Value {
        let resp = self
            .request("tools/call", json!({"name": name, "arguments": args}))
            .await;
        let result = &resp["result"];
        assert_ne!(
            result.get("isError"),
            Some(&json!(true)),
            "tool {name} failed: {resp}"
        );
        result["structuredContent"].clone()
    }

    /// Call a tool without asserting the outcome: (is_error, error_text).
    async fn try_call(&mut self, name: &str, args: Value) -> (bool, String) {
        let resp = self
            .request("tools/call", json!({"name": name, "arguments": args}))
            .await;
        if let Some(err) = resp.get("error") {
            return (true, err["message"].as_str().unwrap_or_default().to_string());
        }
        let result = &resp["result"];
        let is_err = result.get("isError") == Some(&json!(true));
        let text = result["content"][0]["text"].as_str().unwrap_or_default().to_string();
        (is_err, text)
    }

    /// Call a tool, expecting a tool-level error; returns the error text.
    async fn call_expect_error(&mut self, name: &str, args: Value) -> String {
        let resp = self
            .request("tools/call", json!({"name": name, "arguments": args}))
            .await;
        if let Some(err) = resp.get("error") {
            return err["message"].as_str().unwrap_or_default().to_string();
        }
        let result = &resp["result"];
        assert_eq!(
            result.get("isError"),
            Some(&json!(true)),
            "expected {name} to fail: {resp}"
        );
        result["content"][0]["text"].as_str().unwrap_or_default().to_string()
    }
}

const DEFAULT_TOOLS: &[&str] = &[
    "braid_blocked",
    "braid_close",
    "braid_comment",
    "braid_create",
    "braid_defer",
    "braid_dep_add",
    "braid_dep_cycles",
    "braid_dep_list",
    "braid_dep_remove",
    "braid_export",
    "braid_list",
    "braid_ready",
    "braid_reopen",
    "braid_search",
    "braid_show",
    "braid_undefer",
    "braid_update",
];

#[tokio::test(flavor = "multi_thread")]
async fn default_toolset_and_annotations() {
    let tmp = tempfile::tempdir().unwrap();
    let skein = Skein::new(tmp.path(), "a");
    skein.init(DEAD_SERVER);

    let mut client = McpClient::spawn(&skein, &[]).await;
    let names = client.tool_names().await;
    assert_eq!(names, DEFAULT_TOOLS, "default toolset");

    let tools = client.tools().await;
    let get = |name: &str| -> Value {
        tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{name}")).clone()
    };

    // queries are read-only; mutations are not; nothing default-visible is
    // destructive; everything is closed-world (skein-confined)
    assert_eq!(get("braid_ready")["annotations"]["readOnlyHint"], json!(true));
    assert_eq!(get("braid_show")["annotations"]["readOnlyHint"], json!(true));
    assert_eq!(get("braid_create")["annotations"]["readOnlyHint"], json!(false));
    assert_eq!(get("braid_create")["annotations"]["destructiveHint"], json!(false));
    assert_eq!(get("braid_close")["annotations"]["destructiveHint"], json!(false));
    assert_eq!(get("braid_close")["annotations"]["idempotentHint"], json!(true));
    assert_eq!(get("braid_ready")["annotations"]["openWorldHint"], json!(false));

    // every tool has a description and an input schema
    for t in &tools {
        assert!(t["description"].as_str().map(|s| !s.is_empty()).unwrap_or(false));
        assert_eq!(t["inputSchema"]["type"], json!("object"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_serves_only_queries_and_refuses_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let skein = Skein::new(tmp.path(), "a");
    skein.init(DEAD_SERVER);

    let mut client = McpClient::spawn(&skein, &["--read-only"]).await;
    let names = client.tool_names().await;
    assert_eq!(
        names,
        vec![
            "braid_blocked",
            "braid_dep_cycles",
            "braid_dep_list",
            "braid_export",
            "braid_list",
            "braid_ready",
            "braid_search",
            "braid_show",
        ]
    );

    // defense in depth: a hidden tool must also refuse at call time
    let err = client
        .call_expect_error("braid_create", json!({"title": "sneaky"}))
        .await;
    assert!(err.contains("read-only") || err.contains("not available"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn destructive_tools_require_the_launch_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let skein = Skein::new(tmp.path(), "a");
    skein.init(DEAD_SERVER);

    // absent by default, and refused at call time
    let mut client = McpClient::spawn(&skein, &[]).await;
    let names = client.tool_names().await;
    assert!(!names.contains(&"braid_delete".to_string()));
    assert!(!names.contains(&"braid_import".to_string()));
    let err = client
        .call_expect_error("braid_delete", json!({"ids": ["br-x"]}))
        .await;
    assert!(err.contains("--enable-destructive") || err.contains("not available"), "{err}");
    drop(client);

    // present with the flag, annotated destructive
    let mut client = McpClient::spawn(&skein, &["--enable-destructive"]).await;
    let names = client.tool_names().await;
    assert!(names.contains(&"braid_delete".to_string()));
    assert!(names.contains(&"braid_import".to_string()));
    let tools = client.tools().await;
    let del = tools.iter().find(|t| t["name"] == "braid_delete").unwrap();
    assert_eq!(del["annotations"]["destructiveHint"], json!(true));

    // and it actually works
    let created = client.call("braid_create", json!({"title": "doomed"})).await;
    let id = created["id"].as_str().unwrap().to_string();
    let deleted = client.call("braid_delete", json!({"ids": [id]})).await;
    assert_eq!(deleted["deleted"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_workflow_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let skein = Skein::new(tmp.path(), "a");
    skein.init(DEAD_SERVER);

    let mut client = McpClient::spawn(&skein, &[]).await;

    // create with full params
    let created = client
        .call(
            "braid_create",
            json!({
                "title": "Fix the frobnicator",
                "description": "It frobs when it should nicate.",
                "type": "bug",
                "priority": 1,
                "labels": ["frob"],
            }),
        )
        .await;
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["status"], "open");
    assert_eq!(created["created_by"], "mcp-agent", "server-level identity");
    assert_eq!(created["sync"], "offline", "dead server -> offline outcome");

    // the structured record conforms to the published JSON Schema
    let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas/strand.schema.json");
    let schema: Value =
        serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let shown = client.call("braid_show", json!({"id": id})).await;
    assert!(
        validator.is_valid(&shown),
        "braid_show output must conform to the strand schema: {shown}"
    );

    // update, comment, dep, ready, close
    let blocker = client.call("braid_create", json!({"title": "Blocker"})).await;
    let blocker_id = blocker["id"].as_str().unwrap().to_string();
    client
        .call("braid_dep_add", json!({"id": id, "target": blocker_id, "type": "blocks"}))
        .await;
    let ready = client.call("braid_ready", json!({})).await;
    let ready_ids: Vec<&str> = ready["strands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert!(ready_ids.contains(&blocker_id.as_str()));
    assert!(!ready_ids.contains(&id.as_str()), "blocked strand not ready");

    client.call("braid_comment", json!({"id": blocker_id, "text": "on it"})).await;
    client.call("braid_update", json!({"id": blocker_id, "status": "in_progress"})).await;
    let closed = client
        .call("braid_close", json!({"ids": [blocker_id], "reason": "done"}))
        .await;
    assert_eq!(closed["closed"][0]["status"], "closed");

    // blocker closed -> original strand becomes ready
    let ready = client.call("braid_ready", json!({})).await;
    let ready_ids: Vec<&str> = ready["strands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert!(ready_ids.contains(&id.as_str()));

    // unknown tool is a clean error
    client.call_expect_error("braid_explode", json!({})).await;

    // errors are tool-level, not crashes: unknown id
    let err = client.call_expect_error("braid_show", json!({"id": "zzz-nope"})).await;
    assert!(err.contains("no issue"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn doc_id_never_crosses_the_wire() {
    let tmp = tempfile::tempdir().unwrap();
    let skein = Skein::new(tmp.path(), "a");
    let doc_id = skein.init(DEAD_SERVER);

    let mut client = McpClient::spawn(&skein, &["--enable-destructive"]).await;
    client.call("braid_create", json!({"title": "hygiene"})).await;
    client.call("braid_list", json!({})).await;
    client.call("braid_export", json!({})).await;
    client.call_expect_error("braid_show", json!({"id": "zzz-nope"})).await;
    client.tools().await;

    assert!(
        !client.transcript.contains(&doc_id),
        "the doc id is a bearer capability and must never appear in any \
         protocol message"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn long_lived_session_notices_rotation() {
    // The rotation guard is re-checked per operation: a rotation arriving
    // over sync must make a *running* server start refusing.
    let tmp = tempfile::tempdir().unwrap();

    // live in-process sync server
    let relay = samod::Repo::build_tokio()
        .with_storage(samod::storage::InMemoryStorage::new())
        .load()
        .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("tcp://{}", listener.local_addr().unwrap());
    let acceptor = relay.make_acceptor(samod::Url::parse(&url).unwrap()).unwrap();
    let accept_task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let _ = acceptor.accept_tokio_io(stream);
        }
    });

    let a = Skein::new(tmp.path(), "a");
    let doc_id = a.init(&url);

    // MCP server running against the live relay
    let mut client = McpClient::spawn(&a, &[]).await;
    client.call("braid_create", json!({"title": "before rotation"})).await;

    // another clone rotates the skein out from under the running server
    let b = Skein::new(tmp.path(), "b");
    std::fs::write(
        b.work.join(".braid.toml"),
        format!("doc_id = \"{doc_id}\"\nsync_server = \"{url}\"\n"),
    )
    .unwrap();
    let mut c = assert_cmd::Command::cargo_bin("braid").unwrap();
    c.current_dir(&b.work)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", &b.home)
        .env("BRAID_SYNC_TIMEOUT", "10")
        .arg("rotate")
        .assert()
        .success();

    // the running server picks the rotation up over sync (bounded retries
    // while the change propagates)
    let mut saw_rotation = false;
    for _ in 0..40 {
        let (is_err, text) = client.try_call("braid_ready", json!({})).await;
        if is_err && text.contains("rotated") {
            saw_rotation = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert!(saw_rotation, "running MCP server must notice the rotation");

    accept_task.abort();
    relay.stop().await;
}
