WASM_TARGET := wasm32-unknown-unknown
WASM_OUT    := target/$(WASM_TARGET)/release
WASM_DIST   := dist

# Build-info env vars read by `option_env!` in rave::build_info —
# the wasm carries its own commit + time, so any screenshot pins the build.
BUILD_ENV := \
	RAVE_BUILD_COMMIT="$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)$(shell [ -n "$$(git status --porcelain 2>/dev/null)" ] && echo -dirty)" \
	RAVE_BUILD_TIME="$(shell date -u +%Y-%m-%dT%H:%M:%SZ)"

.PHONY: wasm wasm-webgpu wasm-webgl2 wasm-index wasm-audio clean

# `make wasm` builds both backend bundles + the single canonical
# index.html that bootstraps whichever the client can run. All
# artifacts live flat in dist/ — no per-backend subdirectory,
# so URLs stay at root. Bevy 0.19 gates its WebGL2 shader fallback
# defines at compile time on `not(feature = "webgpu")` (bevy_render
# pipeline_cache.rs:237), so the two backends have to be mutually
# exclusive per build. Runtime pick lives in the bootstrap.
wasm: wasm-webgpu wasm-webgl2 wasm-index wasm-audio
	@echo ""
	@echo "staged $(WASM_DIST)/"
	@ls -la $(WASM_DIST)/

wasm-audio:
	@command -v python3 >/dev/null 2>&1 || { echo "error: python3 not on PATH."; exit 1; }
	@command -v ffmpeg  >/dev/null 2>&1 || { echo "error: ffmpeg not on PATH."; exit 1; }
	mkdir -p $(WASM_DIST)/assets/music
	python3 tools/gen-rave-ogg.py /tmp/rave-source.wav
	ffmpeg -y -loglevel error -i /tmp/rave-source.wav \
		-af "aecho=0.8:0.9:40|100|200|400:0.5|0.3|0.15|0.08,aecho=0.7:0.4:600:0.35" \
		-c:a libvorbis -q:a 4 $(WASM_DIST)/assets/music/rave.ogg
	rm -f /tmp/rave-source.wav

wasm-webgpu:
	@command -v wasm-bindgen >/dev/null 2>&1 || { echo "error: wasm-bindgen not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	@command -v bun >/dev/null 2>&1 || { echo "error: bun not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	mkdir -p $(WASM_DIST)
	rm -f $(WASM_DIST)/rave-webgpu*.js $(WASM_DIST)/rave-webgpu*.wasm $(WASM_DIST)/main-webgpu*.js
	$(BUILD_ENV) cargo build --target $(WASM_TARGET) --release --lib --no-default-features --features backend-webgpu
	wasm-bindgen $(WASM_OUT)/rave.wasm --target web --out-dir $(WASM_DIST) --no-typescript
	cd web && bun build src/main.ts --outdir=../$(WASM_DIST) --target=browser --minify --entry-naming "main.js"
	$(BUILD_ENV) bash web/hash-and-stage.sh $(WASM_DIST) webgpu

wasm-webgl2:
	@command -v wasm-bindgen >/dev/null 2>&1 || { echo "error: wasm-bindgen not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	@command -v bun >/dev/null 2>&1 || { echo "error: bun not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	mkdir -p $(WASM_DIST)
	rm -f $(WASM_DIST)/rave-webgl2*.js $(WASM_DIST)/rave-webgl2*.wasm $(WASM_DIST)/main-webgl2*.js
	$(BUILD_ENV) cargo build --target $(WASM_TARGET) --release --lib --no-default-features --features backend-webgl2
	wasm-bindgen $(WASM_OUT)/rave.wasm --target web --out-dir $(WASM_DIST) --no-typescript
	cd web && bun build src/main.ts --outdir=../$(WASM_DIST) --target=browser --minify --entry-naming "main.js"
	$(BUILD_ENV) bash web/hash-and-stage.sh $(WASM_DIST) webgl2

wasm-index:
	@# The two backend builds each produced a `main-<backend>.<hash>.js`.
	@# Sed both filenames into the shared bootstrap template so the
	@# canonical dist/index.html knows which script to load per client.
	@webgpu_main=$$(ls -1 $(WASM_DIST)/main-webgpu.*.js | head -1 | xargs basename); \
	 webgl2_main=$$(ls -1 $(WASM_DIST)/main-webgl2.*.js | head -1 | xargs basename); \
	 sed -e "s|MAIN_WEBGPU_URL_PLACEHOLDER|./$$webgpu_main|g" \
	     -e "s|MAIN_WEBGL2_URL_PLACEHOLDER|./$$webgl2_main|g" \
	     web/index.html > $(WASM_DIST)/index.html

clean:
	rm -rf target dist pkg
