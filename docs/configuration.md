# Configuration

braid resolves its skein per-field, first hit wins:

1. **Environment**: `BRAID_DOC_ID`, `BRAID_SYNC_URL`, `BRAID_AUTHOR`
2. **Repo file**: a gitignored `.braid.toml` in the current directory or
   any parent:

   ```toml
   doc_id = "4UfaPGzzySmw7Y1MR1VVXbfp4fgx"
   sync_server = "wss://sync.automerge.org"   # optional
   author = "alice"                            # optional
   ```

3. **User config**: `~/.config/braid/projects.toml`, selected by a
   *committed*, non-secret `.braid-project` marker file containing a
   project name — useful so fresh worktrees need zero per-worktree setup:

   ```toml
   # ~/.config/braid/projects.toml
   [projects.myproject]
   doc_id = "..."
   sync_server = "wss://sync.example.com"
   ```

Authorship (`created_by`, comment authors) resolves as `BRAID_AUTHOR` →
config `author` → `git config user.name` → OS username.
