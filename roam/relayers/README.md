# relayers

Rust libp2p relayer at `roam/relayers/` replacing the TypeScript
Bun relay that used to live in this directory. Live at
`relay.sbvh.nl`.

## What's done

- Rust binary serves the same WSS endpoint, same PeerId
  (`12D3KooWMSVxS7nt…`), same gossipsub topic, same CloudWatch
  metric set + dimensions (existing alarms unchanged).
- WebSocket upgrade through CloudFront verified: `HTTP 101` for
  HTTP/1.1 + WS upgrade headers (browser path).
- Tag `v0.3.0` cuts the line.

## What didn't port

- The TS `bun-ws-transport.ts:135-139` returned `426 Upgrade
  Required` for non-WS GETs. The Rust `libp2p-websocket` (soketto)
  drops the connection, so a plain browser navigation to
  `https://relay.sbvh.nl/` returns CloudFront 502. Functional
  paths unaffected. A status-page-fronting custom Transport is a
  separate slice if/when the discoverability matters.
- `roam/relay/relay.ts` + `roam/relay/bun-ws-transport.ts` still on
  disk. Deletion after a soak window of running on Rust without
  regression.

## Deploy mechanics

- Cross-compile: `cargo zigbuild --target x86_64-unknown-linux-musl --release` (15M static binary, no glibc dependency)
- Ship: `rsync` to `/home/ubuntu/relayers` on the Lightsail box
- Wire: drop-in `/etc/systemd/system/roam-relay.service.d/relayers.conf`
  swaps `ExecStart`; original unit untouched
- Logging: drop-in `…/logging.conf` sets
  `RUST_LOG=…libp2p_websocket=debug,libp2p_swarm=debug,…` so close
  codes are always in the journal across reboots
