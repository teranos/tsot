
The Symbols of Teranos: ax = ⋈, ix = ⨳, am = ≡, pulse = ꩜, sem = ⊨, delta = δ

WASM - Collectible Card Game

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

**KNOW** the developer is always running the latest version of TSOT.
If there is an issue, it is in the code.

- Never ask if the developer has rebuilt/restarted
- Never suggest running build commands
- Do not remind about rebuild steps, it's poor DX anyways

---

When running long jobs (probes, EA, builds, `cargo test`): write to a
file the user can tail, never `| tail -N` or `| head -N` the live
stream. Truncating the output is optimizing your own legibility at the
cost of the user's visibility into the run.

The @Makefile is for fast daily driver developer convenience,
no parameters, just: `make the_thing`

---

Write cards in .lua, see: cards/

You can compare alternative versions of a card (cost, stats, effect magnitude),
`make probe` recognises `variants = { [key] = { overrides } }` blocks on cards.

Don't push probe results at the user. Don't ask them whether to
probe. Don't probe a single new card — there's nothing to compare.

See @LUA.md and @RULES.md. Sim AI + game-runner internals
in @src/sim/README.md.

Balance, power level, "premium cost", card-economy ratios, win-conditions,
archetype viability — not chat answers. You have no playtest data and no
game intuition; don't editorialize. Just write the card or the mechanic.
If the user asks for balance input, point them at sim output or write
the analysis.

Same applies to strategy talk: lines of play, optimal sequencing,
"tempo", matchup analysis, what a player "should" do with a card,
deckbuilding heuristics, archetype labels, combo identification. You
don't play the game and your strategic intuitions are confabulated
from CCG pattern-matching. Don't generate it. Cards, mechanics, rules,
engine — yes. Strategy — no.
