//! OnZoneChange dispatch — behavioural tests.
//!
//! Contract:
//!   - Fires on EVERY zone transition (any card, any from → any to).
//!   - Broadcast to (a) the moving card itself + (b) every card on BOARD.
//!     No exclusion. Self-move triggers ("if you discard this card, draw
//!     two cards") require the moving card to observe its own transition.
//!   - Handler signature: `function(game, self, moving, from, to)` where
//!     `moving` is a partner-table (`{ instance_id, owner, controller,
//!     attached }`) and `from`/`to` are zone strings (`"board"`,
//!     `"graveyard"`, `"exile"`, `"hand"`, `"deck"`).

use super::*;
use crate::card::EventName;
use crate::game::test_helpers::*;

fn registry_with_fixture(name: &str, source: &str) -> crate::card::CardRegistry {
    let tmp = std::env::temp_dir().join(format!("tsot_fixture_{name}"));
    std::fs::create_dir_all(&tmp).unwrap();
    if let Ok(rd) = std::fs::read_dir(&tmp) {
        for entry in rd.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    std::fs::write(tmp.join(format!("{name}.lua")), source).unwrap();
    crate::card::CardRegistry::load(&tmp).unwrap()
}

/// A BOARD watcher observes another card's zone transition.
///
/// Setup: watcher on A's BOARD; a distinct card in A's HAND. Move that
/// distinct card HAND → GRAVEYARD. Watcher's handler must fire once,
/// receiving `moving.instance_id` = the mover, `from = "hand"`,
/// `to = "graveyard"`.
#[test]
fn on_zone_change_fires_on_board_watchers() {
    let registry = registry_with_fixture(
        "on_zone_change_watcher",
        r#"return {
            id = "zone-watcher",
            on_zone_change = function(game, self, moving, from, to)
                _G.zc_count = (_G.zc_count or 0) + 1
                _G.zc_from = from
                _G.zc_to = to
                _G.zc_moving_iid = moving.instance_id
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "zone-watcher")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let watcher = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &watcher);
    s.a.board.push(watcher.clone());
    {
        let inst = s.card_pool.get_mut(&watcher).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }

    let mover = s.a.hand[0].clone();

    let lua = registry.lua();
    lua.globals().set("zc_count", 0_i32).unwrap();
    lua.globals().set("zc_from", "").unwrap();
    lua.globals().set("zc_to", "").unwrap();
    lua.globals().set("zc_moving_iid", "").unwrap();

    let mut oracle = crate::choice::ScriptedOracle::new(vec![]);
    let mut ctx = crate::game::EventContext::new(lua, &mut oracle);
    s.move_card_or_emit(
        &mover,
        PlayerId::A,
        Zone::Hand,
        Zone::Graveyard,
        "test-watcher",
        Some(&mut ctx),
    )
    .expect("move HAND → GRAVEYARD");

    let count: i32 = lua.globals().get("zc_count").unwrap();
    let from: String = lua.globals().get("zc_from").unwrap();
    let to: String = lua.globals().get("zc_to").unwrap();
    let moving_iid: String = lua.globals().get("zc_moving_iid").unwrap();

    assert_eq!(count, 1, "watcher's on_zone_change fires exactly once");
    assert_eq!(from, "hand");
    assert_eq!(to, "graveyard");
    assert_eq!(moving_iid, mover);
    assert_eq!(s.event_fires[&EventName::OnZoneChange], [1, 0]);
}

/// The moving card receives its own OnZoneChange broadcast.
///
/// Enables "if you discard this card, draw two cards" — the discarded
/// card observes its own HAND → GRAVEYARD transition.
#[test]
fn on_zone_change_fires_on_the_moving_card_itself() {
    let registry = registry_with_fixture(
        "on_zone_change_self",
        r#"return {
            id = "self-observer",
            on_zone_change = function(game, self, moving, from, to)
                if moving.instance_id == self.instance_id then
                    _G.self_count = (_G.self_count or 0) + 1
                    _G.self_from = from
                    _G.self_to = to
                end
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "self-observer")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }

    let lua = registry.lua();
    lua.globals().set("self_count", 0_i32).unwrap();
    lua.globals().set("self_from", "").unwrap();
    lua.globals().set("self_to", "").unwrap();

    let mut oracle = crate::choice::ScriptedOracle::new(vec![]);
    let mut ctx = crate::game::EventContext::new(lua, &mut oracle);
    s.move_card_or_emit(
        &iid,
        PlayerId::A,
        Zone::Hand,
        Zone::Graveyard,
        "test-self-observer",
        Some(&mut ctx),
    )
    .expect("move HAND → GRAVEYARD");

    let count: i32 = lua.globals().get("self_count").unwrap();
    let from: String = lua.globals().get("self_from").unwrap();
    let to: String = lua.globals().get("self_to").unwrap();

    assert_eq!(count, 1, "moving card receives its own OnZoneChange");
    assert_eq!(from, "hand");
    assert_eq!(to, "graveyard");
}

