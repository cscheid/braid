# Rotation: history compaction and leak recovery

A skein's automerge document keeps full history forever, and its doc id
is an irrevocable capability. `braid rotate` addresses both: it exports
the current state into a **fresh document** (shedding history), marks the
old document rotated, and switches your `.braid.toml`. Stale clones get a
clear error and run `braid rotate --adopt` to follow; changes they made
after the cutover are detected and written to `.braid-stragglers.jsonl`
for review and re-import.

Rotation is also how a skein created before braid 0.8 gets the speedup from
scalar prose. Those skeins store prose as `Text`, at one operation per
character, and that history stays in the document until a rotation leaves it
behind. braid works on them unchanged, just more slowly.

If the doc id has **leaked**, use `braid rotate --revoke`: identical
mechanics, except no forwarding pointer is written into the old document
(the attacker can read it — a pointer would hand them the new
capability). Distribute the new secret out-of-band with `braid secret`.

## When the doc id isn't in `.braid.toml`

braid rewrites `.braid.toml` itself, but it can't rewrite
`~/.config/braid/projects.toml` or an environment variable. In those setups,
both `braid rotate` and `braid rotate --adopt` print the new `doc_id` on stdout
and tell you what to update. `--adopt` first checks that the successor loads.
The clone keeps reporting the rotation until you make the change.

All clones that share a `projects.toml` entry switch together when you update
it, so they don't each need `--adopt`. The printed id grants read and write
access, so keep it out of logs and shared text.

Honest limits: rotation protects *future* reads and writes. The old
document's history remains readable forever to anyone holding the old id;
revocation cannot un-leak the past.
