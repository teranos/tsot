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

### Variance measurement (commit `34e1453`)

Measured on the current card pool (`cargo test --release measure_fitness_variance -- --ignored --nocapture`):

```
within-genome stddev (1 baseline, 10 base_seeds):
  n=3:  0.037     n=5:  0.053     n=10: 0.043     n=20: 0.032
between-genome stddev (10 random genomes, 1 base_seed):
  n=3:  0.111     n=5:  0.057     n=10: 0.088     n=20: 0.076
SNR (between / within):
  n=3:  2.99      n=5:  1.08      n=10: 2.03      n=20: 2.38
```

Random decks span fitness `[0.32, 0.60]` against the gauntlet — the engine
discriminates decks; there is something to optimize.

The n=3 SNR is misleading: with K=10 stddev estimates, the within-stddev
itself has high uncertainty at small N. Same caveat on the n=5 between-stddev
outlier (0.057 vs the 0.08-0.10 cluster at n=10/20).

**Revised recommendation: N=10.** SNR=2.0 is comfortable signal; n=20 buys
little for 2× the cost; n=5 is too noisy. Per-evaluation cost ~280ms,
100 × 100 EA run ≈ 47 minutes wall.

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

End-to-end EA. CLI subcommands: `tsot evolve`, `tsot champions-report`.

- `sim::genome` — `to_deck`, `random_genome` (`src/sim/genome.rs`)
- `sim::fitness` — `fitness`, `fitness_breakdown`, `build_gauntlet`, `GAUNTLET_MASTER_SEED=0xEA_C8` (`src/sim/fitness.rs`)
- `sim::ops` — `tournament_select`, `crossover_uniform`, `mutate`, `repair` (`src/sim/ops.rs`)
- `sim::evolve` — `evolve`, `EvolveConfig`, `EvolveResult`, `should_stop_at_ceiling` (`src/sim/evolve.rs`)
- `sim::evolved_deck` — `EvolvedDeck` save/load via JSON (`src/sim/evolved_deck.rs`)
- `main::run_ea` — CLI wiring, live per-generation progress + per-opponent breakdown + auto-save (`src/main.rs`)
- `main::run_champions_report` — aggregate card-frequency / pool-coverage / fitness-correlation across saved champions

## Usage

```bash
# Help
tsot --help
tsot evolve --help
tsot champions-report --help

# Single EA run, defaults (pop=50, gens=30, n=10, seed=0xEA_C8)
cargo run --release -- evolve --stop-at-ceiling 3

# Chain workflow (each champion fights the previous)
cargo run --release -- evolve --no-variants --extra champion.json --save champion.json

# Sample many independent champions at different seeds for aggregation
for s in 1 2 3 4 5 6 7 8 9 10; do
  cargo run --release -- evolve --seed $s --save "champions/champion-$s.json" --stop-at-ceiling 3
done

# Aggregate report across all champions in a directory
cargo run --release -- champions-report --dir champions/ --top 30
```

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

## Known limitations

See `LIMITATIONS.md` → "EA / evolutionary deck search" for the full
list with descriptions. Headline items: within-genome fitness noise
(±0.043 at n=10), below-noise-floor cards survive selection,
gauntlet overfit, no co-occurrence in the report, no diversity
preservation, no mid-run hall-of-fame, aging across card additions,
save path collisions, single fitness signal, sample-size threshold.
