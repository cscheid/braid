# an automerge-centric issue tracker for LLM agents

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

## Development

Always follow TDD workflow: write/update tests BEFORE implementing features. When creating plans, include test specifications as the first phase. Never skip to implementation without a test plan.

## External sources

You may make use of the `external-sources/` directory to store local copies of source code repositories that are useful to search locally.

Currently, this includes:

- [beads_rust](external-sources/beads_rust).
