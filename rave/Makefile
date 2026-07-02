WASM_TARGET := wasm32-unknown-unknown
WASM_OUT    := target/$(WASM_TARGET)/release
WASM_DIST   := dist

# Build-info env vars read by `option_env!` in rave::build_info —
# the wasm carries its own commit + time, so any screenshot pins
# the build.
BUILD_ENV := \
	RAVE_BUILD_COMMIT="$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)$(shell [ -n "$$(git status --porcelain 2>/dev/null)" ] && echo -dirty)" \
	RAVE_BUILD_TIME="$(shell date -u +%Y-%m-%dT%H:%M:%SZ)"

.PHONY: wasm wasm-webgpu wasm-webgl2 wasm-shim clean

# `make wasm` builds both bundles + the top-level shim. Bevy 0.19
# gates its WebGL2 shader fallback defines at compile time on
# `not(feature = "webgpu")` (bevy_render `pipeline_cache.rs:237`),
# so the two backends have to be mutually exclusive per build. The
# shim's `dist/index.html` picks between them at load time based
# on `navigator.gpu`.
wasm: wasm-webgpu wasm-webgl2 wasm-shim
	@echo ""
	@echo "staged $(WASM_DIST)/"
	@ls -R $(WASM_DIST)/

wasm-webgpu:
	@command -v wasm-bindgen >/dev/null 2>&1 || { echo "error: wasm-bindgen not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	@command -v bun >/dev/null 2>&1 || { echo "error: bun not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	rm -rf $(WASM_DIST)/webgpu
	mkdir -p $(WASM_DIST)/webgpu
	$(BUILD_ENV) cargo build --target $(WASM_TARGET) --release --lib --no-default-features --features backend-webgpu
	wasm-bindgen $(WASM_OUT)/rave.wasm --target web --out-dir $(WASM_DIST)/webgpu --no-typescript
	cd web && bun build src/main.ts --outdir=../$(WASM_DIST)/webgpu --target=browser --minify --entry-naming "main.js"
	bash web/hash-and-stage.sh $(WASM_DIST)/webgpu web/index.html

wasm-webgl2:
	@command -v wasm-bindgen >/dev/null 2>&1 || { echo "error: wasm-bindgen not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	@command -v bun >/dev/null 2>&1 || { echo "error: bun not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	rm -rf $(WASM_DIST)/webgl2
	mkdir -p $(WASM_DIST)/webgl2
	$(BUILD_ENV) cargo build --target $(WASM_TARGET) --release --lib --no-default-features --features backend-webgl2
	wasm-bindgen $(WASM_OUT)/rave.wasm --target web --out-dir $(WASM_DIST)/webgl2 --no-typescript
	cd web && bun build src/main.ts --outdir=../$(WASM_DIST)/webgl2 --target=browser --minify --entry-naming "main.js"
	bash web/hash-and-stage.sh $(WASM_DIST)/webgl2 web/index.html

wasm-shim:
	mkdir -p $(WASM_DIST)
	cp web/index-shim.html $(WASM_DIST)/index.html

clean:
	rm -rf target dist pkg
