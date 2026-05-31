# tsot — EA workflow shortcuts.
#
# Common loop:
#   make evolve              # round 1: 7 variants only, saves 5 ranks to champions/r1-rank*.json
#   make evolve              # round 2: 7 variants + round-1 champions
#   make evolve              # round 3: 7 variants + rounds 1-2 champions
#   make report              # HTML report aggregating every champion in champions/
#   make clean-champions     # start over
#
# Each round uses a unique base_seed (0xEA00 + N) so successive rounds
# explore different attractors instead of replaying the same one.

CHAMPS := champions
HTML   := champions-report.html

.PHONY: help evolve evolve-no-variants report clean-champions

help:
	@echo "make evolve              next round of EA (~25min); auto-numbered, unique seed, saves top-5 to $(CHAMPS)/"
	@echo "make evolve-deep         deep EA run (~2-8h depending on gauntlet size): pop=100 gens=100 n=30 k=5 elite=3"
	@echo "make evolve-no-variants  skip baselines/, fight only prior champions (requires at least one round)"
	@echo "make report              champions-report HTML aggregating $(CHAMPS)/"
	@echo "make clean-champions     wipe $(CHAMPS)/ and $(HTML)"

evolve:
	@mkdir -p $(CHAMPS)
	@N=$$(( $$(ls $(CHAMPS)/r*-rank1.json 2>/dev/null | wc -l) + 1 )); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	NUM=$$(($$(echo $$EXTRAS | wc -w | xargs) / 2)); \
	echo "=== round $$N (seed=$$SEED, gauntlet: 7 variants + $$NUM extras) ==="; \
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

evolve-no-variants:
	@mkdir -p $(CHAMPS)
	@N=$$(( $$(ls $(CHAMPS)/r*-rank1.json 2>/dev/null | wc -l) + 1 )); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	NUM=$$(($$(echo $$EXTRAS | wc -w | xargs) / 2)); \
	if [ "$$NUM" = "0" ]; then \
		echo "error: evolve-no-variants needs at least one prior round; run 'make evolve' first"; \
		exit 2; \
	fi; \
	echo "=== round $$N (seed=$$SEED, gauntlet: $$NUM extras, no variants) ==="; \
	cargo run --release -- evolve --seed $$SEED --stop-at-ceiling 3 --save-top 5 --no-variants \
		$$EXTRAS --save $(CHAMPS)/r$$N.json

report:
	cargo run --release -- champions-report --dir $(CHAMPS) --html $(HTML)
	@echo "Open $(HTML)"

clean-champions:
	rm -rf $(CHAMPS) $(HTML)
