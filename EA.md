# EA — Evolutionary Deck Search

## Question

Random sampling from variant pools tells us "given these 7 archetypes, which
wins more on average." It does not tell us **what card combinations the
engine actually rewards**. EA flips the search: treat the 50-card deck as a
genome, evaluate it against a fixed gauntlet, let selection surface
combinations no human (or variant designer) would have written down.

## Scope decision: open pool

Variant becomes a yardstick, not a constraint. The full `playable_pool`
(every cast-supported card across all variants) is the gene pool. Within-
variant evolution is not interesting at the current card count (~25 playable
ids) — random sampling already covers the search space densely enough.

## Genome

```
Genome = Vec<String>   // 50 card ids, order doesn't matter for game,
                       //   but is preserved to make crossover well-defined
```

The 50 cards are drawn (with multiplicity) from `playable_pool`. Hard
constraints enforced by the EA loop, not the genome type:

- length = 50
- per-card cap (start at **3**, matches the deck-builder's default)
- every id ∈ `playable_pool` (the only thing `to_deck` validates)

Nothing else. No color balance, no curve, no variant labels. Let selection
pressure produce whatever structure emerges.

## Fitness

```
fitness(genome) -> f64   // win-rate in [0, 1]
```

**Gauntlet:** the 7 current variant decks (`ra`, `rb`, `hu`, `go`, `uu`,
`pr`, `gg`), built from a fixed master seed so the opponent population
stays stable across generations. Each evaluation:

```
for each opponent in gauntlet:
  for game_index in 0..N:
    play 1 game (genome as side A, opponent as side B)
    play 1 game (genome as side B, opponent as side A)
```

→ `2 × 7 × N` games per individual. At ~2ms/game release:

| N  | games | per-individual | population × generations budget |
|----|-------|---------------|---------------------------------|
| 3  | 42    | ~84ms         | 100 × 100 ≈ 14min               |
| 5  | 70    | ~140ms        | 100 × 100 ≈ 23min                |
| 10 | 140   | ~280ms        | 100 × 100 ≈ 47min                |

N=5 is the starting point — variance still real, cost manageable.

## Operators

**Selection:** tournament, k=3. Simple, no fitness-scaling needed.

**Crossover:** uniform over slots, then **repair**:
- if a card-id count exceeds the per-card cap, swap excess copies for
  randomly drawn ids from `playable_pool`
- if length ≠ 50 (shouldn't happen with uniform crossover, but defensive),
  truncate or pad

**Mutation:** swap `k` random slots for random ids from `playable_pool`,
`k ~ Poisson(λ=1.5)`. Repair afterward.

**Elitism:** carry the top-1 individual unchanged each generation. Avoids
losing a strong individual to a bad recombination.

## What's built

- `CardRegistry::get(id) -> Option<&Card>` (`src/card.rs`)
- `sim::genome::to_deck(registry, &[String]) -> Result<Vec<Card>, GenomeError>` (`src/sim/genome.rs`)

Genome→deck materialization. Tested. Used by nothing yet.

## What's next (irreducibly ordered)

1. **`fitness(genome)`** — calls `to_deck`, builds gauntlet, runs `2 × 7 × N`
   games, returns win-rate. **Decision required:** N (above) and gauntlet
   seed (suggest: a constant, not `pick_seed()` — stable benchmark).
2. **Random initial population** — `random_genome(pool, len, cap, rng)`.
3. **Operators** — `tournament_select`, `crossover_uniform`, `mutate_swap`,
   `repair`.
4. **Generation loop** — `evolve(generations, pop_size) -> Vec<(Genome, f64)>`.
5. **Binary** — `cargo run --bin evolve`, prints best-of-each-generation
   to stdout + writes final population to `evolved-*.json`.

Steps 1–4 are pure functions on `Vec<String>` and a `&CardRegistry`. Step 5
is the I/O wrapper.

## Open design questions

- **Per-card cap.** 3 matches the existing builder. 4 widens the search.
  Card-game intuition says 3 is the sweet spot for diversity; the EA itself
  will tell us if 4-of strategies dominate.
- **Diversity preservation.** Without it, the population collapses onto one
  local maximum within ~20 generations. Cheap fix: similarity penalty on
  selection (Jaccard distance on card-id multisets). Defer until we see it
  happen.
- **Multi-objective.** Win-rate is one signal. Game-length variance, mill
  imbalance, board-state diversity all carry information. Single-objective
  first; revisit if the evolved decks all look the same.
- **Persistence.** Per-individual JSON gets unwieldy fast. SQLite (one row
  per evaluated genome, indexed by genome hash) is the natural fit and was
  already on the roadmap for matchup analytics. Defer until the EA produces
  enough individuals to justify it.

## Non-goals (explicit)

- Replacing the matchup-runner. The runner stays the answer to "which
  archetype wins on average." EA answers "what's possible outside the
  archetypes."
- Self-play co-evolution. Both sides evolving simultaneously is the
  natural extension but doubles the variance and halves the diagnostic
  value. Fixed gauntlet first.
- Tuning for a specific card to win. The whole point is to let the engine
  surface what it rewards, not to confirm a hypothesis.
