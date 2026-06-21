# roam — Identity research notes

Research findings from IDENTITY.md menu items A2 / A3 / A7 / A8 and
the Fission deep-read. Reference material for the M5 / M6 / M7 / M8
mains and for S3 / S4 (export / import).

Items in `~~strike~~` in `IDENTITY.md` cite this doc as their record.

---

## Methodology (how this was assembled)

For each ecosystem entry below, I went to the most authoritative
source first (official adopter pages, spec interop reports, the spec
repo itself). When the official source was thin or returned 404
(libp2p users page), I fell back to: the spec's blog posts, GitHub
topic searches, and independent ecosystem-roundup articles. Cross-
checked one source against another where possible. Names below carry
a confidence flag.

---

## A7 — Who actually ships against the PL identity stack

ATProto and ActivityPub rows skipped per the project's PL-stack
alignment (M2 is off-path).

### libp2p — adopters

The official libp2p users page (`docs.libp2p.io/concepts/introduction/users/`)
returned 404. Names below come from the libp2p Ethereum-merge blog,
the Filecoin spec page, and a Medium ecosystem roundup that the
search surfaced. All cross-confirmed.

Confidence: high (each name is independently known as libp2p-based
at the network layer).

- **IPFS** — Protocol Labs' flagship; libp2p's first major user.
- **Filecoin** — libp2p in the Filecoin spec.
- **Ethereum consensus layer** — Lighthouse, Prysm, Teku, Nimbus,
  Lodestar; the merge transition documented in the libp2p blog.
- **Polkadot / Substrate** — Parity's stack.
- **Polygon, Celestia, Mina, Flow** — named in the third-party
  roundup, all independently known.

Other names that surfaced (lower priority for roam): Nervos, Status.im,
Harmony, Golem, Keep Network.

### did:key — implementers

Source: W3C CCG did:key Method Interoperability Report 1.0 (test
results table, run 2026-05-17). Authoritative.

Confidence: high (named in the interop report).

- **DIF** (Decentralized Identity Foundation)
- **Digital Bazaar, Inc.**
- **Procivis One Core**
- **SpruceID**

Fifth implementer not found in the interop report. Candidates from
memory but unverified by me: **Veramo**, **Trinsic**, **Microsoft ION**.
Verify against the report before relying.

### UCAN — production users and library shippers

Sources: ucan-wg GitHub org, Storacha docs and Medium posts, Fission
guide pages.

Confidence: high for the named orgs; medium for the listed Storacha
customers (publicly named but not verified end-to-end).

- **Storacha** (formerly web3.storage) — ships `w3up`, `ucanto`
  (UCAN-RPC), `add-to-web3` GitHub Action. UCAN is the core auth
  primitive. Petabytes of data managed.
- **Fission Codes** — created UCAN. Ships ODD SDK (formerly
  `webnative`), `fission-suite` Haskell tooling, runs `ucan.xyz`.
- **ucan-wg** organization itself — `ts-ucan`, `rs-ucan`, `go-ucan`
  reference implementations. The spec authors' own libraries.
- **@ipld/dag-ucan** — IPLD ecosystem's UCAN encoding library.
- **Storacha customer ecosystem** — Starling Labs / Shoah Project
  (journalist-anonymity uploads via UCAN delegation), OpenSea
  (delegated upload capabilities scoped by payment plan).

### Gaps to flag honestly

- libp2p has no clean "5 production end-users" list anywhere I could
  find. The names above are real but their categorization (L1 chains,
  storage networks, file sharing) varies.
- did:key implementer count is 4 verified, not 5.
- UCAN's ecosystem is small and Fission → Storacha centric. That's
  the truth of the ecosystem today, not a research gap.

---

## Fission vision (deep read)

Two threads that get conflated: the **2018 "Vision of FISSION"**
Medium article (Ethereum smart-contract messaging — ERC-1066 status
codes + ERC-1444 localization) is from Brooklyn Zelenka's earlier
protocol work and **is not** the current Fission Codes company
vision. The current vision is downstream — same author, different
decade, different problem.

### Core thesis (current company)

Local-first software. Apps run on the user's device, data lives
there primarily, the network is for collaboration not control. The
user's identity, data, and compute belong to the user, not a platform.

### How they manifest it

- **Identity is yours.** A keypair you mint and hold (DID), not an
  account on someone's server. Login = sign with your key.
- **Data is yours.** WNFS (Webnative Filesystem) — encrypted,
  versioned, CRDT-merged, lives on the user's device, replicates via
  IPFS as content-addressed blobs. Even Fission infrastructure
  can't read.
- **Authorization is capability-based, not identity-based.** UCAN:
  you sign tokens saying "this app can do X for the next 24 hours."
  Delegate what an app can do, never hand over who you are.
- **No central server required.** Fission's own infrastructure is
  optional convenience for non-technical users.

### Cultural framing

"A more compassionate and connected world of software." Explicitly
distancing from the speculation-focused side of web3. Peer group is
local-first researchers (Ink & Switch), capability-security folks,
edge-compute people.

