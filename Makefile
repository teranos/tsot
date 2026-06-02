# tsot — EA workflow shortcuts.
#
# Daily-use commands:
#   make evolve              one EA round (~25min, auto-numbered, saves top-5 to champions/)
#   make report              HTML report aggregating champions/ with game samples
#   make curate-baselines    live re-evaluate champions, promote winners into baselines/
#
# Inspection / occasional:
#   make matchup             original 7×7 variant grid → tsot-report.html
#   make matchup-decks       round-robin grid over a deck directory (DIR= override)
#                            default: DIR=baselines/  | override: make matchup-decks DIR=champions
#
# Power user:
#   make evolve-deep         deeper EA run (~2-8h): pop=100 gens=100 n=30
#
# Reset:
#   make clean-champions     wipe champions/ and report HTMLs
#
# Each round uses base_seed = 0xEA00 + round_number so different rounds
# explore different attractors.

CHAMPS := champions
HTML   := champions-report.html
DIR    ?= baselines

# Jaccard fitness penalty for diversity-preserving selection. Tournament
# reads `fitness - ALPHA · mean_jaccard_to_others`. Default 0.1 is a
# starting point — bump to 0.2-0.3 if top-5 still cluster heavily, drop
# to 0 to disable. CLI flag default is 0.0 so bare `tsot evolve` stays
# byte-identical to pre-diversity runs; the Makefile opts in.
ALPHA  ?= 0.3

# Number of unmatched-champion representatives to promote to new
# baselines during `make curate-baselines`. Unmatched champions are
# first inner-clustered among themselves at the same threshold (0.7
# single-linkage Jaccard), one rep per inner-cluster is picked by
# live-eval score, top-K reps are written as new baselines. Default 1
# = grow baselines by at most one new attractor per curate run; set to
# 0 to disable, or bump higher for a one-shot bulk promotion.
PROMOTE ?= 1

# Early stop for `make evolve*`: halt when best-of-generation has
# improved by `<= PLATEAU_EPS` for `PLATEAU_K` consecutive generations.
# Elitism guarantees monotonic non-decreasing best, so PLATEAU_K=4 with
# eps=0.010 means "less than 1% improvement four turns in a row → done."
PLATEAU_K   ?= 4
PLATEAU_EPS ?= 0.010

.PHONY: help matchup-decks evolve evolve-deep report curate-baselines clean-champions pool archetypes prune-champions probe probe-long matchup-mcts

help:
	@echo ""
	@echo "Daily-use:"
	@echo "  make evolve              one EA round (~25min); auto-numbered round, unique seed, top-5 → $(CHAMPS)/"
	@echo "  make report              HTML champions-report aggregating $(CHAMPS)/ (50 sample games/champion)"
	@echo "  make curate-baselines    live re-evaluate champions, promote winners into baselines/"
	@echo "  make prune-champions     cluster champions by Jaccard, keep top-K per cluster, delete the rest"
	@echo ""
	@echo "Matchup grid:"
	@echo "  make matchup-decks       round-robin grid over a deck directory (DIR=baselines default)"
	@echo "                           override: make matchup-decks DIR=champions"
	@echo ""
	@echo "Power-user:"
	@echo "  make evolve-shallow      fast EA round (~1-2min): pop=25 gens=10 n=5 — smoke check, noisy fitness"
	@echo "  make evolve-deep         deeper EA run (~2-8h): pop=100 gens=100 n=30 k=5"
	@echo ""
	@echo "  make clean-champions     wipe $(CHAMPS)/ and $(HTML)"
	@echo ""
	@echo "Card design:"
	@echo "  make pool                static analytics dashboard → card-pool.html (chains curve-sample for the turn-played column; POOL_NO_CURVE=1 to skip)"
	@echo "  make archetypes          cluster decks by Jaccard → archetypes-report.html (Lua, no rebuild)"
	@echo "  make probe [CARD_ID...]  side-by-side compare a card's declared variants (auto-discover if no id)"
	@echo "  make probe-long [...]    same as probe but pop=30 gens=15 n=30 (~3min/variant, σ≈0.025)"

matchup-decks:
	cargo run --release -- matchup-evolved --dir $(DIR) --html matchup-$(notdir $(DIR)).html

