import { createLibp2p } from 'libp2p';
import { bunWebSocketTransport } from './bun-ws-transport.ts';
import { noise } from '@chainsafe/libp2p-noise';
import { yamux } from '@chainsafe/libp2p-yamux';
import { identify } from '@libp2p/identify';
import { circuitRelayServer } from '@libp2p/circuit-relay-v2';
import { gossipsub } from '@chainsafe/libp2p-gossipsub';
import {
  generateKeyPair,
  privateKeyFromProtobuf,
  privateKeyToProtobuf,
} from '@libp2p/crypto/keys';
import type { PrivateKey } from '@libp2p/interface';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';

const KEY_FILE = './relay/.peer-key';
const MULTIADDR_FILE = './dist/relay-multiaddr.txt';
const TOPIC = 'roam-positions/v1';
const LISTEN_PORT = 9001;

async function loadOrCreateKey(): Promise<PrivateKey> {
  if (existsSync(KEY_FILE)) {
    const bytes = await readFile(KEY_FILE);
    return privateKeyFromProtobuf(bytes);
  }
  const key = await generateKeyPair('Ed25519');
  await mkdir('./relay', { recursive: true });
  await writeFile(KEY_FILE, privateKeyToProtobuf(key));
  console.log('[relay] generated new keypair');
  return key;
}

const privateKey = await loadOrCreateKey();

const node = await createLibp2p({
  privateKey,
  addresses: {
    // Listen on the loopback IP (where the socket actually binds) but
    // ANNOUNCE the DNS name. Browsers served from http://localhost dial
    // /dns4/localhost cleanly; some browsers treat 127.0.0.1 and
    // localhost as different origins for mixed-content / CORP purposes.
    listen: [`/ip4/127.0.0.1/tcp/${LISTEN_PORT}/ws`],
    announce: [`/dns4/localhost/tcp/${LISTEN_PORT}/ws`],
  },
  transports: [bunWebSocketTransport()],
  connectionEncrypters: [noise()],
  streamMuxers: [yamux()],
  services: {
    identify: identify(),
    relay: circuitRelayServer({
      reservations: { maxReservations: 128 },
    }),
    pubsub: gossipsub({ allowPublishToZeroTopicPeers: true }),
  },
});

// Subscribe to the topic so peer-exchange propagates subscribers to
// connected browsers — the entire reason this relay exists.
const pubsub = node.services.pubsub;
pubsub.subscribe(TOPIC);

const multiaddrs = node.getMultiaddrs().map((a) => a.toString());
await mkdir('./dist', { recursive: true });
await writeFile(MULTIADDR_FILE, multiaddrs.join('\n') + '\n');

console.log(`[relay] peerId:  ${node.peerId.toString()}`);
console.log(`[relay] listening on:`);
for (const a of multiaddrs) console.log(`  ${a}`);
console.log(`[relay] subscribed to ${TOPIC}`);
console.log(`[relay] wrote multiaddrs → ${MULTIADDR_FILE}`);

node.addEventListener('peer:connect', (e) =>
  console.log(`[relay] peer:connect ${e.detail.toString().slice(-12)}`),
);
node.addEventListener('peer:disconnect', (e) =>
  console.log(`[relay] peer:disconnect ${e.detail.toString().slice(-12)}`),
);
node.addEventListener('connection:open', (e) =>
  console.log(`[relay] connection:open ${e.detail.remotePeer.toString().slice(-12)} via ${e.detail.remoteAddr.toString()}`),
);
node.addEventListener('connection:close', (e) =>
  console.log(`[relay] connection:close ${e.detail.remotePeer.toString().slice(-12)}`),
);

// circuit-relay v2 server events — visibility into browser reservation
// requests (browsers ask the relay to forward their incoming WebRTC
// signaling). Pulled from the services map; cast because TS types on
// the service map are loose.
const relaySvc: any = (node.services as any).relay;
if (relaySvc?.addEventListener) {
  relaySvc.addEventListener('relay:reservation', (e: any) =>
    console.log(`[relay] relay:reservation from ${e.detail?.peerId?.toString()?.slice(-12)} expiry=${e.detail?.expiry}`),
  );
  relaySvc.addEventListener('relay:advert:success', () => console.log('[relay] relay:advert:success'));
  relaySvc.addEventListener('relay:advert:error', (e: any) => console.error('[relay] relay:advert:error', e.detail));
}

// Errors-as-first-class. Surface, structure, exit on fatal — don't
// limp on with the developer thinking the relay is healthy.
process.on('uncaughtException', (err) => {
  console.error('[relay] FATAL uncaughtException:', err);
  console.error(err.stack);
  process.exit(2);
});
process.on('unhandledRejection', (reason: any) => {
  console.error('[relay] FATAL unhandledRejection:', reason);
  if (reason?.stack) console.error(reason.stack);
  process.exit(3);
});

pubsub.addEventListener('subscription-change', (e) => {
  const subs = e.detail.subscriptions
    .map((s) => `${s.subscribe ? '+' : '-'}${s.topic}`)
    .join(' ');
  console.log(
    `[relay] subscription-change ${e.detail.peerId.toString().slice(-12)} ${subs}`,
  );
});

const shutdown = async () => {
  console.log('\n[relay] shutting down…');
  try {
    await node.stop();
  } catch (err) {
    console.error('[relay] error during stop:', err);
  }
  process.exit(0);
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
