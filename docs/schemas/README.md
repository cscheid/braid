# braid JSONL contract

`braid export` writes the skein as JSONL: **one strand record per line,
sorted by id**, each line conforming to
[`strand.schema.json`](strand.schema.json) (JSON Schema draft 2020-12).
This is the contract for downstream tooling — dashboards, analyzers,
backups, anything that consumes braid data without speaking automerge.

Contract tests in `crates/braid/tests/schema_contract.rs` validate real
`braid export` output against the schema, so drift between the
implementation and this contract fails the test suite.

## Writer vs reader: strict out, tolerant in

The schema describes what braid **writes** (strict: required fields,
`additionalProperties: false`). `braid import` **accepts a superset**:

| import tolerance | normalized to |
|---|---|
| beads-style `dependencies` / `comments` as arrays | keyed maps |
| beads integer comment ids | fresh `c-<base36>` string ids |
| `"completed"` status | `"closed"` |
| missing `priority` / `issue_type` | `2` / `"task"` |
| missing timestamps / `created_by` | import time / `"unknown"` |
| unknown top-level fields (`source_repo`, `compaction_level`, …) | dropped |

Records already in braid's own export shape round-trip **byte-exactly**
(comment ids and map keys preserved).

Import *does* enforce the constraints the schema asserts — notably ids
must contain no colons or whitespace — because anything import accepts,
export will later emit, and braid's own output must conform to its own
contract.

## Relationship to the automerge document

The skein lives in an automerge document whose shape is one-to-one with
these records, with three deliberate deltas:

1. **Prose fields** (`description`, `design`, `acceptance_criteria`,
   `notes`, comment `text`) are collaborative `Text` objects in the
   document (concurrent edits interleave); in JSONL they are plain
   strings.
2. **`labels`** is a map-as-set in the document (key presence =
   membership); in JSONL it is a sorted, deduplicated array.
3. The document has a top-level `metadata` object
   (`schema_version`, `name`, `id_prefix`, `created_at`) and an `issues`
   map keyed by strand id. JSONL flattens to just the strands; metadata
   is not exported.

`dependencies` and `comments` are keyed maps in **both** representations
(keys are collision-free, so concurrent inserts converge).

## Versioning

The automerge document carries `metadata.schema_version` (currently
`1`); braid refuses documents with a different version. This JSONL schema
is version-matched to the braid release that ships it (see the
`$comment` in the schema) and evolves in lockstep: a new document schema
version implies a new strand schema. Downstream tooling pinning this
schema should re-validate when upgrading braid.

## Semantics consumers commonly need

- **active** statuses: `open`, `in_progress`; **terminal**: `closed`.
  Unknown statuses: treat as neither.
- **ready** = active status and no `blocks`/`conditional-blocks`/
  `waits-for` edge whose target exists in the skein with a non-terminal
  status. (`parent-child` never blocks the child; open children gate the
  *parent's* close instead.)
- Dependency map keys are `<depends_on_id>:<type>` — informational;
  read the entry's fields rather than parsing keys.
- Dangling dependency targets are legal and never block.
