WASM_TARGET := wasm32-unknown-unknown
WASM_OUT    := target/$(WASM_TARGET)/release
WASM_DIST   := dist

.PHONY: wasm clean

wasm:
	@command -v wasm-bindgen >/dev/null 2>&1 || { echo "error: wasm-bindgen not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	@# Build-info env vars read by `option_env!` in universe::build_info —
	@# the wasm carries its own commit + time, so any screenshot pins the build.
	UNIVERSE_BUILD_COMMIT="$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)$(shell [ -n "$$(git status --porcelain 2>/dev/null)" ] && echo -dirty)" \
	UNIVERSE_BUILD_TIME="$(shell date -u +%Y-%m-%dT%H:%M:%SZ)" \
	cargo build --target $(WASM_TARGET) --release --lib
	wasm-bindgen $(WASM_OUT)/universe.wasm --target web --out-dir $(WASM_DIST) --no-typescript
	cp index.html $(WASM_DIST)/index.html
	@echo ""
	@echo "staged $(WASM_DIST)/"
	@ls -lah $(WASM_DIST)/

clean:
	rm -rf target dist pkg
