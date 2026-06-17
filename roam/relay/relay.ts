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
import {
  SecretsManagerClient,
  GetSecretValueCommand,
  PutSecretValueCommand,
  ResourceNotFoundException,
} from '@aws-sdk/client-secrets-manager';
import {
  CloudWatchClient,
  PutMetricDataCommand,
} from '@aws-sdk/client-cloudwatch';

const KEY_FILE = './relay/.peer-key';
const MULTIADDR_FILE = './dist/relay-multiaddr.txt';
const TOPIC = 'roam-positions/v1';

// Env-driven so the same binary runs locally (loopback, plain ws) and
// behind a TLS terminator like CloudFront (bind 0.0.0.0, announce the
// public DNS over wss/443). Defaults match the historical local-dev
// behavior; production deploy sets ROAM_RELAY_LISTEN_HOST=0.0.0.0 and
// ROAM_RELAY_ANNOUNCE to the public multiaddr.
const LISTEN_HOST = process.env.ROAM_RELAY_LISTEN_HOST ?? '127.0.0.1';
const LISTEN_PORT = Number(process.env.ROAM_RELAY_LISTEN_PORT ?? '9001');
const ANNOUNCE = process.env.ROAM_RELAY_ANNOUNCE ?? `/dns4/localhost/tcp/${LISTEN_PORT}/ws`;
// Disable the dist/relay-multiaddr.txt write on deployments where the
// relay box doesn't carry the static bundle (CloudFront serves it from
// S3 instead). Set ROAM_RELAY_WRITE_DIST=0 in that environment.
const WRITE_DIST_MULTIADDR = process.env.ROAM_RELAY_WRITE_DIST !== '0';

// Identity source. If ROAM_RELAY_IDENTITY_SECRET is set, the relay
// fetches its Ed25519 private key from AWS Secrets Manager; first-ever
// startup PUT-s a freshly generated key into the (already-created)
// secret. Box recreation then re-uses the same identity, so the
// peer-id (and therefore the multiaddr clients bootstrap against)
// stays stable across rebuilds.
//
// When the env var is unset, the relay falls back to the
// `./relay/.peer-key` local file — local dev keeps working without
// any AWS credentials.
const IDENTITY_SECRET = process.env.ROAM_RELAY_IDENTITY_SECRET ?? '';
const AWS_REGION = process.env.AWS_REGION ?? process.env.ROAM_AWS_REGION ?? 'eu-central-1';

async function loadOrCreateKeyFromSecretsManager(secretId: string): Promise<PrivateKey> {
  // Credentials come from the standard provider chain: env vars
  // (ROAM_AWS_ACCESS_KEY_ID / ROAM_AWS_SECRET_ACCESS_KEY mapped into
  // AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY by the systemd unit, or
  // set directly), config files, instance metadata. Lightsail doesn't
  // give us instance metadata, so env vars are the production path.
  const client = new SecretsManagerClient({ region: AWS_REGION });
  try {
    const out = await client.send(new GetSecretValueCommand({ SecretId: secretId }));
    // SecretBinary if PUT as bytes, SecretString if PUT as string. We
    // always PUT bytes from Rust's perspective, but Bun/SDK round-trips
    // can land on SecretString depending on encoding. Handle both.
    let bytes: Uint8Array;
    if (out.SecretBinary) {
      bytes = out.SecretBinary instanceof Uint8Array
        ? out.SecretBinary
        : new Uint8Array(out.SecretBinary as ArrayBufferLike);
    } else if (out.SecretString) {
      bytes = Buffer.from(out.SecretString, 'base64');
    } else {
      throw new Error(`secret ${secretId} has neither SecretBinary nor SecretString`);
    }
    console.log(`[relay] loaded identity from Secrets Manager (${secretId}, ${bytes.byteLength} bytes)`);
    return privateKeyFromProtobuf(bytes);
  } catch (err) {
    if (err instanceof ResourceNotFoundException || (err as any)?.name === 'ResourceNotFoundException') {
      throw new Error(
        `Secret ${secretId} does not exist in region ${AWS_REGION}. Create it via tofu (infra/identity.tf) before starting the relay; the relay does NOT create secrets — only writes a value into an existing one.`,
      );
    }
    throw err;
  }
}

async function ensureValueInSecret(secretId: string, key: PrivateKey): Promise<void> {
  // Called when the existing secret's current value can't be decoded
  // as a protobuf private key — i.e. the secret was just created by
  // tofu and is empty / placeholder. PUT a freshly generated key so
  // the next startup loads it back.
  const client = new SecretsManagerClient({ region: AWS_REGION });
  const bytes = privateKeyToProtobuf(key);
  await client.send(new PutSecretValueCommand({
    SecretId: secretId,
    // Base64 for SecretString to keep round-trips lossless.
    SecretString: Buffer.from(bytes).toString('base64'),
  }));
  console.log(`[relay] PUT new identity into Secrets Manager (${secretId}, ${bytes.byteLength} bytes)`);
}

async function loadOrCreateKey(): Promise<PrivateKey> {
  if (IDENTITY_SECRET) {
    try {
      return await loadOrCreateKeyFromSecretsManager(IDENTITY_SECRET);
    } catch (err) {
      // GetSecretValue can succeed with an empty / non-decodable
      // value if the secret was just created by tofu and the relay is
      // doing first-write. Detect by attempting to decode; if it
      // fails, generate + PUT.
      const msg = (err as Error)?.message ?? '';
      if (msg.includes('does not exist')) {
        // Fatal — tofu didn't create the secret. Surface, don't auto-fix.
        throw err;
      }
      // Likely first-run: empty secret. Generate + PUT.
      console.log(`[relay] secret unreadable as identity (${msg}); generating new key and PUT-ing`);
      const key = await generateKeyPair('Ed25519');
      await ensureValueInSecret(IDENTITY_SECRET, key);
      return key;
    }
  }

  if (existsSync(KEY_FILE)) {
    const bytes = await readFile(KEY_FILE);
    return privateKeyFromProtobuf(bytes);
  }
  const key = await generateKeyPair('Ed25519');
  await mkdir('./relay', { recursive: true });
  await writeFile(KEY_FILE, privateKeyToProtobuf(key));
  console.log('[relay] generated new keypair (local file)');
  return key;
}

