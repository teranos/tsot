# The Symbols of Teranos — UX Requirements

> Working draft. Each item has a stable identifier for review and reference.
> These are baseline requirements for any tsot interface, not optional polish.

## Response-prompt behavior (X)

- **X.1** If the responding player has no playable instant in hand at a response window, the engine resolves without prompting them.
- **X.2** If the responding player has playable instants but none have a legal target in the current state, the engine resolves without prompting them.
- **X.3** When the responding player is being prompted, the active player sees what they are waiting on: which event, which playable response(s) the opponent has.
- **X.4** Each response prompt has a tight timer. On timeout, the engine auto-passes.
- **X.5** The active player may explicitly hold priority to chain their own responses (e.g., draw-two into silent-murder). Otherwise the engine auto-passes for the active player on resolution.
- **X.6** Players may pre-declare conditional responses: "if event E occurs during the opponent's turn, attempt to play card C." The engine consults these before prompting.
- **X.7** Resolution events are visually distinguishable by cause: `resolved_unopposed` (no response was possible) vs `resolved_after_decline` (a response was possible and declined).

## Engine API implications (X-E)

These are surfaces the engine must expose so a UI can implement X.1–X.7.

- **X-E.1** `playable_instants(player, state) → Vec<Card>` — for X.1. Filters by current state: cost payable, type allowed at this moment, not under any "can't play" effect.
- **X-E.2** `legal_targets(card, state) → Vec<Target>` — for X.2. Per card: what does this card's targeting predicate match in the current state?
- **X-E.3** Response-window introspection: each open window exposes its `cause` (the played card or declared attack) and the responder's filtered playable options.
- **X-E.4** Queued-response register: per player, a list of `(condition, intended_action)` pairs the engine consults on every event.
- **X-E.5** Resolution events carry metadata distinguishing `unopposed` from `declined`.
