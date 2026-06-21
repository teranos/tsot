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
