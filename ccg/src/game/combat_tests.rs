use super::*;
use super::super::state::{PlayerId, StatusEffect};
use crate::game::test_helpers::*;

fn put_on_board(s: &mut GameState, side: PlayerId, iid: &InstanceId) {
    s.player_mut(side).hand.retain(|x| x != iid);
    s.player_mut(side).board.push(iid.clone());
}

fn add_ability(s: &mut GameState, iid: &InstanceId, ability: &str) {
    s.card_pool
        .get_mut(iid)
        .unwrap()
        .card_mut()
        .abilities
        .push(ability.to_string());
}

fn enter_combat(s: &mut GameState) {
    // From Untap, advance 3 phases to reach Combat.
    while s.phase != Phase::Combat {
        s.next_phase(None).expect("None ctx never yields");
    }
}

#[test]
fn combat_subsystem_round_trips_through_journal() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    let snapshot = format!("{:?}", s);
    s.journal = Some(crate::game::Journal::new());

    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    s.declare_blocker(&blk, &atk, None).unwrap();
    let _ = s.confirm_blocks(None).unwrap();

    assert_ne!(snapshot, format!("{:?}", s));
    let journal = s.journal.take().unwrap();
    journal.rollback(&mut s);
    assert!(s.journal.is_none());
    assert_eq!(
        snapshot,
        format!("{:?}", s),
        "combat subsystem rollback should restore prior state"
    );
}

#[test]
fn unblocked_attack_mills_defender_deck_to_exile() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    // Skip summoning sickness by giving haste, since the creature was just placed manually.
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    let defender_deck_before = s.b.deck.len();
    let defender_exile_before = s.b.exile.len();
    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    let outcome = s.confirm_blocks(None).unwrap();
    // deck_of(...) makes 1/1 cards, so attacker_x = 1.
    assert_eq!(outcome.defender_milled_to_exile, 1);
    assert_eq!(s.b.deck.len(), defender_deck_before - 1);
    assert_eq!(s.b.exile.len(), defender_exile_before + 1);
    assert!(outcome.deaths.is_empty());
}

#[test]
fn blocked_attack_exchanges_damage_both_die_on_equal_stats() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    s.declare_blocker(&blk, &atk, None).unwrap();
    let outcome = s.confirm_blocks(None).unwrap();
    // Both are 1/1 — each deals 1 to other, both reach damage >= y → die.
    assert_eq!(outcome.defender_milled_to_exile, 0);
    assert!(outcome.deaths.contains(&atk));
    assert!(outcome.deaths.contains(&blk));
    assert!(s.a.graveyard.contains(&atk));
    assert!(s.b.graveyard.contains(&blk));
}

#[test]
fn attacker_taps_on_declaration() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);
    s.declare_attacker(&atk, None).unwrap();
    assert!(s.card_pool.get(&atk).unwrap().tapped);
}

#[test]
fn vigilance_attacker_does_not_tap() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    add_ability(&mut s, &atk, "vigilance");
    enter_combat(&mut s);
    s.declare_attacker(&atk, None).unwrap();
    assert!(!s.card_pool.get(&atk).unwrap().tapped);
}

#[test]
fn defender_cannot_attack() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    add_ability(&mut s, &atk, "defender");
    enter_combat(&mut s);
    assert_eq!(
        s.declare_attacker(&atk, None),
        Err(CombatError::AttackerIsDefender)
    );
}

#[test]
fn summoning_sick_cannot_attack() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    s.card_pool.get_mut(&atk).unwrap().summoning_sick = true;
    enter_combat(&mut s);
    assert_eq!(
        s.declare_attacker(&atk, None),
        Err(CombatError::AttackerSummoningSick)
    );
}

#[test]
fn haste_overrides_summoning_sickness() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    s.card_pool.get_mut(&atk).unwrap().summoning_sick = true;
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);
    assert!(s.declare_attacker(&atk, None).is_ok());
}