/// Round 2a: the corpus Moth draws when a glow creature leaves the
/// board via ANY transition (here: BOARD → EXILE). Currently red —
/// moth's handler is `on_creature_dies` which only fires on
/// BOARD → GRAVEYARD. Green after retrofit to `on_zone_change`.
#[test]
fn moth_draws_when_glow_creature_moves_from_board_to_exile() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let moth = registry
        .cards()
        .iter()
        .find(|c| c.id == "lantern-moth")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    // Moth on A's board, with real card + handlers + face.
    let moth_iid = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &moth_iid);
    s.a.board.push(moth_iid.clone());
    {
        let inst = s.card_pool.get_mut(&moth_iid).unwrap();
        inst.card_mut().handlers = moth.handlers.clone();
        inst.card_mut().id = moth.id.clone();
        inst.card_mut().face = moth.face.clone();
    }

    // Another glow creature on A's board — fixture body, no handlers,
    // face=glow so the moth's filter matches.
    let glow_iid = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &glow_iid);
    s.a.board.push(glow_iid.clone());
    {
        let inst = s.card_pool.get_mut(&glow_iid).unwrap();
        inst.card_mut().face = vec!["glow".to_string()];
        inst.card_mut().handlers = std::collections::BTreeMap::new();
    }

    let hand_before = s.a.hand.len();

    let mut oracle = crate::choice::ScriptedOracle::new(vec![]);
    let mut ctx = crate::game::EventContext::new(registry.lua(), &mut oracle);
    s.move_card_or_emit(
        &glow_iid,
        PlayerId::A,
        Zone::Board,
        Zone::Exile,
        "test-moth-glow-exile",
        Some(&mut ctx),
    )
    .expect("board → exile");

    assert_eq!(
        s.a.hand.len(),
        hand_before + 1,
        "moth draws 1 when a glow creature leaves the board"
    );
}

/// Round 2b: end-to-end via Pale Flicker's ETB. Flicker calls
/// `game.move(target, "exile")` on a glow creature. Currently red —
/// `do_move` in lua_api.rs calls `state.move_card` directly, not the
/// broadcast-firing path, so the Moth never sees the exile. Green
/// after `do_move` routes zone-change dispatch.
#[test]
fn moth_draws_when_flicker_exiles_a_glow_creature() {
    use crate::choice::{ScriptedAnswer, ScriptedOracle};

    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let moth = registry
        .cards()
        .iter()
        .find(|c| c.id == "lantern-moth")
        .unwrap()
        .clone();
    let flicker = registry
        .cards()
        .iter()
        .find(|c| c.id == "pale-flicker")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    // Moth on A's board.
    let moth_iid = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &moth_iid);
    s.a.board.push(moth_iid.clone());
    {
        let inst = s.card_pool.get_mut(&moth_iid).unwrap();
        inst.card_mut().handlers = moth.handlers.clone();
        inst.card_mut().id = moth.id.clone();
        inst.card_mut().face = moth.face.clone();
    }

    // Another glow creature on A's board — Flicker's ETB target.
    let target_iid = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &target_iid);
    s.a.board.push(target_iid.clone());
    {
        let inst = s.card_pool.get_mut(&target_iid).unwrap();
        inst.card_mut().face = vec!["glow".to_string()];
        inst.card_mut().handlers = std::collections::BTreeMap::new();
    }

    // Flicker starts in HAND; the HAND → BOARD move is what fires
    // on_zone_change (the ETB successor) on it, and Flicker's handler
    // reads its filter and exiles the glow target.
    let flicker_iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&flicker_iid).unwrap();
        inst.card_mut().handlers = flicker.handlers.clone();
        inst.card_mut().id = flicker.id.clone();
        inst.card_mut().face = flicker.face.clone();
    }

    // hand_before EXCLUDES the flicker (about to be moved out of hand);
    // the moth's draw will bring one card back into hand, so hand_after
    // should equal hand_before.
    let hand_before = s.a.hand.len() - 1;

    // Script the flicker-target choice.
    let mut oracle =
        ScriptedOracle::new(vec![ScriptedAnswer::Card(Some(target_iid.clone()))]);
    let mut ctx = crate::game::EventContext::new(registry.lua(), &mut oracle);
    s.move_card_or_emit(
        &flicker_iid,
        PlayerId::A,
        Zone::Hand,
        Zone::Board,
        "flicker-etb",
        Some(&mut ctx),
    )
    .expect("flicker Hand → Board");

    assert!(
        s.a.exile.contains(&target_iid),
        "flicker exiled the glow target"
    );
    assert_eq!(
        s.a.hand.len(),
        hand_before + 1,
        "moth draws 1 when flicker exiles a glow creature via game.move"
    );
}

