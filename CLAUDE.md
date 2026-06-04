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

## Git workflow

Committing locally is always fine. **Do not run git commands that change
the remote (`git push`, `git push --force`, branch deletion on origin,
etc.) without asking Carlos first** — even when CI or other tooling would
benefit from a push. Ask, then push.

## Development

Always follow TDD workflow: write/update tests BEFORE implementing features. When creating plans, include test specifications as the first phase. Never skip to implementation without a test plan.

## External sources

You may make use of the `external-sources/` directory to store local copies of source code repositories that are useful to search locally.

Currently, this includes:

- [beads_rust](external-sources/beads_rust).