#[test]
fn tapped_creature_cannot_attack() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    s.card_pool.get_mut(&atk).unwrap().tapped = true;
    enter_combat(&mut s);
    assert_eq!(s.declare_attacker(&atk, None), Err(CombatError::AttackerTapped));
}

#[test]
fn unblockable_attacker_refuses_blockers() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    add_ability(&mut s, &atk, "unblockable");
    enter_combat(&mut s);
    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    assert_eq!(
        s.declare_blocker(&blk, &atk, None),
        Err(CombatError::AttackerUnblockable)
    );
}

#[test]
fn flying_attacker_blocked_by_flyer_succeeds() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    add_ability(&mut s, &atk, "flying");
    add_ability(&mut s, &blk, "flying");
    enter_combat(&mut s);
    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    assert!(s.declare_blocker(&blk, &atk, None).is_ok());
}

#[test]
fn flying_attacker_can_be_blocked_by_subtype_override() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let bird = s.a.hand[0].clone();
    let cat = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &bird);
    put_on_board(&mut s, PlayerId::B, &cat);
    add_ability(&mut s, &bird, "haste");
    add_ability(&mut s, &bird, "flying");
    s.card_pool.get_mut(&bird).unwrap().card_mut().subtypes = vec!["bird".to_string()];
    s.card_pool.get_mut(&cat).unwrap().card_mut().can_block_subtypes = vec!["bird".to_string()];
    enter_combat(&mut s);
    s.declare_attacker(&bird, None).unwrap();
    s.confirm_attacks().unwrap();
    assert!(s.declare_blocker(&cat, &bird, None).is_ok());
}

#[test]
fn flying_attacker_can_be_blocked_by_reach() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    add_ability(&mut s, &atk, "flying");
    add_ability(&mut s, &blk, "reach");
    enter_combat(&mut s);
    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    assert!(s.declare_blocker(&blk, &atk, None).is_ok());
}

#[test]
fn flying_attacker_refuses_ground_blocker() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    add_ability(&mut s, &atk, "flying");
    enter_combat(&mut s);
    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    assert_eq!(
        s.declare_blocker(&blk, &atk, None),
        Err(CombatError::FlyingMustBeBlockedByFlyer)
    );
}

#[test]
fn blocker_with_cannot_block_subtype_is_rejected() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cat = s.a.hand[0].clone();
    let rat = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &cat);
    put_on_board(&mut s, PlayerId::B, &rat);
    add_ability(&mut s, &cat, "haste");
    s.card_pool.get_mut(&cat).unwrap().card_mut().subtypes = vec!["cat".to_string()];
    s.card_pool.get_mut(&rat).unwrap().card_mut().cannot_block_subtypes = vec!["cat".to_string()];
    enter_combat(&mut s);
    s.declare_attacker(&cat, None).unwrap();
    s.confirm_attacks().unwrap();
    assert_eq!(
        s.declare_blocker(&rat, &cat, None),
        Err(CombatError::BlockerCannotBlockSubtype)
    );
}

#[test]
fn blocker_without_cannot_block_subtype_can_still_block() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cat = s.a.hand[0].clone();
    let dog = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &cat);
    put_on_board(&mut s, PlayerId::B, &dog);
    add_ability(&mut s, &cat, "haste");
    s.card_pool.get_mut(&cat).unwrap().card_mut().subtypes = vec!["cat".to_string()];
    // dog has no cannot_block_subtypes restriction — should block fine.
    enter_combat(&mut s);
    s.declare_attacker(&cat, None).unwrap();
    s.confirm_attacks().unwrap();
    assert!(s.declare_blocker(&dog, &cat, None).is_ok());
}

#[test]
fn tapped_blocker_cannot_block() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    s.card_pool.get_mut(&blk).unwrap().tapped = true;
    enter_combat(&mut s);
    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    assert_eq!(
        s.declare_blocker(&blk, &atk, None),
        Err(CombatError::BlockerTapped)
    );
}