/// Cycle 3: Turritopsis Dohrnii reanimates itself when milled via
/// `game.mill`. Currently red — `do_mill` in lua_api.rs calls
/// `state.move_card` directly, no on_zone_change broadcast. Green after
/// `do_mill` routes through the broadcast.
#[test]
fn turritopsis_returns_to_board_when_milled_from_deck() {
    use crate::choice::ScriptedOracle;

    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let turritopsis = registry
        .cards()
        .iter()
        .find(|c| c.id == "turritopsis-dohrnii")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    // Turritopsis on top of A's deck. Reuse an existing hand iid,
    // swap its identity to Turritopsis, and move it to deck top.
    let turr_iid = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &turr_iid);
    s.a.deck.insert(0, turr_iid.clone());
    {
        let inst = s.card_pool.get_mut(&turr_iid).unwrap();
        inst.card_mut().handlers = turritopsis.handlers.clone();
        inst.card_mut().id = turritopsis.id.clone();
        inst.card_mut().face = turritopsis.face.clone();
        inst.card_mut().colors = turritopsis.colors.clone();
        inst.card_mut().subtypes = turritopsis.subtypes.clone();
    }

    // A fixture "miller" — its on_zone_change handler (filtered to
    // self ETB) calls game.mill on A. Driving a Hand → Board move on
    // the miller triggers the mill — exercising the game.mill path
    // end-to-end (mill then broadcasts on_zone_change on the milled
    // card, which is what Turritopsis observes).
    let miller_iid = s.a.hand[0].clone();
    {
        let handler: mlua::Function = registry
            .lua()
            .load(
                r#"return function(game, self, moving, from, to)
                    if moving.instance_id ~= self.instance_id then return end
                    if to ~= "board" then return end
                    if game.host_of(self.instance_id) then return end
                    game.mill(self.owner, 1, "graveyard")
                end"#,
            )
            .eval()
            .unwrap();
        let inst = s.card_pool.get_mut(&miller_iid).unwrap();
        inst.card_mut()
            .handlers
            .insert(EventName::OnZoneChange, handler);
    }

    let mut oracle = ScriptedOracle::new(vec![]);
    let mut ctx = crate::game::EventContext::new(registry.lua(), &mut oracle);
    s.move_card_or_emit(
        &miller_iid,
        PlayerId::A,
        Zone::Hand,
        Zone::Board,
        "miller-etb",
        Some(&mut ctx),
    )
    .expect("miller Hand → Board");

    assert!(
        s.a.board.contains(&turr_iid),
        "turritopsis returns to board when milled from deck"
    );
    assert!(
        !s.a.graveyard.contains(&turr_iid),
        "turritopsis left the graveyard"
    );
}

/// Fold check (cycle 4). A raw `move_card_or_emit` call from engine code
/// must broadcast `on_zone_change` when the caller supplies an
/// `EventContext`. This is the invariant that lets the 23 engine ✗ sites
/// (payment, death, sacrifice, discard, draw, combat-mill, ...) fire
/// zone-change without each pairing a manual `broadcast_zone_change`
/// next to the move.
///
/// Fails today: `move_card_or_emit` has no ctx parameter and never
/// broadcasts. Green after `move_card_or_emit` grows an
/// `Option<&mut EventContext<'_>>` argument that, when `Some`, routes
/// through `broadcast_zone_change` after a successful move.
#[test]
fn move_card_or_emit_fires_on_zone_change_when_ctx_supplied() {
    let registry = registry_with_fixture(
        "move_or_emit_fold",
        r#"return {
            id = "fold-watcher",
            on_zone_change = function(game, self, moving, from, to)
                _G.fold_count = (_G.fold_count or 0) + 1
                _G.fold_from = from
                _G.fold_to = to
                _G.fold_moving_iid = moving.instance_id
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "fold-watcher")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let watcher = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &watcher);
    s.a.board.push(watcher.clone());
    {
        let inst = s.card_pool.get_mut(&watcher).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }

    let mover = s.a.hand[0].clone();

    let lua = registry.lua();
    lua.globals().set("fold_count", 0_i32).unwrap();
    lua.globals().set("fold_from", "").unwrap();
    lua.globals().set("fold_to", "").unwrap();
    lua.globals().set("fold_moving_iid", "").unwrap();

    // Post-fold: this call site takes `Some(&mut ctx)` as the 6th arg.
    // Every ✗ engine site updates identically — pass through whatever
    // ctx is already in scope (usually `ctx.as_deref_mut()` from an
    // `Option<&mut EventContext>` parameter).
    let mut oracle = crate::choice::ScriptedOracle::new(vec![]);
    let mut ctx = crate::game::EventContext::new(lua, &mut oracle);
    s.move_card_or_emit(
        &mover,
        PlayerId::A,
        Zone::Hand,
        Zone::Graveyard,
        "test-fold-check",
        Some(&mut ctx),
    )
    .expect("move HAND → GRAVEYARD");

    let count: i32 = lua.globals().get("fold_count").unwrap();
    let from: String = lua.globals().get("fold_from").unwrap();
    let to: String = lua.globals().get("fold_to").unwrap();
    let moving_iid: String = lua.globals().get("fold_moving_iid").unwrap();

    assert_eq!(
        count, 1,
        "move_card_or_emit with ctx supplied must broadcast on_zone_change"
    );
    assert_eq!(from, "hand");
    assert_eq!(to, "graveyard");
    assert_eq!(moving_iid, mover);
}

