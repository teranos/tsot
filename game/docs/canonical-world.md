# Canonical world mechanics

Subordinate to [VISION](../VISION.md), which is authoritative. This doc
holds the identity and world-state mechanics only; where anything here
conflicts with VISION, VISION wins.

## Reality classes

Two reality classes, set by a player's identity:

- **Canonical**: the shared Teranos. Changes here propagate to every
  peer.
- **Non-canonical**: a personal sandbox. Changes are local to the
  player; no other peer sees them. The world the player inhabits is
  rendered from canonical + their personal overlay.

Identified players (verified through the identity provider layer)
default to canonical. Unidentified or unverified players default to
non-canonical.

## Anti-grief by structure, not enforcement

Because non-canonical changes never propagate, griefers and spammers
who haven't established identity are silently sandboxed. They can build
whatever they want; none of it touches the canonical world or any
other player. They may not even realize they've been demoted. No bans,
no moderation queue, no enforcement loop required at this layer.

## Onboarding: non-canonical → canonical

Promotion to canonical **resets** the player's personal world. The
non-canonical work disappears; the player enters Teranos at the
canonical state. This is the entry fee, and the narrative threshold
of *deciding to play for real*.

- Non-canonical is the tutorial / sandbox space by construction. New
  players experiment in private until they choose to enter.
- The reset is meaningful loss, which is why it's also meaningful
  commitment.
- The canonical world's history is shaped only by identified players,
  giving moderation and identity layers direct authority over what
  Teranos becomes.

## Inventory

When a canonical player picks up a card (or any object) from a tile,
the card *moves* into their inventory: same entity identity, new
`Location`. The move is a canonical world transformation visible to
every observer in render range; nothing is destroyed and respawned.
Observers see the card animate from the ground tile into the
picker's marker.

The alternative, private per-player inventories that the canonical
world doesn't track, would let the card "enter the void" on pickup
from every other player's perspective, making trades, robbery, vendor
stocks, and drop-on-death impossible to model without inventing a new
wire protocol for each.

Non-canonical (sandbox) players carry inventories that exist only in
their personal overlay and don't replicate. On promotion the sandbox
inventory resets along with everything else in the personal overlay.

Cards that outlive a universe carry over through the player's federated
identity (ActivityPub / ATProto), not through world-state; the universe
itself is impermanent (see VISION).

## Split realities by label-set

Beyond the binary canonical / non-canonical, ATProto-style labelling
could permit multiple coexisting canonical-class realities (different
worlds per label-set). This is the mechanism behind VISION's many
universes running in parallel. Not yet specified, just the door.

## Open

- **Personal overlay storage.** Non-canonical changes have to live
  somewhere: local IndexedDB, federated personal store, or both. Out
  of scope until the identity layer exists.
