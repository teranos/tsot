# DRY Analysis — firsthand-verified

This replaces the earlier subagent-generated report (which had inflated
counts, a wrong finding, and rankings unsupported by verification).

**Method.** Every finding below was read directly, site by site — no subagents.
Where I used `grep` to *count* occurrences I say so; where I *read* the code I
give the `file:line` I opened. I do not rank by severity or invent line totals.

**What I'm anchored to.** `ccg/` and `roam/` are byte-identical between this
branch's base (`c753770`) and current `master` (verified: empty diffs), so the
working tree is accurate for them. `rave/` + the `vendor/laye/` workspace are
read from `origin/bevy-libp2p` (the active branch). `universe/` was **deleted**
on `master`, so it is out of scope (and invalidates the old report's
rave↔universe findings).

**Coverage is partial and stated.** I verified a representative set across ccg
CLI, ccg game, roam render/net, and rave/laye. Areas I did **not** read firsthand
are listed at the end as *unverified leads*, not findings.

---

## Verified duplications

### ccg

**V1 — CLI deck-directory loader, repeated across 7 binaries.**
Read: `cli_balance_probe.rs:134-162` (`load_baselines`) and
`cli_matchup_evolved.rs:43-73`. Both do `read_dir → filter extension=="json" →
sort → for each: EvolvedDeck::load → to_cards → push (label, cards)`, with the
same two-arm `unloadable`/`unparseable` `eprintln`. `grep` confirms 7 CLI files
call `EvolvedDeck::load`: balance_probe, champions_report, curate, evolve,
matchup_evolved, matchup_mcts, prune_champions.
→ Extract a shared `load_deck_dir(registry, dir)` (promote `load_baselines`).

**V2 — `--opponent-ai`→`AiKind` parse, repeated across 4 CLIs.**
Read: `cli_curate.rs:127-132` — `match args.opponent_ai.to_ascii_lowercase()
.as_str() { "heuristic" => Heuristic, "uct" => Uct(UctConfig{iterations,
exploration_c, ..Default}) }`. `grep` for `"heuristic"|"uct"` matches 4 files:
balance_probe, curate, curve_sample, evolve. (The earlier "5" was wrong —
matchup_mcts does not use this block.)
→ Extract `parse_ai_kind(name, iters, c, seed) -> Result<AiKind>`.

**V3 — Jaccard single-linkage union-find clustering, reimplemented 3×.**
Read: `cli_prune_champions.rs:282-315` and `cli_champions_report.rs:225-255`
(`grep` shows a third at `cli_curate.rs:290`). All three: `parent =
(0..n).collect()` → nested `fn find` → pairwise `jaccard >= threshold` union →
`BTreeMap<root, Vec<idx>>` → `into_values` → `sort_by_key(Reverse(len))`.
Notably the `find` bodies have **drifted** — prune does two-pass full compression
(`:284-296`), champions_report does path-halving (`:226-232`) — same algorithm,
different code, which is the drift this duplication invites.
→ Extract `jaccard_clusters(sets, threshold) -> Vec<Vec<usize>>` into
`sim::diversity` (which already exports `jaccard`).

**V4 — `cli_matchup_evolved.rs` computes its aggregates twice (stdout + HTML).**
Read: `fn avg` is defined twice — `:121` (stdout path) and `:451` (inside
`write_matchup_evolved_html`, which starts at `:433`). The file is 891 lines
split into a stdout half and an HTML half that recompute from `all_stats`. I
confirmed the duplicated `fn avg` and the two-half structure; I did **not** line-
count the full overlap (the old "~250 lines" figure is unverified).
→ Compute aggregates once into a struct; feed both renderers.

**V5 — `PlayChoices { … }` full literal at 49 sites in tests.**
Read: `play/errors.rs:79` — `PlayChoices` derives `Default`. `grep -c` in
`play_tests.rs`: 49 `PlayChoices {` literals, **0** use `..Default::default()`.
So every site spells out all fields though only one or two are meaningful.
→ Mechanical: `PlayChoices { hand_payment_ids: …, ..Default::default() }`.
No behavior change; removes the N-site edit cost of adding a field.

