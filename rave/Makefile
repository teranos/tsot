WASM_TARGET := wasm32-unknown-unknown
WASM_OUT    := target/$(WASM_TARGET)/release
WASM_DIST   := dist

.PHONY: wasm clean

wasm:
	@command -v wasm-bindgen >/dev/null 2>&1 || { echo "error: wasm-bindgen not on PATH. Run via 'nix develop -c make wasm'."; exit 1; }
	@# Build-info env vars read by `option_env!` in rave::build_info —
	@# the wasm carries its own commit + time, so any screenshot pins the build.
	RAVE_BUILD_COMMIT="$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)$(shell [ -n "$$(git status --porcelain 2>/dev/null)" ] && echo -dirty)" \
	RAVE_BUILD_TIME="$(shell date -u +%Y-%m-%dT%H:%M:%SZ)" \
	cargo build --target $(WASM_TARGET) --release --lib
	wasm-bindgen $(WASM_OUT)/rave.wasm --target web --out-dir $(WASM_DIST) --no-typescript
	@# Content-hash rave.js + rave_bg.wasm into their filenames. The two
	@# are only valid as a matched pair (rave.js calls wasm exports by a
	@# per-build index), but with stable names + a cache TTL a returning
	@# client can pair a fresh rave.js with a stale rave_bg.wasm (or the
	@# reverse) across deploys — which surfaces at runtime as
	@# "__wasm_bindgen_func_elem_N is not a function" and a black canvas.
	@# Hashed names make every URL content-addressed: index.html (short
	@# TTL) always points at one specific rave.<h>.js, which imports one
	@# specific rave_bg.<h>.wasm. A mismatched pair becomes unrepresentable.
	cd $(WASM_DIST) && \
	  wh=$$(sha256sum rave_bg.wasm | cut -c1-12) && \
	  mv rave_bg.wasm rave_bg.$$wh.wasm && \
	  sed -i "s/rave_bg\.wasm/rave_bg.$$wh.wasm/g" rave.js && \
	  jh=$$(sha256sum rave.js | cut -c1-12) && \
	  mv rave.js rave.$$jh.js && \
	  sed -e "s|\./rave\.js|./rave.$$jh.js|g" \
	      -e "s|WASM_URL_PLACEHOLDER|./rave_bg.$$wh.wasm|g" \
	      ../index.html > index.html
	@echo ""
	@echo "staged $(WASM_DIST)/"
	@ls -lah $(WASM_DIST)/

clean:
	rm -rf target dist pkg