#[test]
fn attacker_outside_combat_phase_errors() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    // Still in Untap.
    assert_eq!(
        s.declare_attacker(&atk, None),
        Err(CombatError::NotCombatPhase)
    );
}

#[test]
fn battle_captain_untaps_other_attackers_on_attack() {
    use crate::card::CardRegistry;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let captain = registry
        .cards()
        .iter()
        .find(|c| c.id == "battle-captain")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cap_iid = s.a.hand[0].clone();
    let other_iid = s.a.hand[1].clone();
    {
        let inst = s.card_pool.get_mut(&cap_iid).unwrap();
        inst.card_mut().handlers = captain.handlers.clone();
        inst.card_mut().id = captain.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &cap_iid);
    put_on_board(&mut s, PlayerId::A, &other_iid);
    add_ability(&mut s, &cap_iid, "haste");
    add_ability(&mut s, &other_iid, "haste");
    enter_combat(&mut s);

    // Other creature attacks first; it taps.
    s.declare_attacker(&other_iid, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();
    assert!(s.card_pool.get(&other_iid).unwrap().tapped);

    // Captain attacks; its handler untaps the other attacker.
    s.declare_attacker(&cap_iid, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();
    assert!(s.card_pool.get(&cap_iid).unwrap().tapped); // captain itself stays tapped
    assert!(!s.card_pool.get(&other_iid).unwrap().tapped);
}

#[test]
fn game_card_exposes_id_type_subtypes_stats_tapped() {
    // Fixture: an on_attack handler that captures fields from game.card() on a
    // creature it knows about, dumping them to Lua globals for inspection.
    let registry = registry_with_fixture(
        "game_card_probe",
        r#"return {
            id = "card-probe",
            on_attack = function(game, self)
                local c = game.card(self.instance_id)
                _G.probe_id = c.id
                _G.probe_type = c.type
                _G.probe_first_subtype = c.subtypes[1]
                _G.probe_x = c.x
                _G.probe_y = c.y
                _G.probe_tapped = c.tapped
                _G.probe_owner = c.owner
            end,
        }"#,
    );
    let probe = registry
        .cards()
        .iter()
        .find(|c| c.id == "card-probe")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = probe.handlers.clone();
        inst.card_mut().id = probe.id.clone();
        inst.card_mut().subtypes = vec!["human".to_string()];
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    let globals = registry.lua().globals();
    let id: String = globals.get("probe_id").unwrap();
    let ty: String = globals.get("probe_type").unwrap();
    let sub: String = globals.get("probe_first_subtype").unwrap();
    let x: i32 = globals.get("probe_x").unwrap();
    let y: i32 = globals.get("probe_y").unwrap();
    let tapped: bool = globals.get("probe_tapped").unwrap();
    let owner: String = globals.get("probe_owner").unwrap();
    assert_eq!(id, "card-probe");
    assert_eq!(ty, "creature");
    assert_eq!(sub, "human");
    assert_eq!(x, 1);
    assert_eq!(y, 1);
    assert!(tapped, "attacker is tapped at on_attack fire time");
    assert_eq!(owner, "a");
}

#[test]
fn mortal_bee_attack_exiles_opponent_deck_and_self_taxes() {
    use crate::card::CardRegistry;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let bee = registry
        .cards()
        .iter()
        .find(|c| c.id == "mortal-bee")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = bee.handlers.clone();
        inst.card_mut().id = bee.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    let b_deck_before = s.b.deck.len();
    let b_exile_before = s.b.exile.len();
    let a_deck_before = s.a.deck.len();

    s.declare_attacker(&atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    // Opponent's deck shrinks by 1, exile grows by 1.
    assert_eq!(s.b.deck.len(), b_deck_before - 1);
    assert_eq!(s.b.exile.len(), b_exile_before + 1);
    // Owner's deck untouched.
    assert_eq!(s.a.deck.len(), a_deck_before);
    // SkipUntap(1) status on self.
    let bee_inst = s.card_pool.get(&atk).unwrap();
    assert!(bee_inst.tapped);
    assert!(matches!(
        bee_inst.status_effects.first(),
        Some(StatusEffect::SkipUntap(1))
    ));
}

#[test]
fn game_discard_moves_n_from_hand_to_graveyard() {
    // game.discard moves N cards from hand to graveyard. The smart-discard
    // heuristic in do_discard picks the highest-discard-score card per
    // slot; with this fixture's identical 1/1 vanilla hand cards, all
    // scores tie, so we only assert the count-level invariant here. See
    // the heuristic-targeted tests for the prioritization assertions.
    let registry = registry_with_fixture(
        "game_discard",
        r#"return {
            id = "discard-probe",
            on_attack = function(game, self)
                game.discard(self.owner, 2)
            end,
        }"#,
    );
    let probe = registry
        .cards()
        .iter()
        .find(|c| c.id == "discard-probe")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = probe.handlers.clone();
        inst.card_mut().id = probe.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    let hand_before = s.a.hand.len();
    let gy_before = s.a.graveyard.len();

    s.declare_attacker(&atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    assert_eq!(s.a.hand.len(), hand_before - 2);
    assert_eq!(s.a.graveyard.len(), gy_before + 2);
}

#[test]
fn smart_discard_prefers_vanilla_over_pitch_payoff_jewel() {
    // The smart-discard heuristic must NOT throw away a jewel (OnAttachedAsCost
    // handler) when a vanilla creature is available. Big negative score on
    // pitch-payoff handlers is the design call — jewels are tools.
    let registry = registry_with_fixture(
        "smart_discard",
        r#"return {
            id = "discard-probe",
            on_attack = function(game, self)
                game.discard(self.owner, 1)
            end,
        }"#,
    );
    let probe = registry
        .cards()
        .iter()
        .find(|c| c.id == "discard-probe")
        .unwrap()
        .clone();

    // Hand: [atk, jewel, vanilla, vanilla, vanilla]. After atk moves to
    // BOARD, the heuristic ranks the remainder. Jewel scores -52
    // (OnAttachedAsCost -50, stats -2); each vanilla scores -2. So one of
    // the vanillas should be discarded and the jewel must stay.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let jewel = s.a.hand[1].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = probe.handlers.clone();
        inst.card_mut().id = probe.id.clone();
    }
    // Give the jewel an OnAttachedAsCost handler so discard_score sees it.
    // Reuse the probe's on_attack Function (mlua::Function is a Lua reference,
    // cheap to clone). Body is irrelevant — discard_score only checks key
    // presence in the handlers map.
    let probe_handler = probe
        .handlers
        .get(&crate::card::EventName::OnAttack)
        .unwrap()
        .clone();
    s.card_pool
        .get_mut(&jewel)
        .unwrap()
        .card_mut()
        .handlers
        .insert(crate::card::EventName::OnAttachedAsCost, probe_handler);
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    let hand_before = s.a.hand.len();
    s.declare_attacker(&atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    // Jewel must still be in hand; exactly one card was discarded.
    assert!(s.a.hand.contains(&jewel), "jewel must not be discarded");
    assert_eq!(s.a.hand.len(), hand_before - 1);
    assert_eq!(s.a.graveyard.len(), 1);
    let discarded = s.a.graveyard[0].clone();
    assert_ne!(discarded, jewel, "discarded card must not be the jewel");
}

#[test]
fn game_print_handler_call_does_not_error() {
    // Smoke test only: calling game.print from a handler returns Ok and
    // the fire_self_only path completes normally. stderr capture isn't
    // worth the test scaffolding for a debug primitive.
    let registry = registry_with_fixture(
        "game_print",
        r#"return {
            id = "print-probe",
            on_attack = function(game, self)
                game.print("hello from " .. self.instance_id)
                _G.print_probe_ran = true
            end,
        }"#,
    );
    let probe = registry
        .cards()
        .iter()
        .find(|c| c.id == "print-probe")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = probe.handlers.clone();
        inst.card_mut().id = probe.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    let ran: bool = registry.lua().globals().get("print_probe_ran").unwrap();
    assert!(ran);
}

