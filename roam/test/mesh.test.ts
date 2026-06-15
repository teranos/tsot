import { test, expect, beforeAll, afterAll } from 'bun:test';
import { createLibp2p } from 'libp2p';
import type { Libp2p } from '@libp2p/interface';
import { webSockets } from '@libp2p/websockets';
import { noise } from '@chainsafe/libp2p-noise';
import { yamux } from '@chainsafe/libp2p-yamux';
import { identify } from '@libp2p/identify';
import { gossipsub } from '@chainsafe/libp2p-gossipsub';
import { multiaddr } from '@multiformats/multiaddr';
import { unlinkSync } from 'node:fs';

const TOPIC = 'roam-positions/v1';
const MULTIADDR_FILE = './dist/relay-multiaddr.txt';

async function pollUntil<T>(
  fn: () => T | undefined | Promise<T | undefined>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const v = await fn();
    if (v !== undefined && v !== null && (typeof v !== 'number' || v > 0) && (!Array.isArray(v) || v.length > 0)) {
      return v as T;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timeout waiting for: ${label}`);
}

async function makePeer(): Promise<Libp2p> {
  const node = await createLibp2p({
    addresses: { listen: [] },
    transports: [webSockets()],
    connectionEncrypters: [noise()],
    streamMuxers: [yamux()],
    connectionGater: { denyDialMultiaddr: async () => false },
    services: {
      identify: identify(),
      pubsub: gossipsub({ allowPublishToZeroTopicPeers: true, emitSelf: false }),
    },
  });
  await node.start();
  return node;
}

let relayProc: ReturnType<typeof Bun.spawn> | undefined;
let relayAddr = '';
let A: Libp2p | undefined;
let B: Libp2p | undefined;

beforeAll(async () => {
  try { unlinkSync(MULTIADDR_FILE); } catch {}
  relayProc = Bun.spawn(['bun', 'run', 'relay/relay.ts'], {
    stdout: 'pipe',
    stderr: 'pipe',
  });
  relayAddr = await pollUntil(async () => {
    try {
      const t = await Bun.file(MULTIADDR_FILE).text();
      const first = t.trim().split('\n')[0];
      return first && first.length > 0 ? first : undefined;
    } catch { return undefined; }
  }, 10_000, 'relay multiaddr file');

  A = await makePeer();
  B = await makePeer();
  await A.dial(multiaddr(relayAddr));
  await B.dial(multiaddr(relayAddr));
  (A.services.pubsub as any).subscribe(TOPIC);
  (B.services.pubsub as any).subscribe(TOPIC);
  await pollUntil(() => {
    const meshA = ((A!.services.pubsub as any).getMeshPeers?.(TOPIC) || []).length;
    const meshB = ((B!.services.pubsub as any).getMeshPeers?.(TOPIC) || []).length;
    return meshA > 0 && meshB > 0 ? true : undefined;
  }, 15_000, 'both peers meshed');
});

afterAll(async () => {
  if (A) await A.stop();
  if (B) await B.stop();
  relayProc?.kill();
});

test('round-trip: A publishes, B receives, and the reverse', async () => {
  const pubA = A!.services.pubsub as any;
  const pubB = B!.services.pubsub as any;

  const recvB: any[] = [];
  const recvA: any[] = [];
  const onB = (e: any) => { if (e.detail?.topic === TOPIC) recvB.push(JSON.parse(new TextDecoder().decode(e.detail.data))); };
  const onA = (e: any) => { if (e.detail?.topic === TOPIC) recvA.push(JSON.parse(new TextDecoder().decode(e.detail.data))); };
  pubB.addEventListener('message', onB);
  pubA.addEventListener('message', onA);

  const fromA = { from: 'A', t: Date.now() };
  await pubA.publish(TOPIC, new TextEncoder().encode(JSON.stringify(fromA)));
  await pollUntil(() => recvB.length > 0 ? recvB.length : undefined, 5_000, 'B receives A');
  expect(recvB[0]).toEqual(fromA);

  const fromB = { from: 'B', t: Date.now() };
  await pubB.publish(TOPIC, new TextEncoder().encode(JSON.stringify(fromB)));
  await pollUntil(() => recvA.length > 0 ? recvA.length : undefined, 5_000, 'A receives B');
  expect(recvA[0]).toEqual(fromB);

  pubB.removeEventListener('message', onB);
  pubA.removeEventListener('message', onA);
}, 60_000);

test('soak: 30s @ 20 Hz both ways, ≥95% delivery, counted aborts', async () => {
  const SOAK_MS = 30_000;
  const RATE_HZ = 20;
  const INTERVAL_MS = 1000 / RATE_HZ;
  const MIN_DELIVERY = 0.95;

  const pubA = A!.services.pubsub as any;
  const pubB = B!.services.pubsub as any;

  let sentA = 0, recvB = 0;
  let sentB = 0, recvA = 0;
  let closesA = 0, closesB = 0;
  let pubErrA = 0, pubErrB = 0;

  const onB = (e: any) => { if (e.detail?.topic === TOPIC) recvB++; };
  const onA = (e: any) => { if (e.detail?.topic === TOPIC) recvA++; };
  const closeA = () => closesA++;
  const closeB = () => closesB++;
  pubB.addEventListener('message', onB);
  pubA.addEventListener('message', onA);
  A!.addEventListener('connection:close', closeA);
  B!.addEventListener('connection:close', closeB);

  const start = Date.now();
  const tickA = setInterval(() => {
    sentA++;
    pubA.publish(TOPIC, new TextEncoder().encode(JSON.stringify({ from: 'A', seq: sentA, t: Date.now() })))
      .catch(() => { pubErrA++; });
  }, INTERVAL_MS);
  const tickB = setInterval(() => {
    sentB++;
    pubB.publish(TOPIC, new TextEncoder().encode(JSON.stringify({ from: 'B', seq: sentB, t: Date.now() })))
      .catch(() => { pubErrB++; });
  }, INTERVAL_MS);

  const report = setInterval(() => {
    const elapsed = ((Date.now() - start) / 1000).toFixed(1);
    const lossAB = sentA > 0 ? (1 - recvB / sentA) * 100 : 0;
    const lossBA = sentB > 0 ? (1 - recvA / sentB) * 100 : 0;
    console.log(`  t=${elapsed}s  A→B sent=${sentA} recv=${recvB} loss=${lossAB.toFixed(1)}% pubErr=${pubErrA}  B→A sent=${sentB} recv=${recvA} loss=${lossBA.toFixed(1)}% pubErr=${pubErrB}  closes A=${closesA} B=${closesB}`);
  }, 5000);

  await new Promise((r) => setTimeout(r, SOAK_MS));

  clearInterval(tickA);
  clearInterval(tickB);
  clearInterval(report);
  await new Promise((r) => setTimeout(r, 500));

  const dAB = sentA > 0 ? recvB / sentA : 0;
  const dBA = sentB > 0 ? recvA / sentB : 0;
  console.log(`final  A→B: ${(dAB * 100).toFixed(2)}% (${recvB}/${sentA})  B→A: ${(dBA * 100).toFixed(2)}% (${recvA}/${sentB})  closes A=${closesA} B=${closesB}`);

  pubB.removeEventListener('message', onB);
  pubA.removeEventListener('message', onA);
  A!.removeEventListener('connection:close', closeA);
  B!.removeEventListener('connection:close', closeB);

  expect(dAB).toBeGreaterThanOrEqual(MIN_DELIVERY);
  expect(dBA).toBeGreaterThanOrEqual(MIN_DELIVERY);
}, 120_000);
