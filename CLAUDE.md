Monorepo: shared axioms here, project-specific rules in each
subproject's own `CLAUDE.md`.

Subprojects:
- `ccg/` — TSOT (the card game). See `ccg/CLAUDE.md`.
- `roam/` — the open-world game. See `roam/CLAUDE.md`.

These are independent. Architecture, dependencies, and patterns do
not transfer from one to the other unless the integration is
explicit. The only confirmed integration: v0.5 of roam invokes the
ccg engine to resolve player-vs-player encounters. Anything else is
confabulation.

---

roam AWS: `AWS_PROFILE=sbvh` (account `548351057127`).

---

**A commit is verified code** — the developer has tested it and confirmed it matches intent. Uncommitted changes in the working tree ARE the running code. Never use commit history to determine what is or isn't running. We develop and test on the same machine as where you are running, you do not need to commit in order to get the changes to me, we test here, and if its good, only then we commit.

The only development flow that works for the user is: Talk, discuss, refine, suggest, proof, specify/define, develop, build+run, test, verify, hand-off, ...

After hand-off the cycle starts again, hand-off means you have done everything you can do where you dont need the user's input or testing of verification. User verification happens after your verification, during the hand-off. After which we repeat the cycle or deem it commit worthy.

---

Strict TDD hard-requirement.
Meaning; write a failing test FIRST,
capture the intent and then continue with planned development.

---

Given that this project is still in it's early development:
**Errors are sacred** — first-class citizens, never collapsed,
dropped, swallowed or suppressed.

This is a hard hard-requirement:
Errors land in front of the user,
contextually in points of interaction,
so the developer/user knows what to do next.

If an error is not visible or surfaced, drop everything you do,
and make sure we see the error FIRST before continuing with anything else.

Every refusal surfaces typed at the user's cursor.

**KNOW** the developer is always running the latest code.
If there is an issue, it is in the code.

- Never ask if the developer has rebuilt/restarted
- Never suggest running build commands
- Do not remind about rebuild steps, it's poor DX anyways

---

When running long jobs (probes, EA, builds, `cargo test`): write to a
file the user can tail, never `| tail -N` or `| head -N` the live
stream. Truncating the output is optimizing your own legibility at the
cost of the user's visibility into the run.

Each subproject has its own `Makefile` for fast daily driver developer
convenience. No parameters, just: `make the_thing` from inside the
subproject's directory.
