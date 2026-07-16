.PHONY: music web build clean cdda coverage

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
# features + ranked candidates. HANDOVER checklist #2 — "add buildings"
# is measured, not guessed.
coverage:
	@cargo +nightly run --manifest-path ../crates/cdda/Cargo.toml --bin cdda-coverage --release
