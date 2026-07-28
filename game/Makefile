.PHONY: music web build clean cdda coverage test red

# The gate, byte-for-byte what .github/workflows/game-tests.yml runs.
# Whole suite — lib, every target under tests/, doctests — then clippy
# over every target, then the wasm32 build. Deliberately not narrowed
# to --lib: a test that doesn't run is a test that lies, and that
# narrowing is what let a red tests/ directory pass CI.
#
# The wasm build is here because this project SHIPS wasm. Everything
# above it compiles for the host, so a `cfg(target_arch = "wasm32")`
# block is invisible to all of it — which is exactly how a call to two
# functions that did not exist reached master's CI and broke both the
# deploy and the seer pipeline at once. Native-only green was never the
# claim worth making.
#
# Green here means nothing is broken and every skip is named. It does
# NOT mean every test ran; see `make red`.
test:
	@cargo +nightly test
	@cargo +nightly clippy --all-targets -- -D warnings
	@cargo +nightly build --target wasm32-unknown-unknown --lib

# The declared reds: tests carrying `#[ignore = "RED: ..."]`, which
# capture intent ahead of the code that satisfies it. These are MEANT
# to fail — that's the red bar, and this is how you look at it. A
# non-zero exit from this target is the correct outcome while the work
# is outstanding.
red:
	@cargo +nightly test --no-fail-fast -- --ignored

# Fetch the CDDA map corpus (a pinned dependency, never vendored) into
# crates/cdda/.cdda-src/ so bare `cargo build`/`cargo test` can compile
# it in. Nix users don't need this — the flake sets CDDA_SRC. Pinned to
# the crate's RELEASE file.
cdda:
	@../crates/cdda/tools/fetch.sh

# Generate the placeholder track and encode as OGG Vorbis. Output
# lands in web/assets/ so build.ts picks it up when it copies assets/
# into dist/. Single source of truth: tools/gen-rave-ogg.py.
music:
	@command -v python3 >/dev/null 2>&1 || { echo "error: python3 not on PATH."; exit 1; }
	@command -v ffmpeg  >/dev/null 2>&1 || { echo "error: ffmpeg not on PATH."; exit 1; }
	mkdir -p web/assets
	python3 tools/gen-rave-ogg.py /tmp/rave-source.wav
	ffmpeg -y -loglevel error -i /tmp/rave-source.wav \
		-af "aecho=0.8:0.9:40|100|200|400:0.5|0.3|0.15|0.08,aecho=0.7:0.4:600:0.35" \
		-c:a libvorbis -q:a 4 web/assets/rave.ogg
	rm -f /tmp/rave-source.wav

# Build the shippable web bundle for game.sbvh.nl. Runs music first so
# the freshly-encoded OGG is present in web/assets/ before build.ts
# copies it into dist/.
build: music
	@cd web && bun install --silent && bun run build.ts

clean:
	@rm -rf web/dist web/node_modules web/assets/rave.ogg

# Walk the pinned CDDA mapgen tree, resolve every om_terrain through
# our importer, print resolved/empty/skipped/failed + top unhandled
# features + ranked candidates. "Add buildings" is measured, not
# guessed — see the coverage report output for the top blockers.
coverage:
	@cargo +nightly run --manifest-path ../crates/cdda/Cargo.toml --bin cdda-coverage --release