/// Attach coverage. A cardless sleeve in HAND attaches to a host on BOARD
/// (via `attach_cardless_from_hand` — Angry Glassblower's on-attack). Per
/// user ontology, attached sleeves sit on BOARD, so the transition is
/// HAND → BOARD and `on_zone_change` watchers must observe it.
#[test]
fn attach_from_hand_fires_on_zone_change_hand_to_board() {
    let registry = registry_with_fixture(
        "attach_watcher",
        r#"return {
            id = "attach-watcher",
            on_zone_change = function(game, self, moving, from, to)
                _G.aw_count = (_G.aw_count or 0) + 1
                _G.aw_from = from
                _G.aw_to = to
                _G.aw_moving_iid = moving.instance_id
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "attach-watcher")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    let watcher = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &watcher);
    s.a.board.push(watcher.clone());
    {
        let inst = s.card_pool.get_mut(&watcher).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }

    let host = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &host);
    s.a.board.push(host.clone());

    let payment = s.a.hand[0].clone();
    s.card_pool.get_mut(&payment).unwrap().content = None;

    let lua = registry.lua();
    lua.globals().set("aw_count", 0_i32).unwrap();
    lua.globals().set("aw_from", "").unwrap();
    lua.globals().set("aw_to", "").unwrap();
    lua.globals().set("aw_moving_iid", "").unwrap();

    let mut oracle = crate::choice::ScriptedOracle::new(vec![]);
    let mut ctx = crate::game::EventContext::new(lua, &mut oracle);
    let n = s.attach_cardless_from_hand(&host, PlayerId::A, 1, Some(&mut ctx));
    assert_eq!(n, 1);

    let count: i32 = lua.globals().get("aw_count").unwrap();
    let from: String = lua.globals().get("aw_from").unwrap();
    let to: String = lua.globals().get("aw_to").unwrap();
    let moving_iid: String = lua.globals().get("aw_moving_iid").unwrap();

    assert_eq!(count, 1, "attach must broadcast on_zone_change");
    assert_eq!(from, "hand");
    assert_eq!(to, "board");
    assert_eq!(moving_iid, payment);
}

/// Detach coverage. P.8 cascade: when a host with attached cards is
/// destroyed, those attached cards go to their own owners' EXILE. Under
/// user ontology, attached sits on BOARD → so the cascade is BOARD → EXILE
/// per attached, and every on_zone_change watcher must observe each move.
#[test]
fn p8_cascade_fires_on_zone_change_board_to_exile() {
    let registry = registry_with_fixture(
        "p8_watcher",
        r#"return {
            id = "p8-watcher",
            on_zone_change = function(game, self, moving, from, to)
                _G.p8_count = (_G.p8_count or 0) + 1
                _G.p8_last_from = from
                _G.p8_last_to = to
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "p8-watcher")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    let watcher = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &watcher);
    s.a.board.push(watcher.clone());
    {
        let inst = s.card_pool.get_mut(&watcher).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }

    let host = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &host);
    s.a.board.push(host.clone());

    let att1 = s.a.hand[0].clone();
    let att2 = s.a.hand[1].clone();
    s.a.hand.retain(|x| x != &att1 && x != &att2);
    s.add_attached(&host, &att1);
    s.add_attached(&host, &att2);

    let lua = registry.lua();
    lua.globals().set("p8_count", 0_i32).unwrap();
    lua.globals().set("p8_last_from", "").unwrap();
    lua.globals().set("p8_last_to", "").unwrap();

    let mut oracle = crate::choice::ScriptedOracle::new(vec![]);
    let mut ctx = crate::game::EventContext::new(lua, &mut oracle);
    s.exile_remaining_attached(&host, Some(&mut ctx));

    let count: i32 = lua.globals().get("p8_count").unwrap();
    let from: String = lua.globals().get("p8_last_from").unwrap();
    let to: String = lua.globals().get("p8_last_to").unwrap();

    assert_eq!(count, 2, "one broadcast per attached in the cascade");
    assert_eq!(from, "board");
    assert_eq!(to, "exile");
}
