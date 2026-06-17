# relay/ — upstream tracking

## `bun-ws-transport.ts` — why this exists

`@libp2p/websockets` ships a server-side listener built on the `ws`
package wired into `node:http`. Under Bun, the listener never
receives an `'upgrade'` event for incoming WebSocket upgrade requests,
so the relay's libp2p node never gets the inbound connection callback
and browser peers cannot connect.

Reproduced (2026-06): a stock relay using `@libp2p/websockets`'
listener accepts the TCP connection, the browser sends the HTTP
upgrade, Bun's `node:http` shim does not surface it to the `ws`
package's upgrade-handler registration, and the libp2p side observes
no inbound connection. Same code under Node.js works.

`bun-ws-transport.ts` is a listen-only `Transport` implementation
that goes straight to `Bun.serve()`'s native WebSocket support,
skipping the `ws` + `node:http` layer entirely. It implements the
`Transport` / `Listener` interface against libp2p 2.x and exports
`bunWebSocketTransport()` for `transports: [...]` in `createLibp2p`.

## What we're watching upstream

- **Bun** — `node:http` parity work. The blocking gap is the
  `'upgrade'` event firing on incoming WS upgrade requests. When this
  lands, the stock `@libp2p/websockets` listener should work under
  Bun and this file can be deleted.
  Repo: https://github.com/oven-sh/bun

- **`@libp2p/websockets`** — Bun-specific server adapter. If the
  package gains a Bun-native listener path (e.g. detecting `Bun` at
  runtime and using `Bun.serve` directly), this file is also
  obsolete.
  Repo: https://github.com/libp2p/js-libp2p / monorepo
  package path `packages/transport-websockets/`.

## Migration trigger

Delete `bun-ws-transport.ts` and switch the relay's `transports: [...]`
back to `webSockets()` from `@libp2p/websockets` when:

1. A pinned Bun version successfully runs the upstream
   `@libp2p/websockets` listener end-to-end with browser peers
   connecting through the relay, AND
2. `make test` passes against that Bun version.

Don't delete this file pre-emptively on a Bun version bump — verify
the upstream actually works first. The whole point of having the
custom transport is that we don't trust the integration silently.

## Out of scope for this file

- Outbound dial (`dial()` throws `not implemented`). The relay only
  listens; browsers dial it, never the other way around.
- TLS termination. Plain WS on localhost / behind CloudFront's TLS
  terminator in production. If we ever expose a relay without a TLS
  terminator in front, we'd need WSS support here.

## Pinned context

Versions at time of writing (see `package.json` for current):

- `libp2p` 2.x line
- `@libp2p/interface` 2.x line
- `@multiformats/multiaddr` 12.x line
- Bun runtime — operator's local + the Lightsail box's installed Bun

Any upstream version bump that touches these is a re-verify trigger
for the transport. The transport targets specific interface shapes
(`MultiaddrConnection` with `sink/source/close/abort/remoteAddr/timeline/log`)
which have changed across libp2p major versions before.
