# roam — Canonical World Axiom

## The axiom

**World transformations are social consensus, not raw state writes.**
Two reality classes exist:

- **Canonical** — the shared Teranos. Changes here propagate to every
  peer and persist as the world's history.
- **Non-canonical** — a personal sandbox. Changes are local to the
  player; no other peer sees them. The world the player inhabits is
  rendered from canonical + their personal overlay.

A player's reality class is determined by their identity. Identified
players (verified through the identity provider layer) default to
canonical. Unidentified or unverified players default to non-canonical.

Runtime criterion: `roam::identity::is_identified_self` /
`is_identified_peer`. Future stack picks (did:key, UCAN, M5 signed-
message verification) tighten the rule without changing the call site.

## Anti-grief by structure, not enforcement

Because non-canonical changes never propagate, griefers and spammers
who haven't established identity are silently sandboxed. They can build
whatever they want — none of it touches the canonical world or any
other player. They may not even realize they've been demoted. No bans,
no moderation queue, no enforcement loop required at this layer.

<!-- IDENTITY MENU:
       M6 — implement canonical vs non-canonical world-state routing (the actual fork).
       M7 — implement the promotion flow with the reset axiom enforced. -->

## Transition: non-canonical → canonical

Promotion to canonical **resets** the player's personal world. The
non-canonical work disappears; the player enters Teranos at the
canonical state. This is the entry fee — and the narrative threshold
of *deciding to play for real*.

Once canonical, players don't go back. The reset is one-way; the
threshold is one-way.

## Implications

- Non-canonical is the tutorial / sandbox space by construction. New
  players experiment in private until they choose to enter.
- The reset is meaningful loss, which is why it's also meaningful
  commitment.
- The canonical world's history is shaped only by identified players,
  giving moderation and identity layers direct authority over what
  Teranos becomes.

## Inventories are canonical world entities

When a canonical player picks up a card (or any object) from a tile,
the card *moves* into their inventory — same entity identity, new
`Location`. The move is a canonical world transformation visible to
every observer in render range; nothing is destroyed and respawned.
Observers see the card animate from the ground tile into the
picker's marker.

This falls out of the object-identity axiom in `docs/UI.md` (every
thing in the universe is one persistent object that keeps identity
through every transformation) the moment you accept the principle.
The alternative — private per-player inventories that the canonical
world doesn't track — would let the card "enter the void" on pickup
from every other player's perspective, contradicting the axiom and
making trades, robbery, vendor stocks, and drop-on-death impossible
to model without inventing a new wire protocol for each.

Non-canonical (sandbox) players carry inventories that exist only in
their personal overlay and don't replicate, consistent with the rest
of the sandbox-isolation rule above. On M7 promotion the sandbox
inventory resets along with everything else in the personal overlay.

Wire shape (current): the M6 pickup gossipsub message already carries
the picker's `did:key`; the render layer uses that to target the
"into the marker" animation. Inventory *contents* as queryable
canonical state (so any peer can ask "what is in A's inventory right
now") is a v0.5+ concern that lands when trades and vendors enter
scope.

<!-- IDENTITY MENU (see roam/docs/IDENTITY.md):
       A1 — read this file cold, write 3 questions it leaves unanswered.
       A4 — read Bluesky/ATProto PDS docs; find where the user's signing key lives.
       A7 — list 5 names per ecosystem who actually ship against each candidate below.
       C1 — once a path is picked, rewrite this "Open" section and remove the candidate list.
       M2 — ATProto PDS bridge so a player's handle can claim the roam identity. -->

## Open

- **Identity provider layer.** Picked: `did:key` (Ed25519, libp2p
  PeerId derived from the same key) as the user-facing identifier;
  `rs-ucan` for cross-device capability delegation (M8, deferred);
  WebAuthn-wrapped Ed25519 for hardware-backed enclave (M3, deferred).
  ATProto reframed as the social / moderation layer that binds an
  ATProto handle to a `did:key` (M2, deferred) — not the identifier
  itself. ActivityPub cut. See `docs/IDENTITY.md` for the menu and
  `research/IDENTITY.md` for the deep read.
- **Personal overlay storage.** Non-canonical changes have to live
  somewhere — local IndexedDB, federated personal store, or both. Out
  of scope until the identity layer exists.
- **Split realities by label-set.** Beyond the binary canonical /
  non-canonical, ATProto-style labelling could permit multiple coexisting
  canonical-class realities (different worlds per label-set). This
  document doesn't yet specify how, just notes the door.
