# Canonical World Axiom

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

## Anti-grief by structure, not enforcement

Because non-canonical changes never propagate, griefers and spammers
who haven't established identity are silently sandboxed. They can build
whatever they want — none of it touches the canonical world or any
other player. They may not even realize they've been demoted. No bans,
no moderation queue, no enforcement loop required at this layer.

## Transition: non-canonical → canonical

Promotion to canonical **resets** the player's personal world. The
non-canonical work disappears; the player enters Teranos at the
canonical state. This is the entry fee — and the narrative threshold
of *deciding to play for real*.

Once canonical, players don't go back. The reset is one-way; the
threshold is one-way.

after you finish the 1st new-player-strip the 1st 9x1 strip, that's
when you will be prompted to create an account in case you want to
join the p2p game.

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

This falls out of the object-identity axiom (every
thing in the universe is one persistent object that keeps identity
through every transformation) the moment you accept the principle.
The alternative — private per-player inventories that the canonical
world doesn't track — would let the card "enter the void" on pickup
from every other player's perspective, contradicting the axiom and
making trades, robbery, vendor stocks, and drop-on-death impossible
to model without inventing a new wire protocol for each.

Non-canonical (sandbox) players carry inventories that exist only in
their personal overlay and don't replicate, consistent with the rest
of the sandbox-isolation rule above. On promotion the sandbox
inventory resets along with everything else in the personal overlay.

## Open

- **Personal overlay storage.** Non-canonical changes have to live
  somewhere — local IndexedDB, federated personal store, or both. Out
  of scope until the identity layer exists.
- **Split realities by label-set.** Beyond the binary canonical /
  non-canonical, ATProto-style labelling could permit multiple coexisting
  canonical-class realities (different worlds per label-set). This
  document doesn't yet specify how, just notes the door. i think split
  realities actually becomes way easier with the strips. **(Strips
  parked — see game/README.md; split realities now need a home under
  the shared-clock model, e.g. by phase or by label-set at the same
  phase.)**
