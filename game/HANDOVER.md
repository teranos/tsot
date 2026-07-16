
## Next steps

- **Move off the cube-instance renderer.** The current pipeline is
  hardcoded to instanced-cube rendering; it structurally cannot draw a
  continuous polyhedron per building. Adjacent boxes always render as
  separate shaded pieces (double walls at every junction, no matter
  where the boxes sit). The wall-render dissatisfaction on this branch
  is a rendering-substrate limitation, not a placement one. The next
  branch replaces / augments the renderer with a mesh pipeline (new
  env.* imports, new shader, new collider strategy alongside the box
  colliders) and re-does wall emission as one traced polyhedron per
  building with door/window cutouts. Estimated 10–20 hours of
  infrastructure work before it produces its first correct-looking
  wall — hence its own branch, not a squeeze into this one.

## The frontier (open research)

1. **Can the world hold the whole corpus?** `place_nested`, vehicles,
   monsters, multi-tile specials — which unhandled features must land
   to grow past lone buildings? (Our `shed.json` exists only because
   CDDA has no standalone shed; when nested mapgen lands, it can go.)
2. **When does a building stop being scenery?** Which CDDA flags become
   behaviour — TRANSPARENT→glass, doors open/close,
   CONTAINER/SEALED→loot? The hand-placed jukebox is a CDDA furniture
   entry we could *read* instead of *author*.
3. **Should the generator BE CDDA's grammar?** `palette.rs` already
   rolls per-building seeds through variant palettes. How far up
   (block, town) does authored parameter/distribution reach as the
   world generator vs our per-chunk hash?
4. **Can you go down and up?** Roof cut-away + ghost pass is the first
   z-level UX. CDDA ships basements + upper floors as their own
   mapgen. What's descend/ascend in an iso voxel world?
5. **Do lone buildings become towns?** CDDA authors roads, blocks,
   connected specials. Settlement coherence — more authored data
   through the same pipeline, or new placement logic?
6. **Does the map feed combat?** roam v0.5 invokes the ccg engine for
   PvP; CDDA mapgen carries monster spawns. A building's authored
   spawns → encounters resolved by ccg — map as encounter source, not
   just architecture.
