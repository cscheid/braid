# Migrating from beads

```sh
braid init --prefix bd      # keep your bd- issue ids consistent
braid import .beads/issues.jsonl
```

Import upserts by issue id and accepts both beads JSONL and braid's own
`braid export` output (which is also your backup / grep surface:
`braid export | grep -i crlf`). beads' integer comment ids are replaced
with collision-free string ids; beads-only fields (`source_repo`,
compaction machinery, etc.) are dropped.

Export records conform to a published JSON Schema —
[`schemas/strand.schema.json`](schemas/strand.schema.json) — the contract
for downstream tooling; see [the JSONL contract](schemas/) for import
tolerances and the deltas vs the automerge document shape.
