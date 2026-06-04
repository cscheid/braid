# an automerge-centric issue tracker for LLM agents

## Terminology

**braid** is the program; a **skein** is the collection of issues tracked
for a project (one automerge document); a **strand** is a single issue;
**loom** is reserved for a future sync-server relay. Full rationale and
usage notes: [docs/terminology.md](docs/terminology.md). Use these words in
prose and CLI output; interfaces (command names, schema fields) keep their
existing names unless a strand says otherwise.

This repo dogfoods braid: its own skein is configured via the gitignored
`.braid.toml`. Run `braid agents-info` for usage; attribute your changes
with `BRAID_AUTHOR=claude`.

When you start working on a strand, mark it `in_progress` and assign it
to yourself (`BRAID_AUTHOR=claude braid update <id> --status in_progress
--assignee claude`); when the work lands, close it with a comment
summarizing the outcome.

## Plans

In this repository, we use Markdown documents as plans. The file format for the plans is `claude-notes/plans/yyyy/mm/dd/<plan>.md`.

### File Structure

Plan files should include:

1. **Overview**: Brief description of the plan's goals and context
2. **Checklist**: A markdown checklist of all work items using `- [ ]` syntax
3. **Details**: Additional context, design decisions, or implementation notes as needed

### Maintaining Progress
As you work through a plan:

1. **Update the plan file** after completing each work item
2. **Check off items** by changing `- [ ]` to `- [x]`
3. **Keep the plan file current** - it serves as both a roadmap and progress tracker
4. **Add new items** if you discover additional work during implementation

### Excerpt from a simple Plan File

```markdown
...

## Work Items

- [x] Review current runtime service implementations
- [x] Identify common patterns
- [ ] Update StandalonePlatform to use shared base
- [ ] Update tests
- [ ] Update documentation
```

### When to Use Plan Files

Create plan files for:
- Multi-step features spanning multiple packages
- Complex refactoring that requires coordination
- Tasks where tracking progress helps ensure nothing is missed

Complex plans can have phases, and work items are then split into multiple lists, one for each phase.

For simple tasks (single file changes, bug fixes), the TodoWrite tool is sufficient.

## Documentation discipline

User-facing changes land **with their documentation in the same commit**:

- new/changed CLI commands → `crates/braid/src/agents-info.md` (the
  version-matched agent guide) and, if conceptually significant, README
- new/changed MCP tools → `docs/mcp.md` (capability-tier table)
- schema/JSONL changes → `docs/schemas/` (and its contract tests)
- vocabulary → `docs/terminology.md`

`crates/braid/tests/docs_drift.rs` enforces the first two mechanically
(every subcommand must appear in agents-info; every MCP tool in
docs/mcp.md) — if it fails, update the docs, don't loosen the test.

## Git workflow

Committing locally is always fine. **Do not run git commands that change
the remote (`git push`, `git push --force`, branch deletion on origin,
etc.) without asking Carlos first** — even when CI or other tooling would
benefit from a push. Ask, then push.

## Development

Always follow TDD workflow: write/update tests BEFORE implementing features. When creating plans, include test specifications as the first phase. Never skip to implementation without a test plan.

Run `cargo xtask ci` (fmt --check, clippy, build, full test suite — the
same pipeline as CI) before asking Carlos to push. rustfmt is enforced;
`cargo xtask fmt` applies it. Humans can opt into a pre-push hook with
`cargo xtask install-hooks`.

## External sources

You may make use of the `external-sources/` directory to store local copies of source code repositories that are useful to search locally.

Currently, this includes:

- [beads_rust](external-sources/beads_rust).
