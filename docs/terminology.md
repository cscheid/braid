# braid terminology

Decided 2026-06-03 (see strand `br-collection-noun-olwt3b32`, closed). The
vocabulary follows the fiber metaphor and avoids overloading the program's
own name.

| term | meaning |
|---|---|
| **braid** | the program: the CLI and its library crates. Never the data. |
| **skein** | the collection of all issues tracked for a project. One skein = one automerge document; the doc id in `.braid.toml` identifies (and grants access to) a skein. Replaces the generic "tracker". |
| **strand** | a single issue within a skein. "File a strand", "close a strand". |
| **loom** | *reserved*: a sync-server peer — in particular a future samod-based local relay binary, where skeins are exchanged. Not yet built; don't use it for anything else. |

Why these words:

- *skein* — a loosely wound bundle of strands waiting to be worked. The
  metaphor composes: issues are **strands**; doing the work is braiding
  them; the unworked collection is the **skein**. The word is rare enough
  to grep for and to never collide with the program name in prose or error
  messages ("braid not found" vs "skein not found").
- *strand* — short, ordinary, and exactly right for "one fiber of the
  braid".
- *loom* — the fixed structure braiding happens around: apt for the
  always-on server that ephemeral braid invocations visit.

Usage notes:

- Terminology governs **prose**: docs, CLI output, error messages,
  comments. It does not change interfaces: command names stay (`braid
  create`, not `braid strand`), and the `.braid.toml` fields and JSON
  schema field names are stable.
- Adoption across existing code/docs is tracked by strand
  `br-skein-rename-ato091k5`.

Pronunciation, for the curious: skein rhymes with "rain" (/skeɪn/).