#[test]
fn handler_mutations_round_trip_through_journal() {
    // A fixture handler that calls many game.* methods at once, so the
    // round-trip exercises do_damage / do_mill / do_draw / do_set_tapped /
    // do_add_status / do_discard / do_move / bump_action / bump_event_fire.
    let registry = registry_with_fixture(
        "round_trip",
        r#"return {
            id = "round-trip-probe",
            on_attack = function(game, self)
                local opp = game.opponent(self.owner)
                game.draw(self.owner, 1)
                game.mill(opp, 2, "exile")
                game.damage(self.instance_id, 1)
                game.add_status(self.instance_id, "skip_untap", 2)
                game.tap(self.instance_id)
                game.discard(self.owner, 1)
            end,
        }"#,
    );
    let probe = registry
        .cards()
        .iter()
        .find(|c| c.id == "round-trip-probe")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = probe.handlers.clone();
        inst.card_mut().id = probe.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    let snapshot = format!("{:?}", s);
    s.journal = Some(crate::game::Journal::new());

    s.declare_attacker(
        &atk,
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();

    assert_ne!(snapshot, format!("{:?}", s));
    let journal = s.journal.take().unwrap();
    journal.rollback(&mut s);
    assert!(s.journal.is_none());
    assert_eq!(
        snapshot,
        format!("{:?}", s),
        "handler-driven mutations should round-trip through the journal"
    );
}

