# ⚠️ The document id is a secret

The automerge document id is a **bearer token**: anyone who has it can
read *and write* your skein, forever. Treat it like a credential:

- never commit `.braid.toml` (gitignore it; `braid init` reminds you)
- never paste the doc id into issue text, logs, commits, or PRs
- the default sync server, `wss://sync.automerge.org`, is a **public
  community relay**: it stores your document unencrypted, and possession
  of the id is the only access control. Fine for experiments; for real
  work, run your own sync server and set `sync_server` accordingly.

(The automerge ecosystem's capability-based access control + E2EE work —
Keyhive/Beelay — will eventually improve this story upstream.)

If a doc id leaks, [`braid rotate --revoke`](rotation.md) moves the skein
to a fresh document. It protects *future* reads and writes only — the old
document's history stays readable to anyone who held the old id.
