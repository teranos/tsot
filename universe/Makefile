# Match the monorepo convention: every subproject's one command is
#   nix develop -c make wasm-serve
# Tool underneath (trunk here, caddy in roam) is an implementation
# detail. The user-facing shape is uniform.

.PHONY: wasm-serve clean

wasm-serve:
	@command -v trunk >/dev/null 2>&1 || { echo "error: trunk not on PATH. Run via 'nix develop -c make wasm-serve'."; exit 1; }
	trunk serve

clean:
	rm -rf target dist pkg
