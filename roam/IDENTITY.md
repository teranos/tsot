# roam — Identity Menu

Every item is consumed once. Strike through (`~~`) when eaten.
Pick an appetizer; the appetizer informs what starter, main,
cleanser, and dessert make sense after it. Items in the same row
of the pairings table below pair naturally — but the menu doesn't
enforce it.

## Pairings (informative, not binding)

| If appetizer is… | Starter likely | Main likely | Cleanser likely | Dessert likely |
|---|---|---|---|---|
| A2 / A3 (DID spec) | S1, S6 | M1 | C2, C3 | D1 |
| A4 (ATProto PDS) | S5, S7 | M2 | C1 | D5 |
| A5 (UCAN) | S7 | M8 | C5 | D3 |
| A6 (WebAuthn flows) | S2 | M3 | C4 | D7 |
| A1 / A8 (read what we have) | S8 | M4 | C1, C3 | D2 |

---

## 🥗 Appetizers — research / exploration (≤ 30 min each)

- **A1.** Read `roam/CANONICAL.md` cold; write down 3 questions it doesn't answer for the current implementation.
- ~~**A2.** Read the W3C `did:key` spec — only the encoding section. Confirm Ed25519 → `did:key:z6Mk…`.~~ ✓ M1 implementation + round-trip / z6Mk-prefix tests cover the encoding section empirically (`identity` branch, 90a4d7e).
- ~~**A3.** Confirm libp2p Ed25519 `PeerId` and `did:key` derive from the same 32-byte public key in different encodings. Note the conversion.~~ ✓ S1 SELF panel renders both for the same `keypair.public()`; conversion is `try_into_ed25519().to_bytes() → ed25519_pubkey_to_did_key` (`identity` branch, 158c212).
- **A4.** Read Bluesky/ATProto PDS docs; locate where a user's signing key lives and how the handle binds to it.
- **A5.** Read UCAN v1.0 spec abstract + the invocation envelope shape.
- **A6.** Open one browser-based DID wallet's UX (e.g. an ATProto client). Note what "verification" looks like to the user, in two sentences.
- ~~**A7.** List, in 5 names each, who in the wider ecosystem actually ships against: libp2p PeerId, `did:key`, ATProto, ActivityPub, UCAN.~~ ✓ findings in `IDENTITY-RESEARCH.md` (ATProto / ActivityPub rows skipped per PL alignment; libp2p ≥5 high confidence; did:key 4 verified + 1 unverified candidate; UCAN 5).
- ~~**A8.** Trace, from source on disk, the data flow from `roam_net_generate_identity_bytes()` to PeerId emission. Draw it on paper.~~ ✓ Post-C3 the chain is short and lives in one module: `roam_net_generate_identity_bytes → identity::generate_identity_protobuf → bridge persists → next session passes bytes into roam_net_worker_provider_init → identity::load_or_generate_keypair → keypair.public() → PeerId`. Pen-and-paper redundant given the linear path.

## 🥄 Starters — small concrete work (1–3 hours)

- ~~**S1.** Surface the `did:key:z6Mk…` derivation in the SELF panel alongside PeerId. View-only.~~ ✓ worker precomputes self_did_key, posts in `kind:'ready'`, SELF panel renders the line below `worker peerId` (`identity` branch, 158c212).
- **S2.** Add a "rotate identity" action: clean IndexedDB → mint fresh. Confirmation gate.
- **S3.** Export keypair: download a small text blob containing the protobuf-encoded keypair.
- **S4.** Import keypair: paste/upload the blob, validate, replace IndexedDB entry.
- **S5.** Sketch a "second-device pairing" flow on paper. QR encoding of the bytes. Don't build, just sketch. *Reference: Fission ODD device-link pattern — no key transfer, account UCAN delegated to consumer's agent DID via PIN-confirmed handshake. See `IDENTITY-RESEARCH.md`.*
- **S6.** Write a wasm-bindgen-test asserting `PeerId == did:key` round-trip for the same keypair.
- **S7.** Sign one position broadcast with the identity key; verify on receiver. Wire-format change is part of this slice.
- **S8.** Write a failing test for `load_or_generate_keypair` that runs on native (extract the function out of the wasm-only gate).