**V6 — `zone_mut` duplicated identically.**
Read: `game/movement.rs:13-21` and `game/journal.rs:211-219` — character-for-
character identical 5-arm `match zone { Board/Hand/Deck/Graveyard/Exile }`
(only the param-type spelling differs; same type).
→ One `PlayerState::zone_mut`. Trivial, real.

### roam

**V7 — WebGL program-builder boilerplate, repeated across 4–5 builders.**
Read/grep-confirmed verbatim literals:
`let unit_quad: [f32; 8] = [0.0,0.0,1.0,0.0,0.0,1.0,1.0,1.0];` in
`render_gl/card.rs:101`, `flower.rs:170`, `marker.rs:84`, `tile.rs:118`;
`let indices: [u16; 6] = [0,1,2,2,1,3];` in `card.rs:107`, `flower.rs:176`,
`marker.rs:89`; `create_vertex_array()` in all five `build_*_program`
(`card.rs:87`, `flower.rs:153`, `line.rs:79`, `marker.rs:71`, `tile.rs:101`);
the `buffer_data_with_array_buffer_view(ARRAY_BUFFER, &view, STREAM_DRAW)`
upload at five sites in `render_gl/mod.rs` (`:390,423,452,528,555`).
→ A `build_unit_quad_vao(gl)` + `upload_stream(gl, target, scratch)` in the
existing `render_gl/helpers.rs`.

**V8 — `publish_position` / `publish_pickup` structurally identical.**
Read: `net/state.rs:302-348` vs `:363-392`. Both: bump `_ATTEMPTED` → define wire
struct → `serde_json::to_vec` with `.map_err` bumping `_ERR` + `NetError::
ProviderInternal` → `provider.publish(&Topic(...))` → `match &result { Ok =>
_OK, Err => _ERR }` → return. Differ only in the wire struct, the topic const,
and which four `perf` atomics.
→ A private `publish_counted(topic, bytes, &PublishCounters)`.

### rave + the new `bevy-libp2p` branch (`vendor/laye/`)

**V9 — `laye` transport errors bypass rave's typed-at-cursor surface.**
Read: `rave/src/remote_players.rs::drain_net_events`. The `NetEvent::Error(err)`
arm (`:130`) routes through `error_log.emit(Severity::Error, "[net] {err:?}")`
— the in-app drawer only — while its sibling arms in the same function use the
sacred-error bus: decode failure `:102`, publish serialize `:52`, publish send
`:62` all call `error::emit_region`. And `observability::flush_typed_errors:143`
forwards only `crate::error::drain()` (the sacred bus) to `__raveErrorTyped`.
**Verified consequence:** laye's transport `NetError`s (`PublishFailed`,
`SubscribeFailed`, `NotConnected`, `InvalidTopic`, `ProviderInternal`) reach a
toggle-able drawer with lossy `{err:?}` formatting and never reach the cursor —
inconsistent with the file's own siblings and below what ERROR.md asks. This is
an *inconsistency*, not a swallow (the error is visible somewhere).
→ Route `NetEvent::Error` through `error::emit_region("net", …, reason)` too.

**V10 — two startup `.expect()` panics in the vendored plugin.**
Read: `vendor/laye/crates/bevy-libp2p/src/lib.rs` (`Plugin::build`):
`load_or_fresh(...).expect("laye-me identity load")` and
`laye_net::new(...).expect("laye-net new")`. A corrupt stored identity
(`load_or_fresh(Some(bad))` → `MeError::Decode`) or swarm-init failure **panics
at startup**. rave's panic hook catches it (so not invisible), but it's a hard
crash where the prior roam/rave identity code returned a typed error — and it's
in vendored code, so rave can't change the policy without wrapping construction.

