WASM_TARGET    := wasm32-unknown-unknown
WASM_OUT       := target/$(WASM_TARGET)/release
WASM_DIST      := dist
ROAM_SERVE_PORT ?= 8083

.PHONY: help run relay relay-watch dev test js elm assets wasm wasm-serve clean clean-wasm

help:
	@echo ""
	@echo "Native:"
	@echo "  make run             cargo run --bin roam (sanity)"
	@echo ""
	@echo "WASM:"
	@echo "  make wasm            build + stage dist/ (needs wasm-bindgen-cli)"
	@echo "  make wasm-serve      \`make wasm\` then caddy on dist/ (PORT=$(ROAM_SERVE_PORT) default)"
	@echo "  make clean-wasm      rm -rf dist/"
	@echo ""

run:
	cargo run --bin roam

test:
	@command -v bun >/dev/null 2>&1 || { echo "error: bun not on PATH."; exit 1; }
	@if lsof -tiTCP:9001 -sTCP:LISTEN >/dev/null 2>&1; then \
		kill $$(lsof -tiTCP:9001 -sTCP:LISTEN) 2>/dev/null || true; sleep 0.3; \
	fi
	bun test test/

# Local dev relay (libp2p + circuit-relay v2 + gossipsub). Joins
# the roam-positions topic so connected browsers can find each other.
# Writes its multiaddr to dist/relay-multiaddr.txt; the browser
# fetches that and prepends it to its bootstrap list. Persistent
# peer key in relay/.peer-key keeps the multiaddr stable across runs.
relay:
	@command -v bun >/dev/null 2>&1 || { \
		echo "error: bun not on PATH. Enter nix dev shell or install Bun."; \
		exit 1; \
	}
	@mkdir -p $(WASM_DIST)
	bun run relay

relay-watch:
	@command -v bun >/dev/null 2>&1 || { echo "error: bun not on PATH."; exit 1; }
	@mkdir -p $(WASM_DIST)
	@while true; do \
		bun run relay/relay.ts; \
		ec=$$?; \
		echo "[relay-watch] exited code=$$ec, restarting in 1s (Ctrl-C to stop)"; \
		sleep 1; \
	done

dev:
	@command -v bun   >/dev/null 2>&1 || { echo "error: bun not on PATH.";   exit 1; }
	@command -v caddy >/dev/null 2>&1 || { echo "error: caddy not on PATH."; exit 1; }
	@$(MAKE) -s wasm
	@trap 'echo; echo "[dev] shutting down"; kill 0' INT TERM EXIT; \
	( while true; do bun run relay/relay.ts; echo "[relay-watch] restart in 1s"; sleep 1; done ) & \
	( env ROAM_SERVE_PORT=$(ROAM_SERVE_PORT) caddy run --config Caddyfile --adapter caddyfile ) & \
	wait

# Same relay, run under Node instead of Bun. Used to isolate Bun-vs-Node
# behavior differences in the libp2p WebSocket server-side path. Bundle
# relay.ts to ESM JS via bun, then exec node.
relay-node:
	@command -v node >/dev/null 2>&1 || { echo "error: node not on PATH."; exit 1; }
	@command -v bun  >/dev/null 2>&1 || { echo "error: bun not on PATH (used to bundle TS)."; exit 1; }
	@mkdir -p $(WASM_DIST)
	bun build relay/relay.ts --outfile $(WASM_DIST)/relay.js --target=node --format=esm --external='node:*'
	DEBUG='libp2p:*' node $(WASM_DIST)/relay.js

# Bundle assets/src/js-bridge.js (ES module with libp2p imports) into
# a single dist/js-bridge.js. Browsers can't resolve `import 'libp2p'`
# natively — Bun walks the import graph at build time and emits one
# self-contained file.
js:
	@command -v bun >/dev/null 2>&1 || { \
		echo "error: bun not on PATH. Enter nix dev shell or install Bun."; \
		exit 1; \
	}
	@mkdir -p $(WASM_DIST)
	bun run build

elm:
	@command -v elm >/dev/null 2>&1 || { \
		echo "error: elm not on PATH. Enter nix dev shell."; \
		exit 1; \
	}
	@mkdir -p $(WASM_DIST)
	cd assets && elm make src/Roam/Main.elm --output=$(CURDIR)/$(WASM_DIST)/elm.js

# Stage frontend-only files (HTML + bundled JS + Elm) without rebuilding wasm.
# Use this after editing assets/*.html, assets/src/*.{js,elm}.
assets: js elm
	cp assets/play.html $(WASM_DIST)/index.html

wasm: assets
	@command -v wasm-bindgen >/dev/null 2>&1 || { \
		echo "error: wasm-bindgen not on PATH. Enter nix dev shell: \`nix develop\`"; \
		exit 1; \
	}
	cargo build --target $(WASM_TARGET) --release --lib
	wasm-bindgen $(WASM_OUT)/roam.wasm --target web --out-dir $(WASM_DIST) --no-typescript
	@printf 'window.__ROAM_BUILD__=%s;\n' '{"profile":"release","builtAt":"$(shell date -u +%Y-%m-%dT%H:%M:%SZ)","commit":"$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)$(shell [ -n "$$(git status --porcelain 2>/dev/null)" ] && echo -dirty)"}' > $(WASM_DIST)/build-info.js
	@echo ""
	@echo "staged $(WASM_DIST)/"
	@ls -lah $(WASM_DIST)/

wasm-serve: wasm
	@command -v caddy >/dev/null 2>&1 || { \
		echo "error: caddy not on PATH"; \
		exit 1; \
	}
	@echo "serving $(WASM_DIST)/ at http://localhost:$(ROAM_SERVE_PORT)/  (Ctrl-C to stop)"
	@exec env ROAM_SERVE_PORT=$(ROAM_SERVE_PORT) caddy run --config Caddyfile --adapter caddyfile

clean-wasm:
	rm -rf $(WASM_DIST)

clean: clean-wasm
	cargo clean