const privateKey = await loadOrCreateKey();

const node = await createLibp2p({
  privateKey,
  addresses: {
    // Listen wherever the env says (loopback locally, 0.0.0.0 on a
    // public box); announce whatever the env says (loopback locally,
    // the public wss multiaddr in front of CloudFront / a real TLS
    // terminator). Both default to the historical local-dev shape.
    listen: [`/ip4/${LISTEN_HOST}/tcp/${LISTEN_PORT}/ws`],
    announce: [ANNOUNCE],
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
if (WRITE_DIST_MULTIADDR) {
  await mkdir('./dist', { recursive: true });
  await writeFile(MULTIADDR_FILE, multiaddrs.join('\n') + '\n');
}

console.log(`[relay] peerId:  ${node.peerId.toString()}`);
console.log(`[relay] listening on:`);
for (const a of multiaddrs) console.log(`  ${a}`);
console.log(`[relay] subscribed to ${TOPIC}`);
if (WRITE_DIST_MULTIADDR) {
  console.log(`[relay] wrote multiaddrs → ${MULTIADDR_FILE}`);
} else {
  console.log(`[relay] dist file write disabled (ROAM_RELAY_WRITE_DIST=0)`);
}

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

// CloudWatch metric publishing. The relay reports its own RSS, peer
// count, and pubsub message rate every 60s, directly via PutMetricData.
// No CloudWatch agent, no SSM-stored config, no separate credential
// surface — the same IAM user that owns the identity secret has
// `cloudwatch:PutMetricData` scoped to the `CWAgent` namespace via the
// policy in `infra/observability.tf`. The alarms there match these
// metric names + dimensions exactly.
//
// Disable in dev by leaving AWS_REGION unset (the SDK fails on first
// call; we catch and downgrade to an info log so the relay still runs).
const METRIC_NAMESPACE = process.env.ROAM_RELAY_METRIC_NAMESPACE ?? 'CWAgent';
const INSTANCE_NAME = process.env.ROAM_RELAY_INSTANCE_NAME ?? 'roam-relay-eu-2';
const METRIC_INTERVAL_MS = Number(process.env.ROAM_RELAY_METRIC_INTERVAL_MS ?? '60000');
const PUBLISH_METRICS = process.env.ROAM_RELAY_PUBLISH_METRICS !== '0';

let pubsubMessagesReceived = 0;
pubsub.addEventListener('message', () => { pubsubMessagesReceived++; });

if (PUBLISH_METRICS) {
  const cw = new CloudWatchClient({ region: AWS_REGION });
  let lastMsgCount = 0;
  let consecutiveErrors = 0;
  const publish = async () => {
    try {
      const mem = process.memoryUsage();
      const peers = node.getPeers().length;
      const conns = node.getConnections().length;
      const msgsSinceLast = pubsubMessagesReceived - lastMsgCount;
      lastMsgCount = pubsubMessagesReceived;
      const rate = msgsSinceLast / (METRIC_INTERVAL_MS / 1000);

      const dimensions = [{ Name: 'InstanceName', Value: INSTANCE_NAME }];
      const now = new Date();
      await cw.send(new PutMetricDataCommand({
        Namespace: METRIC_NAMESPACE,
        MetricData: [
          { MetricName: 'procstat_memory_rss',  Dimensions: dimensions, Timestamp: now, Value: mem.rss,      Unit: 'Bytes' },
          { MetricName: 'procstat_memory_vms',  Dimensions: dimensions, Timestamp: now, Value: mem.heapTotal,Unit: 'Bytes' },
          { MetricName: 'mem_used_percent',     Dimensions: dimensions, Timestamp: now, Value: (mem.rss / (1024 * 1024 * 512)) * 100, Unit: 'Percent' },
          { MetricName: 'relay_peer_count',     Dimensions: dimensions, Timestamp: now, Value: peers,        Unit: 'Count' },
          { MetricName: 'relay_connection_count', Dimensions: dimensions, Timestamp: now, Value: conns,      Unit: 'Count' },
          { MetricName: 'relay_pubsub_msgs_per_sec', Dimensions: dimensions, Timestamp: now, Value: rate,    Unit: 'Count/Second' },
        ],
      }));
      consecutiveErrors = 0;
    } catch (err) {
      // Errors-as-first-class but don't crash the relay over a metrics
      // hiccup. Surface once on the first failure, then back off so
      // the journal doesn't fill with the same error every minute.
      consecutiveErrors++;
      if (consecutiveErrors === 1 || consecutiveErrors % 60 === 0) {
        console.error(`[relay] PutMetricData failed (${consecutiveErrors} consecutive): ${(err as Error).message}`);
      }
    }
  };
  setInterval(publish, METRIC_INTERVAL_MS);
  publish();  // immediate first publish so the alarm has data inside its 5-min window
  console.log(`[relay] publishing metrics every ${METRIC_INTERVAL_MS}ms to ${METRIC_NAMESPACE}/InstanceName=${INSTANCE_NAME}`);
} else {
  console.log('[relay] metric publishing disabled (ROAM_RELAY_PUBLISH_METRICS=0)');
}

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
