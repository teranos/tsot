# roam — Network research notes

Substrate / transport / federation projects worth studying for roam.
Each entry: *what the project is, why it matters for roam, what
specifically to study, what we don't yet know.*

These are research targets, not decisions. Decisions about substrate
choices live in `docs/transport.md`. Decisions about identity choices
live in `docs/identity.md` + `research/identity.md`.

---

## Iroh

**What it is:** Rust p2p networking library (`n0-computer`). Focused
on content-addressed sync and direct connections between devices,
with a relay-fallback path when direct connection isn't possible.

**Why it matters for roam:**
- Built on QUIC — the same substrate WebTransport rides on. Studying
  Iroh tells us what a QUIC-native roam transport would look like in
  Rust, including the relay-fallback patterns we'd inherit.
- Direct device-to-device connections without a central relay match
  the long-term roam goal of "no centralised server" past identity.
- Their hole-punching / NAT-traversal work is exactly the surface
  we'd need to understand if roam ever wants direct peer paths
  (currently 100% relayer-routed).

**What to study (specifically — not "read everything"):**
- Their transport stack assembly (QUIC + magic-relays).
- The relay handoff / fallback decision logic.
- How they model content addressing on top of the transport (separable
  from the transport itself, or coupled?).
- Their gossip layer (`iroh-gossip`) — is it pubsub like libp2p's
  gossipsub, or a different model?

**Don't know yet:**
- Whether Iroh interoperates with libp2p at all, or whether adopting
  any Iroh component means a substrate swap.
- Browser story — does Iroh have a wasm32 client path?

**Sources to start from** (verify before relying on):
- https://github.com/n0-computer/iroh
- https://iroh.computer/

---

## DeltaChat

**What it is:** Messaging app that uses email (IMAP/SMTP) as the
transport. Federated by construction — there's no DeltaChat server;
your mail server is your relay.

**Why it matters for roam:**
- The architectural lesson is "use a transport that's already
  ubiquitous and federated." Email isn't going away; mail servers
  are everywhere; encrypted Autocrypt-style messages ride
  unchanged through them. roam's equivalent question: what's our
  ubiquitous + federated substrate?
- Their handling of multi-device sync without a central server is
  directly relevant to the device-pairing problem we deferred under
  M8 in `docs/identity.md`.
- Their UX patterns for "no account, no signup" are worth studying
  for the guest-mode entry path that M7 needs.

**What to study (specifically):**
- The Autocrypt setup-message protocol (multi-device key sync via
  encrypted email-to-self).
- How they handle group state synchronization without a server.
- Their key verification UX — what's the equivalent of "scan QR"
  for them?

**Don't know yet:**
- Whether anything in their model transfers to roam's higher-rate
  traffic (positions at 5 Hz vs DeltaChat's human-pace messages).
- Whether email-as-transport is a literal candidate for roam or
  just an architectural metaphor.

**Sources to start from** (verify before relying on):
- https://delta.chat/
- https://github.com/deltachat
- Autocrypt spec: https://autocrypt.org/

---

## How to add a new entry

Same shape as above:
1. **What it is** (one paragraph, no marketing).
2. **Why it matters for roam** (the concrete relevance, not "it's
   interesting").
3. **What to study** (named, finite). Not "read the source."
4. **Don't know yet** (the honest gaps).
5. **Sources** with a "verify before relying on" note — these change.
