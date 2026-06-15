// Custom libp2p WebSocket transport backed by Bun.serve.
//
// Replaces @libp2p/websockets' server-side listener so we bypass the
// `ws` package + node:http combination that misbehaves under Bun
// (the http server's `'upgrade'` event never fires for incoming WS
// upgrade requests). LISTEN-ONLY — the relay never dials outbound WS.
//
// Targets the libp2p 2.x / @libp2p/utils 6.x API: MultiaddrConnection
// is a plain object with sink/source/close/abort/remoteAddr/timeline/log.
//
// Each Bun ServerWebSocket is wrapped as one MultiaddrConnection by
// pushing incoming messages into an it-pushable source and writing
// outgoing chunks via ws.send(). The pinned multiaddr v12 stack means
// the multiaddr we hand to libp2p has the `.tuples()` API gossipsub
// depends on.

import { transportSymbol, TypedEventEmitter } from '@libp2p/interface';
import type {
  ComponentLogger,
  Connection,
  CreateListenerOptions,
  Listener,
  ListenerEvents,
  Logger,
  MultiaddrConnection,
  Transport,
  Upgrader,
} from '@libp2p/interface';
import { multiaddr } from '@multiformats/multiaddr';
import type { Multiaddr } from '@multiformats/multiaddr';
import { WebSockets } from '@multiformats/multiaddr-matcher';
import { pushable } from 'it-pushable';
import type { Pushable } from 'it-pushable';
import type { Uint8ArrayList } from 'uint8arraylist';
import type { ServerWebSocket, Server as BunServer } from 'bun';

interface Components {
  logger: ComponentLogger;
}

interface ConnSocketData {
  push: Pushable<Uint8Array>;
  log: Logger;
}

function bunWsToMaConn(
  ws: ServerWebSocket<ConnSocketData>,
  remoteAddr: Multiaddr,
  log: Logger,
): { maConn: MultiaddrConnection; push: Pushable<Uint8Array> } {
  const push = pushable<Uint8Array>({ objectMode: false });

  const maConn: MultiaddrConnection = {
    log,
    source: push as unknown as AsyncGenerator<Uint8Array | Uint8ArrayList>,
    async sink(source) {
      const SEND_WATERMARK = 1_048_576; // 1 MiB queued in Bun's buffer
      let dropped = 0;
      try {
        for await (const chunk of source) {
          const bytes = chunk instanceof Uint8Array ? chunk : chunk.subarray();
          const buffered = (ws as any).bufferedAmount ?? 0;
          if (buffered > SEND_WATERMARK) {
            dropped++;
            log.error('send-buffer over watermark (%d > %d); dropping %d bytes (drops=%d)', buffered, SEND_WATERMARK, bytes.byteLength, dropped);
            continue;
          }
          const sent = ws.send(bytes, true);
          if (sent === 0) {
            log.error('ws.send returned 0 — socket closed mid-write');
            break;
          }
        }
      } catch (err) {
        log.error('sink loop error: %e', err);
      }
    },
    remoteAddr,
    timeline: { open: Date.now() },
    async close(_options) {
      try {
        push.end();
      } catch {}
      try {
        ws.close(1000, 'normal');
      } catch {}
      maConn.timeline.close = Date.now();
    },
    abort(err) {
      log.error('aborting connection: %e', err);
      try {
        push.end(err);
      } catch {}
      try {
        ws.terminate();
      } catch {}
      maConn.timeline.close = Date.now();
    },
  };

  return { maConn, push };
}

class BunWsListener extends TypedEventEmitter<ListenerEvents> implements Listener {
  private server?: BunServer;
  private listenMa?: Multiaddr;
  private readonly log: Logger;
  private readonly components: Components;
  private readonly upgrader: Upgrader;
  private readonly abortController = new AbortController();

  constructor(components: Components, options: CreateListenerOptions) {
    super();
    this.components = components;
    this.upgrader = options.upgrader;
    if (!this.upgrader) {
      throw new Error('bun-ws-transport: upgrader missing from CreateListenerOptions');
    }
    this.log = components.logger.forComponent('libp2p:bun-ws-transport:listener');
  }