### Brooklyn Zelenka's intellectual lineage

Created UCAN, co-founded Fission, recent work on **IPVM**
(Interplanetary Virtual Machine — portable compute via IPLD). The
constant across the work is "give the user the keys, build the
infrastructure that respects that."

---

## ODD SDK (current state, 2026)

ODD SDK is the current shipping SDK from Fission Codes — a rewrite
of `webnative` that's explicitly **independent of Fission
infrastructure**. Repo: `github.com/oddsdk/ts-odd`.

### Developer API

- User account creation and session management
- File system operations (POSIX-style: `ls`, `mkdir`, `mv`, `read`,
  `rm`, `write`, `exists`) backed by WNFS
- File versioning via `history` property with delta-based navigation
- Encrypted storage with public/private branch separation
- Key management and device linking
- Capability system via UCANs

### Fission-dependent vs. infrastructure-independent

**Dependent on Fission infra:**
- Account registration and recovery flows
- "Fission auth lobby" for capability-based permissions
- Device pairing authentication server (two-factor-like PIN flow)

**Infrastructure-independent:**
- Core cryptography via the browser's Web Crypto API
- File storage backend (IPLD/IPFS compatible — any IPFS node works)
- Offline-first operation
- Alternative auth strategies (e.g., wallet plugins)

### Identity & auth shape

- Browser Web Crypto API holds the key material
- Session resumption via browser storage
- Sessions are checked via `program.session`
- Passkey support is "actively being worked on" — not done yet
- Currently uses passwordless login + device-linking via UCANs

### Device pairing (most relevant for roam S3 / S4 / M8)

- **No private key ever crosses the wire.** Each device is an
  independent agent with its own keypair + agent DID.
- Producer device shows a PIN; consumer device confirms match.
- WebSocket-bootstrapped authenticated session; the symmetric
  session key gets included in the `facts` section of a UCAN
  (delegates no permissions) to prove the symmetric key originated
  with the producer.
- Producer then issues an **account UCAN** that delegates authority
  from the user's primary account DID to the consumer's agent DID.
- Consumer later presents the account UCAN as proof it can act on
  the user's behalf.

This is the pattern roam should copy for S3 / S4 (export / import)
and M8 (cross-device control). No key transfer, only capability
delegation — exactly what M8's menu line asks for.

### Key recovery

Recovery Kit download mechanism. Requires `oldUsername`,
`readKey`, and new credentials. No expressed opinion on key escrow
or guardian systems.

---

## UCAN spec (deep notes, for M8 implementation)

Source: `ucan-wg/spec` repo, README and main spec.

### Token format

**Envelope:**
- `.0`: Raw signature bytes by issuer over payload
- `.1`: Signature payload containing:
  - `.1.h`: Varsig v1 header (describes signing algorithm)
  - `.1.ucan/<subspec-tag>@<version>`: Token payload

**Required payload fields:**
- `iss` (DID): Issuer
- `aud` (DID): Audience
- `sub` (DID): Subject — the principal the chain concerns
- `cmd` (string): Command to invoke (e.g., `/crud/read`, `/msg/send`)
- `args` (object): Arguments at invocation
- `nonce` (bytes): 12 random bytes recommended (replay prevention)
- `exp` (integer | null): Expiration UTC timestamp (`null` = never)
- `nbf` (integer, optional): "Not before" timestamp
- `meta` (object, optional): Self-evident metadata; doesn't delegate

**Encoding:** DAG-CBOR for signing; DAG-JSON for storage/display.
CIDv1 with base58btc multibase, SHA-256 multihash.

### Delegation

Each delegation passes authority issuer → audience. Each link in the
chain must:
- Be signed by the issuer's private key
- Reference previous delegations as cryptographic proofs
- **Attenuate (narrow) capabilities — never expand them.**
- Chains are immutable and time-bound. Validity interval = latest
  `nbf` to earliest `exp` across the chain.

### Capability shape

`capability = subject × command × policy`

- **Subject:** DID (often `did:key`)
- **Command:** lowercase, slash-prefixed, segment-delimited
  (`/crud/create`, `/crypto/sign`). Top-level `/` = all capabilities
  for a resource.
- **Policy:** constraint logic (e.g., match sender to specific email)

Authority is the union of all delegated capabilities; overlaps follow
set semantics.

### Invocation vs. delegation

- **Delegation:** Passes authority to another agent; idempotent;
  occurs offline.
- **Invocation:** Exercises delegated authority; must be within
  validity interval; includes proof chain; signed by invoker;
  must have unique CID to prevent replay.

### Verifier requirements

- Verify each signature using issuer's public key (from DID)
- Confirm audience matches current validator's DID
- Walk proof chain to establish unbroken delegation path
- Audience of each prior delegation matches issuer of the next
- Reject if current time exceeds `exp` or precedes `nbf`
- Apply ±60 second clock drift buffer
- Compute validity interval as latest `nbf` to earliest `exp`
- Maintain local store of revoked UCAN CIDs
- Track seen CIDs locally (set or Bloom filter) for replay
  prevention; reject duplicate CID invocations within expiry window

