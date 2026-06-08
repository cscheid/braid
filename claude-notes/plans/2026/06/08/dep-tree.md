# braid dep tree <id> (recursive)

Strand: **br-t7g5rv4n** — "braid dep tree <id> (recursive)" (P1 feature,
child of epic **br-f2t3crsl**, braid 0.3.0 / q2 migration).

Source: `2026-06-08-braid-0.3.0-features-for-migration.md`, Feature 1.

## Overview

braid has one-hop dependency views (`dep list <id>`, both directions) and
`dep cycles`, but no recursive tree. beads has `br dep tree <id>`, used in
q2's CLAUDE.md quick-ref and by humans orienting inside an epic. Add
`braid dep tree <id> [--json]` that walks edges recursively and renders an
indented tree with per-node status, breaking cycles gracefully.

## What the code already gives us

The dependency model (confirmed 2026-06-08):

- A child strand holds a dependency `{depends_on_id: <parent>, dep_type:
  parent-child}`. So an epic's children are the strands whose deps point
  *at* the epic with a hierarchical edge.
- `braid_core::domain::dependents_of(skein, id)` returns every strand that
  depends on `id` (any type, any status), sorted by id
  (`crates/braid-core/src/domain.rs:202`).
- `open_children(skein, id)` is the non-terminal, hierarchical-only subset
  (`domain.rs:117`) — too narrow for a tree (we want closed children too),
  but its filter (`d.dep_type.is_hierarchical() && d.depends_on_id == id`)
  is the predicate to reuse.
- `dependency_cycles(skein)` (`domain.rs:138`) gives the structural cycle
  set; `is_blocking()` / `is_hierarchical()` on `DependencyType`
  (`schema.rs:252-268`) classify edges.
- `Session::dep_list` (`ops.rs:377`) already formats neighbor `{id,
  dep_type, status}` and resolves a query fragment via `resolve_issue` —
  the model to follow for `dep_tree`.

## Design decisions (to iterate on)

### Direction and edge set — DECIDED: parent-child only

View = **descendants of an epic**: from `<id>`, follow *incoming* edges
(strands that depend on `<id>`) where the edge is **parent-child**. That
renders epic → subtask trees, the primary use.

**Decided (Carlos, 2026-06-08): parent-child only, no blocking edges, no
`--edges` flag** — don't complicate the domain for this port unless
unavoidable. The doc floated "sensibly, the blocking edges"; we explicitly
decline. Rationale: mixing parent-child and blocking edges in one indented
tree is ambiguous (a node can appear under two unrelated parents), and the
clean epic→subtask tree is what the quick-ref needs. The `TreeNode` carries
the edge type per node, so a future `--edges all` is purely additive if
ever wanted.

### Rendering

- Indented tree, one node per line, status marker like `dep list`:
  `br-abcd1234  Title here            [open]`. Indent by depth (2 spaces or
  `│  `/`├─ ` box-drawing — match whatever `dep list` uses; plain 2-space
  indent if it uses none, to stay grep-friendly).
- Root line is `<id>` itself.
- Resolve `<id>` via the same `resolve_issue` fragment matching as
  `show`/`dep list` so `braid dep tree 6j42` works.

### Cycle / repeat handling

Walk with a `visited: HashSet<String>` along the current path **and**
globally. When an edge points to an id already on the current path, print
the node followed by a `(cycle)` marker and do **not** recurse. For a DAG
node reachable by two paths (diamond), print it each place it appears but
do not re-expand after the first full expansion — mark the repeat with
`(see above)` (or expand once and `(…)` elsewhere). Decide one and document.
Reuse the structural-edge notion from `dependency_cycles` rather than
reimplementing detection; the per-walk visited set is what actually
prevents infinite loops.

### Where the logic lives

- **braid-core**: a pure `dep_tree(skein, root_id, opts) -> TreeNode`
  builder in `domain.rs` (no I/O), returning a nested struct
  `{ id, title, status, dep_type: Option<..>, cycle: bool, children:
  Vec<TreeNode> }`. Unit-testable without a session.
- **ops.rs**: `Session::dep_tree(query) -> Result<TreeNode>` (hydrate +
  resolve + call core), mirroring `dep_list`.
- **commands.rs**: `dep_tree(cwd, query, json)` printer — indented text or
  `serde_json` of the nested struct.
- **main.rs**: new `DepCmd::Tree { id, json }` arm (alongside
  `List`/`Cycles` at `main.rs:325-332`).

### MCP parity — DECIDED: ship `braid_dep_tree`

`dep_list`/`dep_cycles` are MCP tools (`mcp.rs:177,192`); dispatch is a
single `match` arm and the schema is a small JSON object. Once the core
builder exists, adding `braid_dep_tree` is cheap and keeps the tiers
aligned. **Decided (Carlos, 2026-06-08): include it in 0.3.0**, document in
`docs/mcp.md` (docs-discipline requires a row for any new MCP tool).

## Test plan (write first — TDD)

**Phase 1 — core builder unit tests (`domain.rs`):**

- [ ] Skein: epic A with children B, C (each `parent-child` → A); C has
      child D (`parent-child` → C). `dep_tree(A)` yields A→[B, C→[D]] with
      correct nesting and statuses.
- [ ] Mixed statuses: a closed child still appears, marked `[closed]`.
- [ ] Cycle: A blocks B, B blocks A (or a parent-child cycle) terminates
      with a `cycle` flag set and no infinite recursion.
- [ ] Diamond: B and C both parent D — D rendered per the chosen
      repeat-handling rule (documented), no double full-expansion.
- [ ] Leaf with no children → single node, empty `children`.

**Phase 2 — CLI / JSON shape:**

- [ ] `dep tree A` text output: indentation + status markers as specified.
- [ ] `dep tree A --json` matches the documented nested schema (stable
      field names); snapshot or structural assertion.
- [ ] Unknown / ambiguous id fragment errors like `show`/`dep list`.

## Work items

- [ ] Phase 1 + 2 tests written and red.
- [ ] `TreeNode` + `dep_tree` builder in `braid-core::domain` (cycle-safe).
- [ ] `Session::dep_tree` in `ops.rs`.
- [ ] `dep_tree` printer (text + `--json`) in `commands.rs`.
- [ ] `DepCmd::Tree` wired in `main.rs`.
- [ ] `braid_dep_tree` MCP tool + `docs/mcp.md` row.
- [ ] `agents-info.md`: add `dep tree` to the command reference table.
- [ ] `dep --help` / subcommand doc string.
- [ ] `cargo xtask ci` green (incl. `docs_drift` for the new subcommand).

## Docs to touch (same commit)

- `crates/braid/src/agents-info.md` — new subcommand row (docs_drift test
  requires every subcommand to appear here).
- `docs/mcp.md` — row for the new `braid_dep_tree` tool.
- README — mention only if conceptually significant (probably one line in
  the dependency section).