fn registry_with_fixture(name: &str, source: &str) -> crate::card::CardRegistry {
    let tmp = std::env::temp_dir().join(format!("tsot_fixture_{name}"));
    std::fs::create_dir_all(&tmp).unwrap();
    // Clean any stale fixture from a prior run.
    if let Ok(rd) = std::fs::read_dir(&tmp) {
        for entry in rd.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let path = tmp.join(format!("{name}.lua"));
    std::fs::write(&path, source).unwrap();
    crate::card::CardRegistry::load(&tmp).unwrap()
}

#[test]
fn on_attack_handler_fires_when_attacker_declared() {
    let registry = registry_with_fixture(
        "on_attack",
        r#"return {
            id = "fire-on-attack",
            on_attack = function(game, self)
                _G.fire_on_attack_count = (_G.fire_on_attack_count or 0) + 1
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "fire-on-attack")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    registry
        .lua()
        .globals()
        .set("fire_on_attack_count", 0_i32)
        .unwrap();
    s.declare_attacker(&atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    let count: i32 = registry
        .lua()
        .globals()
        .get("fire_on_attack_count")
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(s.event_fires[&crate::card::EventName::OnAttack], [1, 0]);
}

#[test]
fn on_block_handler_fires_when_blocker_declared() {
    let registry = registry_with_fixture(
        "on_block",
        r#"return {
            id = "fire-on-block-side",
            on_block = function(game, self, attacker)
                _G.fire_on_block_side_count = (_G.fire_on_block_side_count or 0) + 1
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "fire-on-block-side")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    // Handler goes on the BLOCKER, not the attacker.
    {
        let inst = s.card_pool.get_mut(&blk).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    registry
        .lua()
        .globals()
        .set("fire_on_block_side_count", 0_i32)
        .unwrap();
    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    s.declare_blocker(&blk, &atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    let count: i32 = registry
        .lua()
        .globals()
        .get("fire_on_block_side_count")
        .unwrap();
    assert_eq!(count, 1);
    // Owner of blocker is B → credited to B.
    assert_eq!(s.event_fires[&crate::card::EventName::OnBlock], [0, 1]);
}

#[test]
fn midnight_raven_attack_moves_top_of_deck_to_bottom() {
    use crate::card::CardRegistry;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let raven = registry
        .cards()
        .iter()
        .find(|c| c.id == "midnight-raven")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = raven.handlers.clone();
        inst.card_mut().id = raven.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    let top_before = s.a.deck[0].clone();
    let bottom_before = s.a.deck.last().unwrap().clone();
    let deck_len = s.a.deck.len();

    s.declare_attacker(&atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    // Top card moved to bottom; deck length unchanged.
    assert_eq!(s.a.deck.len(), deck_len);
    assert_eq!(s.a.deck.last().unwrap(), &top_before);
    // The card that *was* on the bottom is now one above the bottom.
    assert_eq!(s.a.deck[deck_len - 2], bottom_before);
    assert_eq!(s.event_fires[&crate::card::EventName::OnAttack], [1, 0]);
}

#[test]
fn thorn_beetle_on_block_damages_attacker() {
    use crate::card::CardRegistry;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let beetle = registry
        .cards()
        .iter()
        .find(|c| c.id == "thorn-beetle")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&blk).unwrap();
        inst.card_mut().handlers = beetle.handlers.clone();
        inst.card_mut().id = beetle.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    s.declare_blocker(&blk, &atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    // Handler pinged the attacker for 1.
    assert_eq!(s.card_pool.get(&atk).unwrap().damage, 1.0);
    assert_eq!(s.event_fires[&crate::card::EventName::OnBlock], [0, 1]);
}

#[test]
fn on_blocked_by_handler_fires_when_block_declared() {
    use crate::card::CardRegistry;
    use std::fs;

    // Write a fixture card whose on_blocked_by handler sets a Lua global,
    // so we can observe the fire from the host side.
    let tmp = std::env::temp_dir().join("tsot_on_blocked_by_test");
    fs::create_dir_all(&tmp).unwrap();
    let card_path = tmp.join("fire-on-block.lua");
    fs::write(
        &card_path,
        r#"return {
            id = "fire-on-block",
            on_blocked_by = function(game, self, blocker)
                _G.fire_on_block_count = (_G.fire_on_block_count or 0) + 1
            end,
        }"#,
    )
    .unwrap();

    let registry = CardRegistry::load(&tmp).unwrap();
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "fire-on-block")
        .unwrap()
        .clone();

    // Build a game where the fixture attacks; any vanilla creature blocks.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    // Swap the attacker's card data for the fixture (keep stats so combat math works).
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    s.declare_blocker(&blk, &atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    let count: i32 = registry
        .lua()
        .globals()
        .get("fire_on_block_count")
        .unwrap();
    assert_eq!(count, 1);

    fs::remove_file(&card_path).ok();
}

#[test]
fn tantrum_imp_handler_damages_blocker_and_mills_defender() {
    use crate::card::CardRegistry;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let tantrum = registry
        .cards()
        .iter()
        .find(|c| c.id == "tantrum-imp")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    // Replace attacker's card data with tantrum-imp's (handler + id),
    // keep the 1/1 stats so combat math stays predictable.
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = tantrum.handlers.clone();
        inst.card_mut().id = tantrum.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();

    let defender_deck_before = s.b.deck.len();
    let defender_exile_before = s.b.exile.len();

    s.declare_blocker(&blk, &atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    // Handler ran during declare_blocker (before resolve_combat):
    // blocker took 1 damage; defender's deck top went to exile.
    assert_eq!(
        s.card_pool.get(&blk).unwrap().damage,
        1.0,
        "blocker should have 1 damage from handler"
    );
    assert_eq!(s.b.deck.len(), defender_deck_before - 1);
    assert_eq!(s.b.exile.len(), defender_exile_before + 1);
}

#[test]
fn squirrel_overrun_handler_draws_a_card_when_blocked() {
    use crate::card::CardRegistry;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let squirrel = registry
        .cards()
        .iter()
        .find(|c| c.id == "squirrel-overrun")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    let blk = s.b.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = squirrel.handlers.clone();
        inst.card_mut().id = squirrel.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    put_on_board(&mut s, PlayerId::B, &blk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();

    let a_hand_before = s.a.hand.len();
    let a_deck_before = s.a.deck.len();

    s.declare_blocker(&blk, &atk, Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    assert_eq!(s.a.hand.len(), a_hand_before + 1);
    assert_eq!(s.a.deck.len(), a_deck_before - 1);
    assert_eq!(s.total_fires(PlayerId::A), 1);
    assert_eq!(s.total_fires(PlayerId::B), 0);
    assert_eq!(s.event_fires[&crate::card::EventName::OnBlockedBy], [1, 0]);
}

#[test]
fn trustworthy_lender_on_die_returns_attached_to_hand() {
    use crate::card::CardRegistry;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let lender = registry
        .cards()
        .iter()
        .find(|c| c.id == "trustworthy-lender")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let lender_iid = s.a.hand[0].clone();
    let attached_iid = s.a.hand[1].clone();
    let killer_iid = s.b.hand[0].clone();

    // Swap lender's card data in (keep stats so 1/1 vs 1/1 is mutual kill).
    {
        let inst = s.card_pool.get_mut(&lender_iid).unwrap();
        inst.card_mut().handlers = lender.handlers.clone();
        inst.card_mut().id = lender.id.clone();
    }

    put_on_board(&mut s, PlayerId::A, &lender_iid);
    put_on_board(&mut s, PlayerId::B, &killer_iid);
    // Attach the payment to lender (replicates what play_card would do).
    s.a.hand.retain(|x| x != &attached_iid);
    s.card_pool
        .get_mut(&lender_iid)
        .unwrap()
        .attached
        .push(attached_iid.clone());
    s.card_pool.get_mut(&attached_iid).unwrap().face_down = true;

    add_ability(&mut s, &lender_iid, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&lender_iid, None).unwrap();
    s.confirm_attacks().unwrap();
    s.declare_blocker(&killer_iid, &lender_iid, None).unwrap();
    let outcome = s.confirm_blocks(Some(&mut crate::game::EventContext::lua_only(registry.lua()))).unwrap();

    assert!(outcome.deaths.contains(&lender_iid));
    assert!(s.a.graveyard.contains(&lender_iid));

    // Handler returned attached to A's hand and flipped it face-up.
    assert!(s.a.hand.contains(&attached_iid));
    assert!(!s
        .card_pool
        .get(&lender_iid)
        .unwrap()
        .attached
        .contains(&attached_iid));
    assert!(!s.card_pool.get(&attached_iid).unwrap().face_down);
    assert_eq!(s.total_fires(PlayerId::A), 1);
    assert_eq!(s.event_fires[&crate::card::EventName::OnDie], [1, 0]);
}

#[test]
fn unblocked_attack_can_cause_deckout_win() {
    // Defender has only 1 card left in deck; 1-power attack mills it → defender loses.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(6, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);
    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    let outcome = s.confirm_blocks(None).unwrap();
    assert_eq!(outcome.defender_milled_to_exile, 1);
    assert_eq!(s.winner, Some(PlayerId::A));
}

/// Mirror of the activate.rs / lua_api.rs propagation tests at the
/// combat boundary. Pins the contract that when an `on_attack` handler
/// calls `game.choose_card` against a HumanReplayOracle with no replay,
/// `declare_attacker` returns `Err(CombatError::ChoicePending(_))`
/// instead of silently swallowing the suspend.
#[test]
fn declare_attacker_returns_choice_pending_when_on_attack_yields() {
    use crate::card::EventName;
    use crate::choice::{ChoicePending, RandomOracle};
    use crate::game::context::EventContext;
    use crate::sim::human::HumanReplayOracle;
    use mlua::Lua;
    use rand::SeedableRng;

    let lua = Lua::new();
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let atk = s.a.hand[0].clone();
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    // Park a target on the opponent's board to give the handler a
    // pool element. The wrapper short-circuits to Pending before
    // reading it; the iid just has to exist for the Lua side.
    let target = s.b.hand[0].clone();
    put_on_board(&mut s, PlayerId::B, &target);

    let handler_src = format!(
        r#"return function(game, self)
             local picked = game.choose_card({{ "{target}" }}, {{ prompt = "test" }})
             if picked ~= nil then game.damage(picked, 1) end
           end"#
    );
    let handler: mlua::Function = lua.load(&handler_src).eval().unwrap();
    s.card_pool
        .get_mut(&atk)
        .unwrap()
        .card_mut()
        .handlers
        .insert(EventName::OnAttack, handler);

    let mut oracle = HumanReplayOracle::new(
        RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0)),
        Some(PlayerId::A),
    );

    enter_combat(&mut s);
    let result = {
        let mut ctx = EventContext::new(&lua, &mut oracle);
        s.declare_attacker(&atk, Some(&mut ctx))
    };

    match result {
        Err(CombatError::ChoicePending(ChoicePending::Card(req))) => {
            assert_eq!(req.asker, Some(PlayerId::A));
            assert!(!req.pool.is_empty());
        }
        other => panic!(
            "expected Err(CombatError::ChoicePending(Card)), got {other:?}"
        ),
    }
}

/// Mirrors the existing OnDealtDamageToPlayer attached-iteration contract:
/// when an attacker has cards in its `attached` list, every attached card's
/// `on_attack` handler must fire when the attacker is declared. TNF / VEGF
/// rely on this — both are mutations attached to a host whose on_attack
/// triggers on the host's attack declaration.
#[test]
fn on_attack_handler_fires_on_attached_cards() {
    let registry = registry_with_fixture(
        "on_attack_attached",
        r#"return {
            id = "fire-on-attack-attached",
            on_attack = function(game, self)
                _G.fire_on_attack_attached_count = (_G.fire_on_attack_attached_count or 0) + 1
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "fire-on-attack-attached")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = s.a.hand[0].clone();
    let mutation = s.a.hand[1].clone();
    // Install the on_attack handler on the mutation, leave the host
    // handler-less. With the fix, the mutation's handler fires when the
    // host attacks. Without the fix, the iteration over attacker.attached
    // is missing and the mutation's handler never runs.
    {
        let inst = s.card_pool.get_mut(&mutation).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &host);
    s.add_attached(&host, &mutation);
    add_ability(&mut s, &host, "haste");
    enter_combat(&mut s);

    registry
        .lua()
        .globals()
        .set("fire_on_attack_attached_count", 0_i32)
        .unwrap();
    s.declare_attacker(&host, Some(&mut crate::game::EventContext::lua_only(registry.lua())))
        .unwrap();

    let count: i32 = registry
        .lua()
        .globals()
        .get("fire_on_attack_attached_count")
        .unwrap();
    assert_eq!(
        count, 1,
        "attached card's on_attack handler must fire when host attacks"
    );
}

#[test]
fn on_tapped_fires_when_a_creature_attacks() {
    // Window Cleaner's trigger: a creature with no inherent tap fires
    // on_tapped the moment it taps by attacking.
    let registry = registry_with_fixture(
        "tap_probe",
        r#"return {
            id = "tap-probe",
            on_tapped = function(game, self)
                _G.on_tapped_fired = (_G.on_tapped_fired or 0) + 1
                _G.on_tapped_who = self.instance_id
            end,
        }"#,
    );
    let probe = registry.cards().iter().find(|c| c.id == "tap-probe").unwrap().clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let atk = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&atk).unwrap();
        inst.card_mut().handlers = probe.handlers.clone();
        inst.card_mut().id = probe.id.clone();
    }
    put_on_board(&mut s, PlayerId::A, &atk);
    add_ability(&mut s, &atk, "haste");
    enter_combat(&mut s);

    s.declare_attacker(&atk, Some(&mut crate::game::EventContext::lua_only(registry.lua())))
        .unwrap();

    let globals = registry.lua().globals();
    let fired: i32 = globals.get("on_tapped_fired").unwrap_or(0);
    let who: String = globals.get("on_tapped_who").unwrap_or_default();
    assert_eq!(fired, 1, "on_tapped fired once when the creature attacked");
    assert_eq!(who, atk, "on_tapped fired on the tapped attacker");
}