---

## Recommendations for roam

Repeating the Fission-vs-Storacha conclusion in actionable form.

### Adopt as a library, not a framework

- **For M8 (UCAN capability delegation):** use `rs-ucan` (spec
  authors' own Rust implementation; smallest dep surface). Don't
  build delegation chains from scratch. Don't adopt all of ODD SDK
  — it bundles WNFS + identity-platform UX that roam doesn't need.
- **For M3 (WebAuthn-wraps-Ed25519) if pursued:** read ODD SDK's
  source for the wrap pattern, reimplement in roam's worker via
  `web-sys` + Web Crypto API. Don't depend on ODD.
- **For S3 / S4 (export / import):** read ODD SDK's device-linking
  flow above. Copy the pattern: producer device generates session
  key, consumer device confirms PIN, producer issues account UCAN.
  No key transfer.
- **For M5 (signature verification at relayer):** libp2p territory.
  Fission doesn't help. gossipsub `ValidationMode::Strict` already
  does most of this at the libp2p layer; M5's productive add is
  expose the verified pubkey to the application + tighten M4's peer
  rule.
- **For M6 / M7 (canonical / non-canonical, promotion):** roam-
  specific. No external reference; build yourself.

### What you can't avoid building

- Gossipsub topology and relay architecture (libp2p)
- Canonical / non-canonical world-state CRDT (M6)
- Promotion-with-reset flow (M7)
- Game-specific mechanics (positions, pickups, mesh)

### What you DO save by reading Fission

The hard *research* on identity-and-capability UX in browsers:
- How to make device pairing feel intuitive without key transfer
- How to do key recovery without guardians or escrow
- How to express "this app can do X for 24 hours" as a primitive
- How to walk a delegation chain on the verifier side without
  pulling in a framework

Read their code, depend on `rs-ucan`, build the rest.

---

## Sources

### libp2p
- [libp2p projects page](https://libp2p.io/projects/)
- [libp2p & Ethereum (the Merge) blog](https://libp2p.io/blog/libp2p-and-ethereum/)
- [Filecoin spec — libp2p](https://spec.filecoin.io/libraries/libp2p/)
- [Libp2p network engine behind Ethereum, Polkadot, IPFS — Medium roundup](https://medium.com/@anmoldh121/libp2p-the-network-engine-behind-ethereum-polkadot-and-ipfs-bc2686affa6d)

### did:key
- [did:key Method Interoperability Report 1.0](https://w3c-ccg.github.io/did-key-test-suite/)
- [The did:key Method v0.9 spec](https://w3c-ccg.github.io/did-key-spec/)
- [W3C CCG did:key Method repo](https://github.com/w3c-ccg/did-key-spec)

### UCAN
- [UCAN spec (ucan-wg)](https://github.com/ucan-wg/spec)
- [UCAN spec README](https://github.com/ucan-wg/spec/blob/main/README.md)
- [ucan-wg/go-ucan](https://github.com/ucan-wg/go-ucan)
- [ucan-wg/invocation](https://github.com/ucan-wg/invocation)

### Fission Codes
- [Fission Codes homepage — The future of identity, data, and compute](https://fission.codes/)
- [Brooklyn Zelenka interview — Fission CTO](https://fission.codes/blog/meet-the-fission-team-an-interview-with-our-cto-brooke-zelenka/)
- [The Evolution of Local-First Software](https://fission.codes/blog/the-evolution-of-local-first-software-empowering-users-in-a-connected-world/)
- [On Building a Decentralized Database](https://fission.codes/blog/on-building-a-decentralized-database/)
- [The Edge of Tomorrow — IPVM](https://fission.codes/blog/edge-of-tomorrow/)
- [A Vision of FISSION — 2018 Ethereum precursor](https://medium.com/spadebuilders/vision-of-fission-b4f9e00c6cb3)
- [Fission Guide — Authentication Strategies](https://guide.fission.codes/developers/webnative/authentication-strategies)
- [Fission Guide — Auth + Device Linking](https://guide.fission.codes/developers/webnative/auth)
- [WNFS — Webnative Filesystem](https://guide.fission.codes/developers/webnative/file-system-wnfs)

### ODD SDK
- [oddsdk/ts-odd repo](https://github.com/oddsdk/ts-odd)
- [Fission Codes WNFS ecosystem page](https://fission.codes/ecosystem/wnfs/)

### Storacha
- [Storacha Network](https://storacha.network/)
- [Storacha UCAN concepts docs](https://docs.storacha.network/concepts/ucan/)
- [Storacha w3up — UCAN protocol implementation](https://github.com/storacha/w3up)
- [Storacha — "The Internet Is Permissioned Wrong" (Medium)](https://medium.com/@storacha/the-internet-is-permissioned-wrong-but-ucan-fix-it-439c36c5dc79)
- [Storacha — Filecoin Foundation ecosystem entry](https://fil.org/ecosystem-explorer/storacha-network)
