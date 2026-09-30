# The blind agent test

After a big change to the kit or its docs, a fresh agent that knows nothing but the repository builds a real mod.
Where it stumbles, the docs or tools are wrong or missing. Each run so far found things no review did.

## How to run it

1. Start an agent in its own git worktree (Claude Code: an `Agent` with `isolation: worktree`), with no context
   from your session.
2. Give it a task the way a user would, and the rules. Pick a mod that doesn't exist yet and needs something the
   last test didn't (a new registry, a new game system, a bug to find).

```text
You work in openlina-kit (your working directory), a toolkit for modding the game Mosa Lina. The game is installed
through Steam. Nix is available (`nix develop`).

Task: <one paragraph, like a user would write it: what the mod does, its options, "tests that can really fail",
a showcase gif, a docs entry. "If you find bugs in the kit, you may fix them.">

Rules:
- Install or update nothing (no rustup targets/toolchains, system packages, `nix profile install`); use what is
  there. If something is missing, note it in the report.
- Don't change the game install, Steam or its settings, ~/.local/share/openlina, or the user's saves.
- Upload nothing (no `lina publish --yes`, no `git push`), log in nowhere. Commits in the current branch are fine.
- Delete nothing outside this repository.

At the end: commit, and report
1. what you built (files, tests, `lina test` results for your mod and for everything),
2. how you went about it, with rough time per phase (reading, finding code, implementing, testing, docs),
3. every friction point: what in the docs was missing, wrong or unclear, which commands or tools didn't work or
   were missing, where you guessed. Concretely, with file/command. This is the most important part.
```

3. Check its claims (run its tests yourself), merge the branch, then work through the friction points: fix the
   tool or the doc it names, and record the run below.

## Runs

| date | task | time | what it found |
|---|---|---|---|
| stage 4 | ammo-boost (general) | ~50 min | docs gaps in the first playbook (fixed in cc769c6) |
| 2026-09-30 | swap (item, ray cast) | long | a toolchain file that updated the user's rustup, no ray cast helper, no position checks, `lina fn` couldn't find injected functions, gif order, … (8 points) |
| 2026-10-01 | moon-gravity (modifier, game physics) | ~20 min | `lina fn --ops` printed hundreds of registers, `lina refs`/`callers` ignored natives, no position bounds, seed-dependent roll tests, `lina probe` couldn't set harness options, a misleading trace-positions message, undocumented jump/tutorial behavior, noisy templates and doctor output (all fixed) |
