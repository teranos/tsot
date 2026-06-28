WASM_TARGET := wasm32-unknown-unknown
WASM_OUT    := target/$(WASM_TARGET)/release
WASM_DIST   := dist

.PHONY: wasm clean

wasm:
	@command -v wasm-bindgen >/dev/null 2>&1 || { echo "error: wasm-bindgen not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	@command -v bun >/dev/null 2>&1 || { echo "error: bun not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	@# Build-info env vars read by `option_env!` in rave::build_info —
	@# the wasm carries its own commit + time, so any screenshot pins the build.
	RAVE_BUILD_COMMIT="$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)$(shell [ -n "$$(git status --porcelain 2>/dev/null)" ] && echo -dirty)" \
	RAVE_BUILD_TIME="$(shell date -u +%Y-%m-%dT%H:%M:%SZ)" \
	cargo build --target $(WASM_TARGET) --release --lib
	wasm-bindgen $(WASM_OUT)/rave.wasm --target web --out-dir $(WASM_DIST) --no-typescript
	@# Bundle the TS modules under web/ into a single dist/main.js.
	@# Bun is build-time only — the browser never sees TypeScript.
	cd web && bun build src/main.ts --outdir=../$(WASM_DIST) --target=browser --minify --entry-naming "main.js"
	@# Content-hash rave.js + rave_bg.wasm + main.js. The three are
	@# only valid as a matched set, so the URLs become content-addressed
	@# and a cached client can never pair a fresh JS with a stale wasm.
	cd $(WASM_DIST) && \
	  wh=$$(sha256sum rave_bg.wasm | cut -c1-12) && \
	  mv rave_bg.wasm rave_bg.$$wh.wasm && \
	  sed -i "s/rave_bg\.wasm/rave_bg.$$wh.wasm/g" rave.js && \
	  jh=$$(sha256sum rave.js | cut -c1-12) && \
	  mv rave.js rave.$$jh.js && \
	  sed -i "s|\./rave\.js|./rave.$$jh.js|g" main.js && \
	  sed -i "s|WASM_URL_PLACEHOLDER|./rave_bg.$$wh.wasm|g" main.js && \
	  mh=$$(sha256sum main.js | cut -c1-12) && \
	  mv main.js main.$$mh.js && \
	  sed -e "s|\./main\.js|./main.$$mh.js|g" \
	      ../web/index.html > index.html
	@echo ""
	@echo "staged $(WASM_DIST)/"
	@ls -lah $(WASM_DIST)/

clean:
	rm -rf target dist pkg
