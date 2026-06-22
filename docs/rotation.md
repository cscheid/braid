# Rotation: history compaction and leak recovery

A skein's automerge document keeps full history forever, and its doc id
is an irrevocable capability. `braid rotate` addresses both: it exports
the current state into a **fresh document** (shedding history), marks the
old document rotated, and switches your `.braid.toml`. Stale clones get a
clear error and run `braid rotate --adopt` to follow; changes they made
after the cutover are detected and written to `.braid-stragglers.jsonl`
for review and re-import.

If the doc id has **leaked**, use `braid rotate --revoke`: identical
mechanics, except no forwarding pointer is written into the old document
(the attacker can read it — a pointer would hand them the new
capability). Distribute the new secret out-of-band with `braid secret`.

Honest limits: rotation protects *future* reads and writes. The old
document's history remains readable forever to anyone holding the old id;
revocation cannot un-leak the past.
