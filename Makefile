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

.PHONY: help matchup-decks evolve evolve-deep report curate-baselines clean-champions

help:
	@echo ""
	@echo "Daily-use:"
	@echo "  make evolve              one EA round (~25min); auto-numbered round, unique seed, top-5 → $(CHAMPS)/"
	@echo "  make report              HTML champions-report aggregating $(CHAMPS)/ (50 sample games/champion)"
	@echo "  make curate-baselines    live re-evaluate champions, promote winners into baselines/"
	@echo ""
	@echo "Matchup grid:"
	@echo "  make matchup-decks       round-robin grid over a deck directory (DIR=baselines default)"
	@echo "                           override: make matchup-decks DIR=champions"
	@echo ""
	@echo "Power-user:"
	@echo "  make evolve-deep         deeper EA run (~2-8h): pop=100 gens=100 n=30 k=5"
	@echo ""
	@echo "  make clean-champions     wipe $(CHAMPS)/ and $(HTML)"

matchup-decks:
	cargo run --release -- matchup-evolved --dir $(DIR) --html matchup-$(notdir $(DIR)).html

evolve:
	@mkdir -p $(CHAMPS)
	@N=$$(( $$(ls $(CHAMPS)/r*-rank1.json 2>/dev/null | wc -l) + 1 )); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	NUM=$$(($$(echo $$EXTRAS | wc -w | xargs) / 2)); \
	echo "=== round $$N (seed=$$SEED, gauntlet: 5 baselines + $$NUM extras) ==="; \
	cargo run --release -- evolve --seed $$SEED --stop-at-ceiling 3 --save-top 5 \
		$$EXTRAS --save $(CHAMPS)/r$$N.json

evolve-deep:
	@mkdir -p $(CHAMPS)
	@N=$$(( $$(ls $(CHAMPS)/r*-rank1.json 2>/dev/null | wc -l) + 1 )); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	NUM=$$(($$(echo $$EXTRAS | wc -w | xargs) / 2)); \
	echo "=== deep round $$N (seed=$$SEED, gauntlet: 5 baselines + $$NUM extras) ==="; \
	echo "    pop=100 gens=100 n=30 tournament-k=5 elite=3 — expect 2-8 hours wall"; \
	cargo run --release -- evolve --seed $$SEED \
		--pop 100 --gens 100 --n 30 --tournament-k 5 --elite 3 --stop-at-ceiling 5 \
		--save-top 5 \
		$$EXTRAS --save $(CHAMPS)/r$$N.json

report:
	cargo run --release -- champions-report --dir $(CHAMPS) --html $(HTML) --sample-games 50
	@echo "Open $(HTML)"

curate-baselines:
	cargo run --release -- curate-baselines

clean-champions:
	rm -rf $(CHAMPS) $(HTML) matchup-*.html
