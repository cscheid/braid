# `braid-housekeeping` skill

## Overview

A user-invoked skill that runs periodic chores from the
`cargo xtask housekeeping` report and opens one PR for review. See
`xtask-housekeeping.md` for the command itself. `main` now requires PRs, so this
also updates the `CLAUDE.md` git rule and `braid-release`.

## Work items

- [x] Drift test: the skill must mention every report check
- [x] `.claude/skills/braid-housekeeping/SKILL.md`
- [x] `CLAUDE.md`: pushing fresh branches and opening PRs is fine
- [x] `braid-release`: release through a PR
- [x] First real run (PR #27), and fix what it shows
- [x] `npm-lockfile` check: npm gets the crate flags
- [x] `HOUSEKEEPING:` markers and the `notes` check

## Decisions

- `disable-model-invocation`: invoking this skill authorizes a push, so it must
  only ever run because someone asked.
- Notes are for workarounds waiting on something external; strands are for work.
  The rule is in `CLAUDE.md`.
- The lockfile check comes before any build, since building runs the new code.
- `samod` and `automerge` are held back from routine bumps.

## Follow-up

- An xtask check comparing tools named in CI, `build.rs` and xtask against the
  Prerequisites table. For now this is a judgment step in the skill.
- Before unattended runs: a separate identity (GitHub App), a sandbox, a `v*` tag
  ruleset, and moving the signing key into a protected environment.
