# tsot — EA workflow shortcuts.
#
# This file is a THIN WRAPPER over the `tsot` CLI. Targets exist
# only as muscle-memory aliases for `cargo run -- <subcommand>`.
# DO NOT add flags, defaults, env vars, log redirection, or `probe-deep`/
# `evolve-deep` style alternates that bake configuration into the
# Makefile. Defaults live in the CLI (clap `default_value_t`). If a
# setting needs to change, change it in the CLI, not here.
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

.PHONY: help matchup-decks evolve evolve-deep evolve-mcts evolve-uct report curate-baselines clean-champions pool prune-champions probe probe-long matchup-mcts assets wasm wasm-serve wasm-dev wasm-dev-serve clean-wasm art art-forever

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
	@echo "  make pool                pool + archetypes dashboard → card-pool.html (chains curve-sample; POOL_NO_CURVE=1 to skip, POOL_NO_ARCHETYPES=1 to skip cluster section)"
	@echo "  make probe [CARD_ID...]  side-by-side compare a card's declared variants (auto-discover if no id)"
	@echo "  make probe-long [...]    same as probe but pop=30 gens=15 n=30 (~3min/variant, σ≈0.025)"
	@echo "  make art                 generate art for one random card without art yet → gen_art/{id}_W_H.png"
	@echo "                           defaults match ~/sd-cpp/ install; throttled to keep machine usable"
	@echo "                           override: SD_BIN / SD_MODEL / SD_THREADS / SD_VRAM_RESERVE / SD_FAST=1"
	@echo "  make art-forever         loop \`make art\` forever (Ctrl-C to stop)"
	@echo ""
	@echo "WASM browser play:"
	@echo "  make wasm                build the wasm bundle + stage dist/ (needs emcc on PATH)"
	@echo "  make wasm-serve          \`make wasm\` then python3 -m http.server on dist/ (PORT=8080 default)"
	@echo "  make wasm-dev            wasm build with -g (names section preserved) for readable stacks"
	@echo "  make wasm-dev-serve      \`make wasm-dev\` then serve dist/ (PORT=8080 default)"
	@echo "  make clean-wasm          rm -rf dist/"

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
	echo "    pop=25 gens=10 n=4 — fast smoke check, noisy fitness"; \
	cargo run --release -- evolve --seed $$SEED \
		--pop 25 --gens 10 --n 4 --stop-at-ceiling 3 \
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

# EA round where the gauntlet OPPONENT plays MCTS. Evolved decks must
# beat strong play to score high. ~16× slower per fitness eval — expect
# ~2-4 hours wall vs 7-8 min for `make evolve`. Override MCTS_ROLLOUTS
# (default 5) to tune; smaller = faster, weaker opponent.
EVOLVE_MCTS_ROLLOUTS ?= 5
EVOLVE_MCTS_MAX_CANDIDATES ?= 10

