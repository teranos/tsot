
The Symbols of Teranos: ax = ⋈, ix = ⨳, am = ≡, pulse = ꩜, sem = ⊨

Collectible Card Game

Your role: write the code the user asks for.
Their requests stem from intuition about the game. 
When they ask something, do it. 

Write cards in lua cards/.

Use probe when you need to measure something.
Don't push probe results at the user. Don't ask them whether to
probe. Don't probe a single new card — there's nothing to compare.

When the user wants to compare alternative versions of a card (cost,
stats, effect magnitude), add a `variants = { [key] = { overrides } }`
block to the card's .lua — never duplicate the file. The loader
emits one card per variant; `make probe` picks them up side-by-side.

See @LUA.md and @RULES.md. Sim AI + game-runner internals
in @src/sim/README.md.

Strict TDD required. Meaning, write a failing test FIRST, capture the intent and then continue with planned development.

**KNOW** the developer is always running the latest version of TSOT. If there is an issue, it is in the code.

Never:
- Ask if the developer has rebuilt/restarted
- Suggest running build commands
- Remind about rebuild steps 
- Imply the running binary might be stale

**A commit is verified code** — the developer has tested it and confirmed it matches intent. Uncommitted changes in the working tree ARE the running code. Never use commit history to determine what is or isn't running.

**Errors are sacred** — first-class citizens, never collapsed,
dropped, or suppressed. They land in the LOG panel with every other
engine event.

When running long jobs (probes, EA, builds, `cargo test`): write to a
file the user can tail, never `| tail -N` or `| head -N` the live
stream. Truncating the output is optimizing your own legibility at the
cost of the user's visibility into the run.

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
