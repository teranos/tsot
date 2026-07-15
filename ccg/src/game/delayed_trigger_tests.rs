//! Delayed-trigger registry (slice-11 follow-up) — behaviour tests.
//!
//! A handler schedules a future trigger via `game.schedule_next_turn`;
//! the turn loop fires it (as `OnDelayedTrigger`) at the start of the
//! scheduling player's NEXT turn, routed through the deferred-event
//! queue. Re-scheduling from inside `on_delayed_trigger` yields a
//! recurring trigger.

use super::*;
use crate::card::EventName;
use crate::choice::ScriptedOracle;
use crate::game::context::EventContext;
use crate::game::lua_api::fire_self_only;
use crate::game::test_helpers::*;

fn fixture_registry() -> crate::card::CardRegistry {
    let tmp = std::env::temp_dir().join("tsot_fixture_delayed_probe");
    std::fs::create_dir_all(&tmp).unwrap();
    if let Ok(rd) = std::fs::read_dir(&tmp) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    std::fs::write(
        tmp.join("delayed-probe.lua"),
        r#"return {
            id = "delayed-probe",
            on_enter_board = function(game, self)
                game.schedule_next_turn(self.instance_id)
            end,
            on_delayed_trigger = function(game, self)
                _G.delayed_fired = (_G.delayed_fired or 0) + 1
            end,
        }"#,
    )
    .unwrap();
    crate::card::CardRegistry::load(&tmp).unwrap()
}

/// Put a delayed-probe on A's board and return its iid.
fn probe_on_board(s: &mut GameState, registry: &crate::card::CardRegistry) -> InstanceId {
    let card = registry.cards().iter().find(|c| c.id == "delayed-probe").unwrap().clone();
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.card_mut().handlers = card.handlers.clone();
        inst.card_mut().id = card.id.clone();
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());
    iid
}

#[test]
fn schedule_next_turn_registers_a_delayed_trigger_for_the_owner() {
    let registry = fixture_registry();
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let iid = probe_on_board(&mut s, &registry);

    let mut oracle = ScriptedOracle::new(vec![]);
    fire_self_only(registry.lua(), &mut s, &mut oracle, EventName::OnEnterBoard, &iid)
        .expect("etb answers locally");

    assert_eq!(s.delayed_triggers.len(), 1, "one delayed trigger registered");
    let t = &s.delayed_triggers[0];
    assert_eq!(t.fire_for, PlayerId::A, "scheduled for the owner's next turn");
    assert_eq!(t.iid, iid, "targets the scheduling card");
}

#[test]
fn delayed_trigger_fires_at_the_scheduling_players_next_turn() {
    let registry = fixture_registry();
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let iid = probe_on_board(&mut s, &registry);
    s.delayed_triggers.push(crate::game::DelayedTrigger { fire_for: PlayerId::A, iid: iid.clone() });

    registry.lua().globals().set("delayed_fired", 0i32).unwrap();

    // Sitting at B's End → next_phase enters A's Untap, where A's delayed
    // trigger is due.
    s.active_player = PlayerId::B;
    s.phase = crate::game::Phase::End;
    let mut oracle = ScriptedOracle::new(vec![]);
    s.next_phase(Some(&mut EventContext::new(registry.lua(), &mut oracle)))
        .expect("phase advance");

    let fired: i32 = registry.lua().globals().get("delayed_fired").unwrap_or(0);
    assert_eq!(fired, 1, "the delayed trigger fired at A's next turn begin");
    assert!(s.delayed_triggers.is_empty(), "a fired trigger is one-shot — removed");
}

#[test]
fn delayed_trigger_does_not_fire_on_the_opponents_turn() {
    let registry = fixture_registry();
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let iid = probe_on_board(&mut s, &registry);
    s.delayed_triggers.push(crate::game::DelayedTrigger { fire_for: PlayerId::A, iid });

    registry.lua().globals().set("delayed_fired", 0i32).unwrap();

    // Sitting at A's End → next_phase enters B's Untap. A's trigger is not
    // due on B's turn.
    s.active_player = PlayerId::A;
    s.phase = crate::game::Phase::End;
    let mut oracle = ScriptedOracle::new(vec![]);
    s.next_phase(Some(&mut EventContext::new(registry.lua(), &mut oracle)))
        .expect("phase advance");

    let fired: i32 = registry.lua().globals().get("delayed_fired").unwrap_or(0);
    assert_eq!(fired, 0, "A's trigger did not fire on B's turn");
    assert_eq!(s.delayed_triggers.len(), 1, "the trigger stays queued for A's turn");
}

#[test]
fn premonition_draws_two_at_your_next_turn_from_the_graveyard() {
    // The shipped card that uses the registry: cast Premonition (it
    // resolves to the graveyard now), and the draw fires at the start of
    // your next turn — from the graveyard, where no on_turn_begin could
    // reach it.
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let card = registry.cards().iter().find(|c| c.id == "premonition").unwrap().clone();

    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    // Put Premonition in A's graveyard (as if it just resolved) and fire
    // its on_play to schedule the delayed draw.
    let prem = s.a.hand[0].clone();
    s.card_pool.get_mut(&prem).unwrap().content = Some(card);
    let _ = s.move_card(&prem, PlayerId::A, Zone::Hand, Zone::Graveyard);

    let mut oracle = ScriptedOracle::new(vec![]);
    fire_self_only(registry.lua(), &mut s, &mut oracle, EventName::OnPlay, &prem)
        .expect("on_play answers locally");
    assert_eq!(s.delayed_triggers.len(), 1, "Premonition scheduled its draw");

    let deck_before = s.a.deck.len();

    // Advance from B's End into A's next turn begin — the draw fires.
    s.active_player = PlayerId::B;
    s.phase = crate::game::Phase::End;
    s.next_phase(Some(&mut EventContext::new(registry.lua(), &mut oracle)))
        .expect("phase advance");

    assert_eq!(s.a.deck.len(), deck_before - 2, "Premonition drew two at A's next turn");
    assert!(s.delayed_triggers.is_empty(), "the one-shot draw is spent");
}