  async listen(ma: Multiaddr): Promise<void> {
    const { host, port } = parseMultiaddr(ma);
    this.listenMa = ma;
    const log = this.log;
    const upgrader = this.upgrader;
    const signal = this.abortController.signal;
    const components = this.components;

    this.server = Bun.serve<ConnSocketData>({
      hostname: host,
      port: Number(port),
      fetch(req, server) {
        const upgraded = server.upgrade(req, { data: {} as ConnSocketData });
        if (upgraded) return;
        return new Response('Only WebSocket connections are supported', {
          status: 426,
          headers: { Upgrade: 'websocket' },
        });
      },
      websocket: {
        open(ws) {
          const remoteAddr = multiaddr(`/ip4/${ws.remoteAddress}/tcp/0/ws`);
          const connLog = components.logger.forComponent(
            `libp2p:bun-ws-transport:connection:${ws.remoteAddress}`,
          );
          const { maConn, push } = bunWsToMaConn(ws, remoteAddr, connLog);
          ws.data.push = push;
          ws.data.log = connLog;
          connLog('open from %s', ws.remoteAddress);

          upgrader
            .upgradeInbound(maConn, { signal: signal as any })
            .then(() => connLog('upgraded'))
            .catch((err: Error) => {
              connLog.error('upgrade failed: %e', err);
              try { maConn.abort(err); } catch {}
            });
        },
        message(ws, data) {
          const push = ws.data.push;
          if (!push) {
            ws.data.log?.error('message but no push attached');
            return;
          }
          let bytes: Uint8Array;
          if (typeof data === 'string') {
            bytes = new TextEncoder().encode(data);
          } else if (data instanceof Uint8Array) {
            bytes = data;
          } else {
            bytes = new Uint8Array(data as ArrayBufferLike);
          }
          push.push(bytes);
        },
        close(ws, code, reason) {
          ws.data.log?.('close code=%d reason="%s"', code, reason);
          try { ws.data.push?.end(); } catch {}
        },
        drain(_ws) {
          // Bun manages its own send buffer; no-op.
        },
      },
    });

    log(`listening on ${host}:${port}`);
    this.safeDispatchEvent('listening');
  }

  async close(): Promise<void> {
    this.abortController.abort();
    if (this.server) {
      this.server.stop(true);
      this.server = undefined;
    }
    this.safeDispatchEvent('close');
  }

  getAddrs(): Multiaddr[] {
    return this.listenMa ? [this.listenMa] : [];
  }

  updateAnnounceAddrs(_addrs: Multiaddr[]): void {}
}

class BunWebSocketTransport implements Transport {
  readonly [transportSymbol] = true as const;
  readonly [Symbol.toStringTag] = '@roam/bun-ws-transport';
  private readonly components: Components;

  constructor(components: Components) {
    this.components = components;
  }

  async dial(_ma: Multiaddr): Promise<Connection> {
    throw new Error('bun-ws-transport: dial not implemented; relay listens only');
  }

  dialFilter(_multiaddrs: Multiaddr[]): Multiaddr[] {
    return [];
  }

  createListener(options: CreateListenerOptions): Listener {
    return new BunWsListener(this.components, options);
  }

  listenFilter(multiaddrs: Multiaddr[]): Multiaddr[] {
    return multiaddrs.filter((ma) => WebSockets.exactMatch(ma));
  }
}

function parseMultiaddr(ma: Multiaddr): { host: string; port: number } {
  if (typeof (ma as any).toOptions === 'function') {
    const { host, port } = (ma as any).toOptions();
    return { host, port: Number(port) };
  }
  const s = ma.toString();
  const m = s.match(/^\/(ip4|ip6|dns4|dns6|dns)\/([^/]+)\/tcp\/(\d+)/);
  if (!m) throw new Error(`bun-ws-transport: cannot parse multiaddr: ${s}`);
  return { host: m[2]!, port: Number(m[3]!) };
}

export function bunWebSocketTransport() {
  return (components: Components): Transport => new BunWebSocketTransport(components);
}