evolve-mcts:
	@mkdir -p $(CHAMPS)
	@HIGHEST=0; for f in $(CHAMPS)/r*-rank1.json; do [ -f "$$f" ] || continue; base=$$(basename "$$f" -rank1.json); num=$${base#r}; if [ "$$num" -gt "$$HIGHEST" ]; then HIGHEST=$$num; fi; done; \
	N=$$((HIGHEST + 1)); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	NUM=$$(($$(echo $$EXTRAS | wc -w | xargs) / 2)); \
	echo "=== mcts-opponent round $$N (seed=$$SEED, opponent=MCTS rollouts=$(EVOLVE_MCTS_ROLLOUTS) max-cand=$(EVOLVE_MCTS_MAX_CANDIDATES)) ==="; \
	echo "    candidate side plays Heuristic; opponent side plays MCTS — expect ~2-4h wall"; \
	cargo run --release -- evolve --seed $$SEED --stop-at-ceiling 3 --save-top 5 \
		--opponent-ai mcts \
		--opponent-mcts-rollouts $(EVOLVE_MCTS_ROLLOUTS) \
		--opponent-mcts-max-candidates $(EVOLVE_MCTS_MAX_CANDIDATES) \
		$$EXTRAS --save $(CHAMPS)/r$$N.json

# EA against UCT (UCB1 tree-search MCTS). Stronger than `evolve-mcts`
# — UCT beat one-ply MCTS 100% at matched compute on mirror-match
# baseline decks. Override EVOLVE_UCT_ITERATIONS to scale strength
# (50 ≈ one-ply MCTS budget; 100 is meaningfully stronger; 200+ for
# slow / high-quality runs).
EVOLVE_UCT_ITERATIONS ?= 50
EVOLVE_UCT_MAX_CANDIDATES ?= 10

evolve-uct:
	@mkdir -p $(CHAMPS)
	@HIGHEST=0; for f in $(CHAMPS)/r*-rank1.json; do [ -f "$$f" ] || continue; base=$$(basename "$$f" -rank1.json); num=$${base#r}; if [ "$$num" -gt "$$HIGHEST" ]; then HIGHEST=$$num; fi; done; \
	N=$$((HIGHEST + 1)); \
	SEED=$$(printf '0x%x' $$((0xEA00 + N))); \
	EXTRAS=""; \
	for f in $(CHAMPS)/*.json; do \
		[ -f "$$f" ] && EXTRAS="$$EXTRAS --extra $$f"; \
	done; \
	echo "=== uct-opponent round $$N (seed=$$SEED, opponent=UCT iterations=$(EVOLVE_UCT_ITERATIONS) max-cand=$(EVOLVE_UCT_MAX_CANDIDATES)) ==="; \
	echo "    candidate side plays Heuristic; opponent side plays UCT — expect ~2-6h wall"; \
	cargo run --release -- evolve --seed $$SEED --stop-at-ceiling 3 --save-top 5 \
		--opponent-ai uct \
		--opponent-uct-iterations $(EVOLVE_UCT_ITERATIONS) \
		--opponent-mcts-max-candidates $(EVOLVE_UCT_MAX_CANDIDATES) \
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
	@# Pool + archetypes dashboard. Refresh turn-curve data unless
	@# `POOL_NO_CURVE=1`; skip the deck-archetypes section with
	@# `POOL_NO_ARCHETYPES=1`. The Python renderer shells out to lua5.4
	@# internally to evaluate cards/*.lua (handlers are Lua function
	@# bodies) and reads baselines/champions JSON for clustering.
	@if [ "$$POOL_NO_CURVE" != "1" ]; then \
		cargo run --release -- curve-sample $$CURVE_ARGS; \
	fi
	python3 tools/cards-report.py $${POOL_NO_ARCHETYPES:+--no-archetypes}
	@echo "Open card-pool.html"

# Mirror-match MCTS vs Heuristic. MCTS-vs-Heuristic + Heuristic-vs-MCTS
# back-to-back on the SAME random deck (eliminates deck-quality as a
# confounder). 50% = MCTS adds no signal; 55-65% = working; 70%+ = the
# heuristic has obvious gaps. Slow (~3-5 min/game at default rollouts);
# tune via MATCHUP_MCTS_ARGS.
MATCHUP_MCTS_ARGS ?=

matchup-mcts:
	cargo run --release -- matchup-mcts $(MATCHUP_MCTS_ARGS)

# Build the wasm bundle and stage everything the browser needs
# (`tsot_wasm.js`, `tsot_wasm.wasm`, `index.html`) into `dist/`.
# Requires emscripten on PATH — `emcc --version` should work before
# you run this. Install via emsdk:
#   git clone https://github.com/emscripten-core/emsdk; cd emsdk
#   ./emsdk install latest && ./emsdk activate latest
#   source ./emsdk_env.sh
WASM_TARGET    := wasm32-unknown-emscripten
WASM_OUT       := target/$(WASM_TARGET)/release
WASM_OUT_DEV   := target/$(WASM_TARGET)/debug
WASM_DIST      := dist
ELM_SRC        := assets

# H7-Elm Stage 1. Compiles the Elm dev-tool sources under `assets/src/`
# into a single `dist/bundle.js`. `wasm` and `wasm-dev` depend on this
# so the dist/ output always carries the Elm bundle alongside the wasm
# artifacts. The bridge file `assets/js-bridge.js` boots the Elm app
# into `<div id="elm-root">` in play.html; both files get copied into
# dist/ by the wasm targets below.
assets:
	@command -v elm >/dev/null 2>&1 || { \
		echo "error: elm not on PATH."; \
		echo "       Nix users: re-enter the dev shell — flake.nix now provides elmPackages.elm."; \
		echo "         \`exit && nix develop\`"; \
		exit 1; \
	}
	@mkdir -p $(WASM_DIST)
	cd $(ELM_SRC) && elm make src/Main.elm --output=$(CURDIR)/$(WASM_DIST)/bundle.js
	@# H7-Elm Stage 7: also copy the JS bridge + play.html into dist/
	@# so `make assets` is a complete frontend refresh — running
	@# `make wasm-dev-serve` after edits-only-to-Elm-or-JS-bridge no
	@# longer requires a full Rust rebuild for the new code to land.
	cp assets/js-bridge.js $(WASM_DIST)/js-bridge.js
	cp assets/play.html $(WASM_DIST)/index.html

wasm: assets
	@command -v emcc >/dev/null 2>&1 || { \
		echo "error: emcc not on PATH."; \
		echo "       Nix users: re-enter the dev shell — flake.nix now provides emscripten."; \
		echo "         \`exit && nix develop\`"; \
		echo "       Non-Nix: install emscripten via emsdk and \`source ./emsdk_env.sh\`."; \
		exit 1; \
	}
	cargo build -Z build-std=std,panic_abort --target $(WASM_TARGET) --release --bin tsot_wasm
	@mkdir -p $(WASM_DIST)
	cp $(WASM_OUT)/tsot_wasm.js $(WASM_DIST)/tsot_wasm.js
	cp $(WASM_OUT)/tsot_wasm.wasm $(WASM_DIST)/tsot_wasm.wasm
	cp assets/play.html $(WASM_DIST)/index.html
	cp assets/tsot-worker.js $(WASM_DIST)/tsot-worker.js
	cp assets/js-bridge.js $(WASM_DIST)/js-bridge.js
	@# Build-info footer: the page reads window.__TSOT_BUILD__ and
	@# renders it faintly in the bottom-right. Tells the developer
	@# WHICH build they're running without leaving the dev tool.
	@printf 'window.__TSOT_BUILD__=%s;\n' '{"profile":"release","builtAt":"$(shell date -u +%Y-%m-%dT%H:%M:%SZ)","commit":"$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)"}' > $(WASM_DIST)/build-info.js
	@echo ""
	@echo "wasm bundle staged in $(WASM_DIST)/"
	@ls -lah $(WASM_DIST)/

# Serve the dist directory on localhost. Default port 8080; override
# via `WASM_SERVE_PORT=...`. No COOP/COEP headers — those are only
# needed if we later enable SharedArrayBuffer (pthreads / wasm-workers),
# which we don't for v1.
WASM_SERVE_PORT ?= 8080

wasm-serve: wasm
	@command -v caddy >/dev/null 2>&1 || { \
		echo "error: caddy not on PATH. Install via your package manager"; \
		echo "       (Nix shell: add `caddy` to flake.nix devShell)."; \
		exit 1; \
	}
	@echo ""
	@echo "Serving $(WASM_DIST)/ at http://localhost:$(WASM_SERVE_PORT)/  (COOP/COEP enabled, Ctrl-C to stop)"
	@# Caddyfile reads $WASM_SERVE_PORT, defaults to 8080. COOP/COEP
	@# headers put the page in a cross-origin-isolated context —
	@# prerequisite for SharedArrayBuffer / shared wasm memory /
	@# atomic UCT cancellation.
	@exec env WASM_SERVE_PORT=$(WASM_SERVE_PORT) caddy run --config Caddyfile --adapter caddyfile

# Developer build. Same staging as `make wasm` but the wasm carries
# the names section (`-g` to emcc) so the JS exception stacks the
# worker captures resolve `wasm-function[N]` indices to Rust symbol
# names. Also builds in the cargo `dev` profile (no --release), which
# keeps debug-assertions on and produces friendlier panic messages.
# Slower to compile, bigger binary, but a load-bug stack now reads
# `tsot::sim::step::StepEngine::step` instead of `wasm-function[641]`.
#
# `RUSTFLAGS` here REPLACES `.cargo/config.toml`'s `target.<triple>.rustflags`
# per cargo's documented precedence, so we duplicate the production link-
# args list and append `-g`. If we ever change the production link-args,
# keep these two lists in sync.
wasm-dev: assets
	@command -v emcc >/dev/null 2>&1 || { \
		echo "error: emcc not on PATH."; \
		echo "       Nix users: re-enter the dev shell — flake.nix now provides emscripten."; \
		echo "         \`exit && nix develop\`"; \
		echo "       Non-Nix: install emscripten via emsdk and \`source ./emsdk_env.sh\`."; \
		exit 1; \
	}
	EMCC_CFLAGS="-fwasm-exceptions -sSUPPORT_LONGJMP=wasm -pthread" \
	CFLAGS_wasm32_unknown_emscripten="-fwasm-exceptions -sSUPPORT_LONGJMP=wasm -pthread" \
	RUSTFLAGS='-C link-args=-sEXPORTED_FUNCTIONS=_main,_tsot_hello,_tsot_echo,_tsot_free_string,_tsot_start_game,_tsot_apply_action,_tsot_drain_partial_trace,_tsot_list_card_pool,_tsot_list_preset_decks,_tsot_save_game,_tsot_load_game,_tsot_test_panic,_tsot_preview_uct,_tsot_cancel_uct,_tsot_run_auto_game,_UCT_CANCEL_FLAG -C link-args=-sEXPORTED_RUNTIME_METHODS=ccall,cwrap,UTF8ToString,stringToUTF8,lengthBytesUTF8,HEAP8 -C link-args=--js-library=assets/wasm-worker-lib.js -C link-args=-sMODULARIZE=1 -C link-args=-sEXPORT_NAME=createTsotModule -C link-args=-sALLOW_MEMORY_GROWTH=1 -C link-args=-sSTACK_SIZE=16777216 -C link-args=-sINITIAL_MEMORY=67108864 -C link-args=-sSUPPORT_LONGJMP=wasm -C link-args=-sUSE_PTHREADS=1 -C link-args=-sPTHREAD_POOL_SIZE=0 -C link-args=-sSHARED_MEMORY=1 -C target-feature=+atomics,+bulk-memory -C link-arg=-g' cargo build -Z build-std=std,panic_abort --target $(WASM_TARGET) --bin tsot_wasm
	@mkdir -p $(WASM_DIST)
	cp $(WASM_OUT_DEV)/tsot_wasm.js $(WASM_DIST)/tsot_wasm.js
	cp $(WASM_OUT_DEV)/tsot_wasm.wasm $(WASM_DIST)/tsot_wasm.wasm
	cp assets/play.html $(WASM_DIST)/index.html
	cp assets/tsot-worker.js $(WASM_DIST)/tsot-worker.js
	cp assets/js-bridge.js $(WASM_DIST)/js-bridge.js
	@printf 'window.__TSOT_BUILD__=%s;\n' '{"profile":"dev","builtAt":"$(shell date -u +%Y-%m-%dT%H:%M:%SZ)","commit":"$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)"}' > $(WASM_DIST)/build-info.js
	@echo ""
	@echo "wasm-dev bundle (with -g symbol names) staged in $(WASM_DIST)/"
	@ls -lah $(WASM_DIST)/

# Same as `wasm-serve` but builds the dev bundle first.
wasm-dev-serve: wasm-dev
	@command -v caddy >/dev/null 2>&1 || { \
		echo "error: caddy not on PATH. Install via your package manager"; \
		echo "       (Nix shell: add `caddy` to flake.nix devShell)."; \
		exit 1; \
	}
	@echo ""
	@echo "Serving $(WASM_DIST)/ (dev) at http://localhost:$(WASM_SERVE_PORT)/  (COOP/COEP enabled, Ctrl-C to stop)"
	@exec env WASM_SERVE_PORT=$(WASM_SERVE_PORT) caddy run --config Caddyfile --adapter caddyfile

clean-wasm:
	rm -rf $(WASM_DIST)

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

# `make art` — one random card without art gets a generated piece, dropped
# into gen_art/{id}_W_H.png. Picks a card, reads its .lua metadata via the
# same lua5.4 driver pattern as cards-report.py, builds a prompt, shells
# out to stable-diffusion.cpp's `sd` binary, then carves slot-grid holes
# via `magick` if the card declares any. Re-run to fill more cards.
#
# Defaults below prioritize keeping the machine usable during generation
# (~3-5 min per card on 8 GB M1) over generation speed. Any of these env
# vars can be overridden on the command line: `SD_BLEED=15 make art`.
art:
	@mkdir -p gen_art
	@# SD_BLEED — % to crop off each side after generating larger; 0 = generate at target size, SD self-draws frames; 15 = generate 30% larger and crop SD's self-frames off.
	@# SD_THREADS — number of CPU cores sd-cli may use during generation (lower = OS keeps more cores).
	@# SD_VRAM_RESERVE — GiB to keep free for the OS / display compositor (higher = less GPU contention, less sd-cli budget).
	@# SD_TINT — strength of the post-process color tint, 0–100 (0 disables, 25 keeps SD's natural palette through, 100 monochrome).
	@# SD_TEXT — whether to draw the themed text overlay on top of the art (1 yes, 0 raw art only).
	@SD_BLEED=$${SD_BLEED:-10} \
	 SD_THREADS=$${SD_THREADS:-1} \
	 SD_VRAM_RESERVE=$${SD_VRAM_RESERVE:-4.0} \
	 SD_TINT=$${SD_TINT:-25} \
	 SD_TEXT=$${SD_TEXT:-1} \
	 python3 tools/gen_art.py

# Loop `make art` forever. Ctrl-C to stop.
art-forever:
	@while true; do $(MAKE) art; done