## 🍽️ Main courses — load-bearing implementation (multi-day)

- ~~**M1.** Adopt `did:key` as the project's primary identifier. PeerId becomes the underlying libp2p detail; user-facing surfaces show DID.~~ ✓ encoding `roam::identity::ed25519_pubkey_to_did_key` + decode + 5 falsifiable tests (`identity` branch). UI surfacing tracked under S1.
- **M2.** ATProto PDS bridge: a player's ATProto handle can claim their roam identity. Defines a verification flow.
- **M3.** WebAuthn-wraps-Ed25519: hardware-backed key, never exits the secure enclave. Loses portability for some browsers; gains theft resistance.
- ~~**M4.** Define the structural meaning of "identified" for `CANONICAL.md`. Concrete runtime criterion. Without this, the canonical/non-canonical split has no implementation path.~~ ✓ `roam::identity::is_identified_self` / `is_identified_peer` (`identity` branch).
- **M5.** Gossipsub signature verification at the relayer. Relayer rejects wire messages whose claimed source doesn't match the signing key.
- **M6.** Canonical / non-canonical world-state routing. The actual fork mechanism world transformations route through.
- **M7.** Promotion flow: non-canonical → canonical with the sandbox-reset axiom enforced.
- **M8.** UCAN-based capability delegation: cross-device control via signed capability tokens, no key transfer. *Decided: depend on `rs-ucan` directly; not ODD SDK (JS-first, framework-shaped). See `IDENTITY-RESEARCH.md`.*

## 🍋 Cleansers — between decisions (doc, refactor, audit)

- **C1.** Rewrite `CANONICAL.md` "Open" section once the identity path is picked. Remove the four-name candidate list.
- **C2.** Update `roam/README.md` identity bullets to reflect the picked path. Cut anything that wasn't picked.
- ~~**C3.** Move identity code into `roam/src/identity/` as a dedicated module. Currently scattered across `rust_libp2p.rs` + `wasm_ffi.rs` + `js-bridge.js`.~~ ✓ `roam::identity` module; keypair handling consolidated, JS bridge already extracted to `assets/src/identity.js` in 0.3.2 (`identity` branch).
- **C4.** Write a player-facing one-pager: "what identity means in roam." Not a spec — a UX explanation.
- **C5.** Emit identity events (mint, load, export, import, rotate, sign, verify) into the trace bus with dedicated tags. Render in event log with a color.
- **C6.** Audit every `Keypair::generate_ed25519()` call site across the project. Confirm each one either uses the persistent key or has an explicit reason to generate fresh.

## 🍰 Desserts — polish / closure (small, satisfying)

- **D1.** Show `you are: <short-DID>` in the world-HUD next to peer count.
- ~~**D2.** Tag the version after a main course completion. Annotate with what closed.~~ ✓ `v0.3.4` on `1d233bd` — annotation lists M4, C3, M1, S1, C2, A7, A2/A3/A8 + the rs-ucan / ATProto framing.
- **D3.** Add a sparkline of signed-message rate to the LIBP2P CONNECTIONS panel.
- **D4.** When the relayer status page lands, add a `/identity` route that explains the relay's own identity.
- **D5.** Write a one-paragraph "the why" for the identity choice — CHANGELOG-style, public.
- **D6.** Polish the rotate-identity confirmation modal copy.
- **D7.** Add visible confirmation when an import succeeds (peerId line lights up briefly).

---

## How to use this menu

1. Open the file. Look at the unstrucked items.
2. Pick an appetizer. The pairing table is informative; you can ignore it.
3. The appetizer should change what you know — about the spec, the code, or the ecosystem. After it, the starter/main that fits is usually obvious.
4. Eat the items. Strike them through with `~~` when consumed.
5. When the menu is half-empty, refill: add new items based on what the work revealed. The menu is meant to evolve.

The menu is a forcing function against drift, not a roadmap. Roadmaps commit to order; menus offer choice and remove from inventory.
