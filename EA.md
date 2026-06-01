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
variant evolution is not interesting — random sampling already covers the
search space densely enough.

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

**Gauntlet:** curated EA-evolved decks loaded from `baselines/*.json` at
EA-mode startup. Each file is an `EvolvedDeck` JSON written by a prior
`tsot evolve` run that produced a strong, distinct attractor. The
initial baselines (5 decks) span the diversity discovered in the first
3 EA rounds: 2 eac8-cluster strategies, 1 ea03 flying-aggro strategy,
2 unrelated-seed singletons. The variant decks (`ra`, `rb`, `hu`, `go`,
`uu`, `pr`, `gg`) that the EA originally used were random samplings
from color pools — they were dropped because evolved opponents are
both stronger and more reproducible. `sim::variants` still exists, but
only for `deck_token` packing and the `fitness` test fixtures; no
runtime subcommand consumes them. Each evaluation:

```
for each opponent in gauntlet:
  for game_index in 0..N:
    play 1 game (genome as side A, opponent as side B)
    play 1 game (genome as side B, opponent as side A)
```

→ `2 × G × N` games per individual where G = baseline count + --extra
count. With the current 5 baselines and no extras: G=5 → 10 games per
individual at N=1, 50 at N=5, 100 at N=10. Add `--extra` files (e.g.
prior champions saved by the Makefile) to grow G. At ~2ms/game release:

| G  | N  | games | per-individual | 100×100 budget |
|----|----|-------|---------------|----------------|
| 5  | 10 | 100   | ~200ms        | ~33min         |
| 10 | 10 | 200   | ~400ms        | ~67min         |
| 20 | 10 | 400   | ~800ms        | ~133min        |

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

End-to-end EA. Four CLI subcommands: `tsot evolve`, `tsot matchup-evolved`,
`tsot champions-report`, `tsot curate-baselines`. No standalone matchup
mode anymore — random variant decks were dropped as opponents in favor of
the curated `baselines/` directory (see "Gauntlet" above).

Module layout:

- `sim::genome` — `to_deck`, `random_genome`
- `sim::fitness` — `fitness`, `fitness_breakdown` (gauntlet building used only by tests now)
- `sim::ops` — `tournament_select` (reads a `scores: &[f64]` vector → index; lets the caller pick raw fitness or a shaped vector for diversity-aware selection), `crossover_uniform`, `mutate`, `repair`
- `sim::diversity` — `jaccard`, `mean_jaccard_to_others`, `selection_scores`, `mean_pairwise_distance`; the shared Jaccard implementation used by both selection-time shaping and the post-hoc clustering CLIs
- `sim::evolve` — `evolve`, `EvolveConfig`, `EvolveResult`, `should_stop_at_ceiling`, `should_stop_at_plateau`; result carries `per_gen_card_freq` + `per_gen_mean_fitness` for the trajectory report
- `sim::evolved_deck` — `EvolvedDeck` save/load via JSON
- `sim::parallel_eval` — rayon thread-pool fitness evaluator with per-worker thread-local `CardRegistry`
- `cli_evolve` / `cli_matchup_evolved` / `cli_champions_report` / `cli_curate` — one CLI handler per subcommand
- `evolve_report` — HTML trajectory writer (fitness lines + card-presence heatmap per generation)
- `champions_report` — HTML aggregator (card frequency, clustering, pool coverage)

## Performance

The fitness evaluation step is the hot loop — every other phase of an
EA run is cheap by comparison. `sim::parallel_eval::parallel_evaluate_genomes`
fans evaluations across rayon's global thread pool with thread-local
`CardRegistry` + materialized gauntlet (mlua's `Lua` is `!Send`, so each
worker owns its own VM). Measured **3.4× speedup** on a 50-pop × 10-gen
run (130s → 38s wall) on an 8-core machine. The 25-min default run drops
to ~7-8 min. Not 8× because each worker pays Lua init cost (~500ms) and
the inner game loop serializes on internal allocation.

Determinism is preserved by the sequential generation phase that draws
all RNG decisions for a generation in order, followed by the parallel
fitness step which is a pure function of `(genome, gauntlet, fit_seed)`.
Same `cfg` → byte-identical `EvolveResult`.

## Usage

The Makefile is the supported entry point. Run `make help`:

```
make evolve              one EA round (~7-8min); auto-numbered, unique seed, top-5 → champions/
make report              HTML champions-report with --sample-games 50
make curate-baselines    live re-evaluate champions, promote winners into baselines/
make prune-champions     cluster champions by Jaccard, keep top-K per cluster, delete the rest
make matchup-decks       round-robin grid; DIR=baselines (default) or DIR=champions
make evolve-deep         deeper EA run (~2-8h): pop=100 gens=100 n=30 k=5
make clean-champions     wipe champions/ and report HTMLs
make pool                static card-pool analytics dashboard → card-pool.html (Lua, no rebuild)
make archetypes          cluster decks by Jaccard → archetypes-report.html (Lua, no rebuild)
```

The `prune-champions` target answers the "gauntlet keeps growing" problem.
Without it, each round adds 5 champions and the per-individual game count
grows linearly. Pruning by archetype keeps the gauntlet bounded by
(distinct archetypes × keep-per-cluster). After a few rounds run
`make prune-champions` to deduplicate slot variations.

Round counter uses the highest existing `r{N}-rank1.json` in
`champions/` and increments. Seed = `0xEA00 + N` so successive rounds
explore different attractors.

Direct CLI invocation (for ad-hoc runs):

```bash
cargo run --release -- evolve --help
cargo run --release -- evolve --seed 0xea08 --pop 25 --gens 15 --n 5
cargo run --release -- evolve --no-variants --extra champions/r7-rank1.json --save champion.json
```

## Open design questions

- **Per-card cap.** 3 matches the existing builder. 4 widens the search.
  Card-game intuition says 3 is the sweet spot for diversity; the EA itself
  will tell us if 4-of strategies dominate.
- **Diversity preservation.** *Resolved 2026-06-01.* Jaccard fitness penalty
  on tournament selection (`--diversity-alpha`, `sim::diversity::selection_scores`).
  Elitism still carries by raw fitness. Fitness sharing (Goldberg) is the
  next relative if α-tuning plateaus.
- **Multi-objective.** Win-rate is one signal. Game-length variance, mill
  imbalance, board-state diversity all carry information. Single-objective
  first; revisit if the evolved decks all look the same.
- **Persistence.** Per-individual JSON gets unwieldy fast. SQLite (one row
  per evaluated genome, indexed by genome hash) is the natural fit and was
  already on the roadmap for matchup analytics. Defer until the EA produces
  enough individuals to justify it.

## Non-goals (explicit)

- Self-play co-evolution. Both sides evolving simultaneously is the
  natural extension but doubles the variance and halves the diagnostic
  value. Fixed gauntlet first; `curate-baselines` between rounds is the
  cheap substitute.
- Tuning for a specific card to win. The whole point is to let the engine
  surface what it rewards, not to confirm a hypothesis.

(The variant matchup runner that this branch replaced is gone entirely;
it was random samplings of color pools used as opponents, which turned
out to be a much weaker benchmark than evolved champions.)

## Known limitations

See `LIMITATIONS.md` → "EA / evolutionary deck search" for the full
list with descriptions. Headline items: within-genome fitness noise
(±0.043 at n=10), below-noise-floor cards survive selection,
gauntlet overfit, no co-occurrence in the report, no diversity
preservation, no mid-run hall-of-fame, aging across card additions,
save path collisions, single fitness signal, sample-size threshold.
