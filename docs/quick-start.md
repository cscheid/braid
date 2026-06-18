# Quick start

```sh
# in your project directory
braid init                  # creates a skein, writes .braid.toml
echo .braid.toml >> .gitignore

braid create "Fix the frobnicator" --type bug --priority 1
braid ready                 # what's workable right now
braid close br-x7k2m9q4 --reason "fixed"
```

On another machine / clone / worktree of the same project:

```sh
braid init --join <doc-id>  # paste the doc id from the first machine
braid list                  # open strands, fetched from the sync server
```

Agents: run `braid agents-info` for a complete, version-matched usage
guide. To wire braid into a project's agent tooling, run `braid agents-info
--install <dir>` (e.g. `.claude/skills/braid/`): it writes a `SKILL.md`
with YAML frontmatter (so it's a discoverable skill) over a body that
defers to `braid agents-info` for the authoritative guide. The installer is
idempotent — it refreshes the braid-managed head in place and preserves any
trailing content, so re-running on a new braid version just refreshes it.
