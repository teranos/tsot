.PHONY: web build clean

# Run the seer viewer dev server. Defaults to proxying real production
# history from seer.sbvh.nl; set SEER_DEV_DATA=<path> to serve a local
# seer-host output directory instead.
web:
	@cd viewer && bun install --silent && bun run dev

# Produce the shippable dist/ that the deploy workflow uploads to S3.
build:
	@cd viewer && bun install --silent && bun run build

clean:
	@rm -rf viewer/dist viewer/node_modules