**V11 — identity logic reimplemented in roam vs `laye-me` (resolves on migration).**
Read: `roam/src/identity.rs:166-191` (`load_or_generate_keypair` /
`generate_identity_protobuf`) vs `vendor/laye/crates/me/src/lib.rs`
(`load`/`fresh`/`load_or_fresh`/`to_bytes`). Same libp2p Ed25519 protobuf
load-or-generate logic; **different** error type (`NetError::ProviderInternal`
vs `MeError::Decode`) and laye splits it into smaller fns. So this is *not* a
byte-identical copy (the old report was wrong) — it's the same logic in two
places, where `laye-me` is the clean extraction. roam has **not** migrated
(it still has its own `net/` + `identity.rs` on `master`); rave consumes laye.
→ When roam adopts `laye-me`/`laye-net`, V8 and this duplication go away. The
`bevy-libp2p` branch is the first consumer; roam is the obvious second.

### cross-project

**V12 — the two relayer integration tests are ~70% identical.**
Read: `wc` + `git diff --no-index` between
`crates/rave-positions-test/tests/positions_via_relayer.rs` (258 lines) and
`chat_via_relayer.rs` (244) → only 74 lines differ (30+/44−). The harness
(`ClientBehaviour`, `build_client_swarm`, `pick_free_port`, relayer
spawn/mesh-wait) is shared; only the topic, payload, and asserts differ.
→ A `tests/common/mod.rs` harness. The project convention ("a sibling test per
gossipsub topic") means this keeps getting re-pasted otherwise.

**V13 — `universe/` deleted (meta).**
Verified by `git diff --stat c753770..origin/master`: the whole directory, its
`deploy-universe.yml`, and `infra/universe.tf` are removed. This is why the old
report's cross-project C1 (rave↔universe observability crate) and C8 (shared
Bevy profile) no longer apply — there is no second Bevy app.

---

## Downgraded / retracted

**`choice.rs` per-intent scoring — DOWNGRADED.** The earlier report called this
"high severity, the `OnAttachedAsCost` block copied 6× verbatim." Reading
`choice.rs:577-676` firsthand: the `let Some(cand) = … else { return 0 }` guard
*is* repeated (~7×), but the `OnAttachedAsCost` check is **not** a verbatim copy
— `target_score:598` tests the candidate's own handlers (+10) while
`steal_score:644` tests each *attached* card's handlers (+30). It's a shared
scoring *vocabulary* applied to different objects with different constants, in
deliberately distinct heuristics. Low value, judgment call — not the headline it
was made into.

**roam `NetEvent::SubscriptionChange` "drift" — RETRACTED.** Earlier I called
`roam/src/net/state.rs` "missing the sub_change arm — correctness risk." It is
**not** missing: `state.rs:243` is `NetEvent::SubscriptionChange { .. } => {}`,
an explicit no-op. `wasm_ffi.rs:783` emits a trace for the same event, but these
are different drain paths and a main-thread path choosing not to trace
subscription churn is defensible. Not a bug.

---

## Unverified leads (NOT findings — I did not read these firsthand)

Carried from the subagent pass; treat as candidates to confirm, not claims:
- ccg `sim/run.rs` Pattern-B cost-fill / tap-substitution repetition; `mcts.rs`
  vs `uct.rs` candidate preamble; `GameStats` initializer twice.
- ccg `game` cost-aggregation loop, sacrifice-validation, death-trigger blocks;
  the GY-anchor picker/resolver split flagged as a correctness risk.
- ccg `card/loader.rs` "sequence of lowercased strings" parse repeated; array-or-
  slot-map parse.
- ccg `wasm_ffi.rs` extern-wrapper macro opportunity; AI-parse 3×.
- ccg `error.rs`/`trace.rs` 14-field `Error { … }` literal at multiple sites.
- roam `perf.rs` per-counter restatement; render_gl per-frame uniform setting.
- `laye-net` swarm internals; rave↔roam JS bridge error-shape helper.
- `tsot-card` single-consumer / doc overclaim (the crate claims dual use; only
  roam depends on it per `grep` — but I have not re-read both Cargo.toml myself).