evolve:
	@mkdir -p $(CHAMPS)
	@HIGHEST=0; for f in $(CHAMPS)/r*-rank1.json; do [ -f "$$f" ] || continue; base=$$(basename "$$f" -rank1.json); num=$${base#r}; if [ "$$num" -gt "$$HIGHEST" ]; then HIGHEST=$$num; fi; done; \
	N=$$((HIGHEST + 1)); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	NUM=$$(($$(echo $$EXTRAS | wc -w | xargs) / 2)); \
	echo "=== round $$N (seed=$$SEED, gauntlet: 5 baselines + $$NUM extras, alpha=$(ALPHA)) ==="; \
	cargo run --release -- evolve --seed $$SEED --stop-at-ceiling 3 --save-top 5 \
		--diversity-alpha $(ALPHA) \
		--stop-at-plateau $(PLATEAU_K) --plateau-eps $(PLATEAU_EPS) \
		$$EXTRAS --save $(CHAMPS)/r$$N.json

evolve-shallow:
	@mkdir -p $(CHAMPS)
	@HIGHEST=0; for f in $(CHAMPS)/r*-rank1.json; do [ -f "$$f" ] || continue; base=$$(basename "$$f" -rank1.json); num=$${base#r}; if [ "$$num" -gt "$$HIGHEST" ]; then HIGHEST=$$num; fi; done; \
	N=$$((HIGHEST + 1)); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	NUM=$$(($$(echo $$EXTRAS | wc -w | xargs) / 2)); \
	echo "=== shallow round $$N (seed=$$SEED, gauntlet: 5 baselines + $$NUM extras, alpha=$(ALPHA)) ==="; \
	echo "    pop=25 gens=10 n=5 — fast smoke check, noisy fitness"; \
	cargo run --release -- evolve --seed $$SEED \
		--pop 25 --gens 10 --n 5 --stop-at-ceiling 3 \
		--diversity-alpha $(ALPHA) \
		--stop-at-plateau $(PLATEAU_K) --plateau-eps $(PLATEAU_EPS) \
		--save-top 5 \
		$$EXTRAS --save $(CHAMPS)/r$$N.json

evolve-deep:
	@mkdir -p $(CHAMPS)
	@HIGHEST=0; for f in $(CHAMPS)/r*-rank1.json; do [ -f "$$f" ] || continue; base=$$(basename "$$f" -rank1.json); num=$${base#r}; if [ "$$num" -gt "$$HIGHEST" ]; then HIGHEST=$$num; fi; done; \
	N=$$((HIGHEST + 1)); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	NUM=$$(($$(echo $$EXTRAS | wc -w | xargs) / 2)); \
	echo "=== deep round $$N (seed=$$SEED, gauntlet: 5 baselines + $$NUM extras, alpha=$(ALPHA)) ==="; \
	echo "    pop=100 gens=100 n=30 tournament-k=5 elite=3 — expect 2-8 hours wall"; \
	cargo run --release -- evolve --seed $$SEED \
		--pop 100 --gens 100 --n 30 --tournament-k 5 --elite 3 --stop-at-ceiling 5 \
		--diversity-alpha $(ALPHA) \
		--stop-at-plateau $(PLATEAU_K) --plateau-eps $(PLATEAU_EPS) \
		--save-top 5 \
		$$EXTRAS --save $(CHAMPS)/r$$N.json

report:
	cargo run --release -- champions-report --dir $(CHAMPS) --html $(HTML) --sample-games 50
	@echo "Open $(HTML)"

curate-baselines:
	cargo run --release -- curate-baselines --promote-unmatched $(PROMOTE)

prune-champions:
	cargo run --release -- prune-champions

clean-champions:
	rm -rf $(CHAMPS) $(HTML) matchup-*.html

pool:
	@# Refresh the turn-curve data unless `POOL_NO_CURVE=1`. The Lua
	@# dashboard picks up `card-curve.json` if present and skips the
	@# turn-curve section otherwise.
	@if [ "$$POOL_NO_CURVE" != "1" ]; then \
		cargo run --release -- curve-sample $$CURVE_ARGS; \
	fi
	lua5.4 tools/cards-report.lua
	@echo "Open card-pool.html"

archetypes:
	lua5.4 tools/archetypes-report.lua
	@echo "Open archetypes-report.html"

# Mirror-match MCTS vs Heuristic. MCTS-vs-Heuristic + Heuristic-vs-MCTS
# back-to-back on the SAME random deck (eliminates deck-quality as a
# confounder). 50% = MCTS adds no signal; 55-65% = working; 70%+ = the
# heuristic has obvious gaps. Slow (~3-5 min/game at default rollouts);
# tune via MATCHUP_MCTS_ARGS.
MATCHUP_MCTS_ARGS ?=

matchup-mcts:
	cargo run --release -- matchup-mcts $(MATCHUP_MCTS_ARGS)

# Balance-probe runs the side-by-side EA over cards that declare
# variants inline in their .lua file (`variants = { [key] = { ... } }`).
#
#   make probe                       # auto-discover every card with variants
#   make probe dark-salamander       # probe just this card's variants
#   make probe-long                  # same auto-discovery, full-rigor params
#
# No paths. The LLM edits cards/*.lua; you type `make probe`. Variants
# are excluded from `make evolve` automatically (CardRegistry flags
# them is_variant = true; main.rs's playable_pool filter skips them).
#
# Positional card ids are captured from $(MAKECMDGOALS); the bare-word
# "goals" are swallowed by an empty rule so make doesn't error.
PROBE_CARD_GOALS := $(filter-out probe probe-long,$(MAKECMDGOALS))
PROBE_ARGS       ?=

probe:
	cargo run --release -- balance-probe $(PROBE_CARD_GOALS) $(PROBE_ARGS)

# Long-form probe: pop=30 gens=15 n=30 — about 3 min per variant,
# σ ≈ 0.025 (vs σ ≈ 0.043 at the default n=10). Use when first-pass
# `make probe` shows a small gap and you need to know if it's real.
probe-long:
	cargo run --release -- balance-probe $(PROBE_CARD_GOALS) --pop 30 --gens 15 --n 30 $(PROBE_ARGS)

# Swallow positional card-id goals so `make probe dark-salamander` doesn't
# try to build them as targets. ONLY declared when `probe` / `probe-long`
# is one of the requested goals — otherwise the empty rule would override
# every other real target (`evolve`, `pool`, ...) you might also be running.
ifneq (,$(filter probe probe-long,$(MAKECMDGOALS)))
$(PROBE_CARD_GOALS):
	@:
endif
