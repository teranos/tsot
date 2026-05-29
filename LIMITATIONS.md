# tsot — Known Limitations

> What the engine cannot do today, organized into four slices of work.
> Code TODOs are tagged with `events`, `costs`, `types`, or `stack` to map back here.

## 1. `events` — Lua execution and triggered-ability dispatch

Phase 1 is in progress. All six events from the v1 taxonomy (`on_enter_board`, `on_die`, `on_attack`, `on_block`, `on_blocked_by`, `on_play`) have wired fire sites and fire in the sim against real card handlers. The minimal `game` API surface (`damage`, `mill`, `draw`, `move`, `opponent`, `deck_top`) is exposed. **What remains in Phase 1:** the mlua sandbox is not enabled, and several API methods (`tap`, `untap`, `zones`, `card`, `add_status`, `discard`, `print`) are pending. **Phase 2** (choice, `static` continuous effects) is the next architectural slice and is the biggest gate on the corpus.

See `LUA.md` for the working status block.

**Scope:**
- Event taxonomy and emitter (enter-the-board, attack, block, damage, die, draw, discard, end-of-turn, …).
- Lua function-as-ability pattern: cards return `on_die = function(game, self) … end` etc.
- Trigger registry: when an event fires, look up matching handlers across cards in play.
- Game-side API surface for Lua (move, draw, choose, ask-player, etc.).
- Continuous-modifier dispatch (Squirrel-overrun's `+1/+1 per attached`, Companion Bird's flying-grant, Modern LCD Clock's cost reduction).
- Sandboxing (mlua sandbox mode).

**Unlocks:** every existing card's ability. mesopelagic-fish's death-trigger, stinging-bee's damage-lockdown, squirrel-overrun's attack-trigger, etc. — none of these fire today.

**Hard parts:** designing the game-API surface that Lua scripts can call. Once that exists, individual card abilities are typically 5–20 LOC of Lua each.

## 2. `costs` — Cost-source coverage

`play_card` supports **HAND**, **MILL**, and **GRAVEYARD** cost sources (GRAVEYARD currently selects deterministically — most-recent N exiled — pending the choice API). Two sources and variable X remain.

**Scope:**
- `SACRIFICE` cost (P.16): pick a creature you control, move BOARD → GRAVEYARD. Used by flesh-eating-plant.
- `SELF` cost (P.5): the played card itself to EXILE. Used by opponent-draw.
- Variable X (`is_x` flag): player picks X at cast time. Used by hydra, recast, stream-of-thought, DTST-creature2.
- Linked-X across cost components (recast: `X hand` + `X graveyard` share the same X).

**Unlocks:** flesh-eating-plant, opponent-draw, hydra, recast, stream-of-thought — every card with one of the remaining cost sources.

**Hard parts:** the API for "player picks N cards from zone Z" needs to live on the choice surface (UX X-E.1, X-E.2). Linked-X needs schema consideration.

## 3. `types` — Non-creature card-type plays

`play_card` handles **Creature** and **Instant** (routes to GRAVEYARD per P.1; timing per C.6 not yet enforced). The other three types remain.

**Scope:**
- **SPELL** (C.9–C.10) → GRAVEYARD on play; only during controller's turn.
- **ARTIFACT** (P.19) → BOARD.
- **ENVIRONMENT** (P.21) → BOARD, subject to P.22 (one at a time, global) and P.23 (can't replace).
- Timing checks (U.7, U.8): non-instants only legal in Main phases.

**Unlocks:** silent-murder, easy-pickings, glaring-sunlight, modern-lcd-clock, amsterdam-city — every card of one of the remaining types.

**Hard parts:** mostly straightforward branching in `play_card`. Environment slot-management (P.22/P.23) needs the displacement question resolved (currently new can't be played while old exists).

## 4. `stack` — Response windows and priority

R.1–R.7 describe a recursive response chain. None of it exists in code.

**Scope:**
- Open a response window on (a) card played, (b) attack declared per R.1.
- Track the response chain in GameState (an additional state field).
- Priority sequence: active player first (R.7), then non-active, alternating until both pass.
- LIFO resolution (R.2). Recursive responses (R.4).
- Window closes when chain is empty and both pass (R.5, R.6).
- Smart prompting per UX X.1–X.7: skip when no playable instants, skip when no legal target, show the active player what they're waiting on, tight timer, hold-priority, pre-declared responses, distinguish unopposed vs declined.
- Engine introspection per UX X-E.1–X-E.5: `playable_instants`, `legal_targets`, queued-response register, resolution-event metadata.

**Unlocks:** all instant casting and combat-trick play. Until this exists, falter and silent-murder and draw-two are unreachable mid-combat.

**Hard parts:** the priority/pass state machine is fiddly. UX skip-logic depends on knowing "what would the opponent's legal responses be," which requires the introspection API.

---

## Cross-cutting: journal & rollback

Independent of the four themes but enabling for all of them: tsot's
engine mutates state at scattered sites with no way to preview, rewind,
or replay. The plan is a journal-based mutation log with per-entry
inverses. Foundation for sim AI preview (skip suicide plays), replay
capture, save / load, undo, AI search trees, and multiplayer rollback.

See `JOURNAL.md` for the multi-session plan.

---

## Other smaller items (not their own theme)

- **Mulligan** (S.2/S.3) — small once we wire UI choice.
- **Control changes** (T.1 — current code assumes owner == controller).
- **P.8 attached → EXILE on host's death** (currently dropped on the floor).
- **Decks aren't shuffled by the engine** (sim does this manually; future deck loader should too).
- **No artifact-as-permanent** death/destruction rules separate from P.4 (P.4 is creature-specific).
- **No "wall" type or rules**, deliberately purged.

These slot into one of the four themes or are independent small items handled per-need.

---

## Code TODOs

Code sites are tagged `// TODO(events|costs|types|stack): …` and reference rule IDs.
Grep `grep -rn 'TODO(' src/` for the full list.
