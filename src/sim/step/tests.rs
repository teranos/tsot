    #![allow(deprecated)]
    use super::*;
    use crate::card::CardType;
    use crate::cast_routing::CastRouting;

    /// S1 scaffold sanity check. Builds a `StepEngine` over a vanilla
    /// 50-card mirror deck, asserts the cursor begins at `StartTurn`
    /// and the engine state hasn't advanced past turn 1 yet.
    #[test]
    fn step_engine_constructs_at_start_turn() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.kind.is_castable()
            })
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);

        let engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        assert!(
            matches!(engine.cursor, EngineCursor::StartTurn),
            "fresh engine should sit at StartTurn, got {:?}",
            engine.cursor
        );
        assert_eq!(engine.state.turn, 1, "fresh game is on turn 1");
        assert_eq!(
            engine.state.active_player,
            PlayerId::A,
            "side A acts first"
        );
    }

    /// S2 target: full vanilla game (Heuristic-vs-Heuristic, vanilla
    /// 50-card mirror) runs to completion via repeated `step(None)`
    /// calls. Asserts: terminates within a sane step budget, never
    /// yields `NeedHuman` (no humans in this game), produces a
    /// `Done(stats)` with a winner set.
    #[test]
    fn step_engine_completes_vanilla_heuristic_game() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        let mut steps = 0u32;
        let final_stats = loop {
            steps += 1;
            assert!(
                steps < 100_000,
                "step budget exceeded — engine isn't terminating (cursor: {:?})",
                engine.cursor
            );
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(prompt) => {
                    panic!("vanilla Heuristic game should never yield: {prompt:?}")
                }
                StepResult::Done(stats) => break stats,
            }
        };

        assert!(final_stats.turns > 0, "no turns played");
        assert!(
            matches!(engine.cursor, EngineCursor::GameOver),
            "post-Done cursor should be GameOver, got {:?}",
            engine.cursor
        );
    }

    /// S3: byte-for-byte parity vs `run_game_continue` on the same
    /// seed + same decks. If this passes, the step state machine is
    /// observably indistinguishable from the legacy runner for
    /// vanilla games — gives us a safety net for the bigger
    /// refactors (S7+ Lua handlers, S11 edge cases) coming next.
    ///
    /// S2 scope only covers Pattern B + combat, not activations
    /// (those land in S9). The template filter excludes any card
    /// with an `activated` block so `run_game_continue`'s activation
    /// pass and `StepEngine`'s missing pass don't diverge — once S9
    /// adds activation cursors, this filter can drop the
    /// `c.activated.is_empty()` clause.
    #[test]
    fn step_engine_parity_vs_run_game_continue() {
        use crate::game::Journal;
        use crate::sim::run::run_game_continue;
        use rand::SeedableRng;

        let seed: u64 = 0xBEEF;
        let registry_a = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let template = registry_a
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.activated.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .unwrap()
            .clone();
        let deck_a_cards: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b_cards = deck_a_cards.clone();

        // Path 1: legacy run_game_continue.
        let mut state1 = GameState::new(deck_a_cards.clone(), deck_b_cards.clone());
        state1.replay_journal = Some(Journal::new());
        let mut rng1 = StdRng::seed_from_u64(seed);
        let mut log1: Vec<String> = Vec::new();
        let ais1 = [AiKind::Heuristic, AiKind::Heuristic];
        let stats1 = run_game_continue(
            &mut state1,
            &mut rng1,
            &mut log1,
            &registry_a,
            &ais1,
        );

        // Path 2: StepEngine. Separate CardRegistry so the Lua VMs
        // can't influence each other (vanilla cards have no handlers
        // so this is belt-and-braces).
        let registry_b = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let mut state2 = GameState::new(deck_a_cards, deck_b_cards);
        state2.replay_journal = Some(Journal::new());
        let mut engine = StepEngine::new(
            state2,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry_b,
            seed,
        );
        let stats2 = engine.run_to_end();

        // Snapshot a few intermediate signals to localize divergence.
        eprintln!(
            "[parity] run_game_continue: winner={:?} turns={} a_played={} b_played={} a_attacks={} b_attacks={} a_milled={} b_milled={}",
            stats1.winner, stats1.turns, stats1.a_played, stats1.b_played,
            stats1.a_attacks, stats1.b_attacks, stats1.a_milled_to_exile, stats1.b_milled_to_exile,
        );
        eprintln!(
            "[parity] StepEngine        : winner={:?} turns={} a_played={} b_played={} a_attacks={} b_attacks={} a_milled={} b_milled={}",
            stats2.winner, stats2.turns, stats2.a_played, stats2.b_played,
            stats2.a_attacks, stats2.b_attacks, stats2.a_milled_to_exile, stats2.b_milled_to_exile,
        );

        assert_eq!(stats1.winner, stats2.winner, "winner differs");
        assert_eq!(stats1.turns, stats2.turns, "turn count differs");
        assert_eq!(stats1.a_played, stats2.a_played, "a_played differs");
        assert_eq!(stats1.b_played, stats2.b_played, "b_played differs");
        assert_eq!(stats1.a_attacks, stats2.a_attacks, "a_attacks differs");
        assert_eq!(stats1.b_attacks, stats2.b_attacks, "b_attacks differs");
        assert_eq!(stats1.a_deaths, stats2.a_deaths, "a_deaths differs");
        assert_eq!(stats1.b_deaths, stats2.b_deaths, "b_deaths differs");
        assert_eq!(stats1.a_final_board, stats2.a_final_board, "a_final_board differs");
        assert_eq!(stats1.b_final_board, stats2.b_final_board, "b_final_board differs");
        assert_eq!(stats1.a_final_gy, stats2.a_final_gy, "a_final_gy differs");
        assert_eq!(stats1.b_final_gy, stats2.b_final_gy, "b_final_gy differs");
        assert_eq!(
            stats1.a_milled_to_exile, stats2.a_milled_to_exile,
            "a_milled_to_exile differs"
        );
        assert_eq!(
            stats1.b_milled_to_exile, stats2.b_milled_to_exile,
            "b_milled_to_exile differs"
        );
        assert_eq!(
            stats1.a_played_card_ids, stats2.a_played_card_ids,
            "a_played_card_ids set differs"
        );
        assert_eq!(
            stats1.b_played_card_ids, stats2.b_played_card_ids,
            "b_played_card_ids set differs"
        );
    }

    /// Template + registry pair for the S4 human-dispatch tests:
    /// vanilla creature with `hand`/`mill`-only cost (no graveyard or
    /// X), no handlers, no activated abilities. Ensures the human
    /// side actually has playable candidates on turn 1.
    fn human_test_setup() -> (CardRegistry, crate::card::Card) {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.activated.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .unwrap()
            .clone();
        (registry, template)
    }

    /// S4: with `AiKind::Human` on side A, the engine yields a
    /// `NeedHuman(PickCard{…})` instead of dispatching the AI picker.
    /// The yielded prompt carries `player=A` and a non-empty
    /// `candidates` list (vanilla mirror deck → A always has hand
    /// cards to play on turn 1).
    #[test]
    fn step_engine_yields_pickcard_for_human_on_pattern_b() {
        use crate::sim::human::{HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        let prompt = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before any human prompt"),
            }
        };
        match *prompt {
            HumanPrompt::PickCard {
                player,
                ref candidates,
                ..
            } => {
                assert_eq!(player, PlayerId::A);
                assert!(!candidates.is_empty(), "vanilla deck should have playables");
            }
            ref other => panic!("expected PickCard, got {other:?}"),
        }
    }

    /// S4: human responds `Pass`, engine advances to `DeclareAttackers`
    /// without playing any cards. Hand size is unchanged.
    #[test]
    fn step_engine_human_pass_advances_to_combat() {
        use crate::sim::human::{HumanAction, HumanInterface};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // Drive to the first PickCard yield.
        loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(_) => break,
                StepResult::Done(_) => panic!("game ended early"),
            }
        }
        let hand_before = engine.state.a.hand.len();
        // Pass: no play, advance into combat (no creatures on board
        // so eventually we wrap through DeclareAttackers → EndTurn).
        match engine.step(Some(HumanAction::Pass)) {
            StepResult::Continue => {}
            other => panic!("expected Continue after Pass, got {other:?}"),
        }
        assert_eq!(
            engine.state.a.hand.len(),
            hand_before,
            "Pass should not consume hand cards"
        );
    }

    /// Pick a creature whose only cost component is an X-cost hand
    /// payment. Hydra fits today (`cost = {{is_x = true, source = "hand"}}`).
    /// Used by the S8 ChooseInt test so we can trigger
    /// `build_pattern_b_choices`'s X-pick yield from a human plays.
    fn x_cost_human_setup() -> (CardRegistry, crate::card::Card) {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.activated.is_empty()
                    && c.cost.len() == 1
                    && c.cost[0].is_x
                    && matches!(c.cost[0].source, crate::card::CostSource::Hand)
            })
            .expect("expected at least one vanilla X-cost-hand creature in the corpus")
            .clone();
        (registry, template)
    }

    /// S8: human plays an X-cost-hand creature. The first oracle call
    /// in `build_pattern_b_choices` is `choose_int` (X-pick). With the
    /// replay queue empty, the engine yields
    /// `NeedHuman(ChooseInt{…})`. Resuming with `ChoiceInt{value}`
    /// drives the X-pick, then the resolve falls through to the
    /// X*hand-payment yield (`ChooseCard`); resuming that too lands
    /// the creature on board.
    #[test]
    fn step_engine_human_x_cost_yields_choose_int_then_choose_card() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = x_cost_human_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // Drive to the first PickCard.
        let prompt = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickCard"),
            }
        };
        let to_play = match *prompt {
            HumanPrompt::PickCard { ref candidates, .. } => candidates[0].clone(),
            ref other => panic!("expected PickCard, got {other:?}"),
        };

        let board_before = engine.state.a.board.len();

        // PlayCard → engine starts resolving → X-cost card → choose_int yield.
        let int_prompt = match engine.step(Some(HumanAction::PlayCard {
            iid: to_play.clone(),
        })) {
            StepResult::NeedHuman(p) => p,
            other => panic!("expected NeedHuman(ChooseInt), got {other:?}"),
        };
        let (min, max) = match *int_prompt {
            HumanPrompt::ChooseInt { min, max, .. } => (min, max),
            ref other => panic!("expected ChooseInt, got {other:?}"),
        };
        assert!(min >= 1, "X-pick min should be ≥ 1, got {min}");
        assert!(max >= min, "X-pick max ≥ min, got {max} vs {min}");

        // Resume with X=1 — minimum payment. Build re-runs, choose_int
        // consumes our reply, then resolve_hand_payment fires once for
        // the single X-slot → choose_card yield.
        let card_prompt = match engine.step(Some(HumanAction::ChoiceInt { value: 1 })) {
            StepResult::NeedHuman(p) => p,
            other => panic!("expected NeedHuman(ChooseCard) after ChoiceInt, got {other:?}"),
        };
        let pool = match *card_prompt {
            HumanPrompt::ChooseCard { ref pool, .. } => pool.clone(),
            ref other => panic!("expected ChooseCard, got {other:?}"),
        };
        assert!(!pool.is_empty(), "X-cost payment pool should be non-empty");

        let payment = pool[0].clone();
        match engine.step(Some(HumanAction::ChoiceCard {
            iid: Some(payment.clone()),
        })) {
            StepResult::Continue => {}
            other => panic!("expected Continue after ChoiceCard, got {other:?}"),
        }

        assert!(
            engine.state.a.board.contains(&to_play),
            "X-cost cast should put the card on A's board"
        );
        assert_eq!(
            engine.state.a.board.len(),
            board_before + 1,
            "board should gain exactly one card"
        );
    }

    /// S7: human plays a 1H creature. `resolve_hand_payment` calls
    /// `oracle.choose_card`; replay queue is empty → engine yields
    /// `NeedHuman(ChooseCard{…})` instead of blocking. Replying with
    /// `ChoiceCard{iid}` resumes the resolve, `play_card` runs, and
    /// the picked card lands on A's board. A's hand drops by two:
    /// one for the card itself, one for the payment.
    #[test]
    fn step_engine_human_playcard_yields_choose_card_for_hand_payment() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        // Template must have a hand cost ≥ 1 for the yield to fire.
        assert!(
            template.cost.iter().any(|c| matches!(
                c.source,
                crate::card::CostSource::Hand
            ) && c.amount >= 1),
            "human_test_setup picked a card with no hand cost — the \
             ChooseCard yield won't fire on this card",
        );
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // Drive to the first PickCard prompt.
        let prompt = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickCard"),
            }
        };
        let to_play = match *prompt {
            HumanPrompt::PickCard { ref candidates, .. } => candidates[0].clone(),
            ref other => panic!("expected PickCard, got {other:?}"),
        };

        let hand_before = engine.state.a.hand.len();
        let board_before = engine.state.a.board.len();

        // Send PlayCard. Build runs, hits resolve_hand_payment, asks
        // `oracle.choose_card`, replay is empty → engine yields
        // ChooseCard back to us.
        let choose_prompt = match engine.step(Some(HumanAction::PlayCard {
            iid: to_play.clone(),
        })) {
            StepResult::Continue => panic!("expected NeedHuman(ChooseCard), got Continue"),
            StepResult::NeedHuman(p) => p,
            StepResult::Done(_) => panic!("game ended unexpectedly"),
        };
        let (pool, asker) = match *choose_prompt {
            HumanPrompt::ChooseCard {
                ref pool,
                asker,
                ..
            } => (pool.clone(), asker),
            ref other => panic!("expected ChooseCard, got {other:?}"),
        };
        assert_eq!(asker, PlayerId::A, "asker should be the human side");
        assert!(!pool.is_empty(), "hand-payment pool must be non-empty");

        // Pick the first eligible iid as the payment.
        let payment = pool[0].clone();

        // Resume: build re-runs with the replay queue [Card(Some(iid))].
        // resolve_hand_payment consumes it; build returns Choices.
        // play_card runs, card moves hand → board.
        match engine.step(Some(HumanAction::ChoiceCard {
            iid: Some(payment.clone()),
        })) {
            StepResult::Continue => {}
            StepResult::NeedHuman(p) => {
                panic!("expected Continue after ChoiceCard, got NeedHuman({p:?})")
            }
            StepResult::Done(_) => panic!("game ended unexpectedly"),
        }

        assert!(
            engine.state.a.board.contains(&to_play),
            "played iid should be on A's board, board={:?}",
            engine.state.a.board
        );
        assert!(
            !engine.state.a.hand.contains(&payment),
            "paid iid should have left A's hand"
        );
        assert_eq!(
            engine.state.a.hand.len(),
            hand_before - 2,
            "hand should drop by 2 (card cast + payment)"
        );
        assert_eq!(
            engine.state.a.board.len(),
            board_before + 1,
            "board should gain exactly the cast card"
        );
    }

    /// S5: with `AiKind::Human` on side A, after the Pattern B pass
    /// the engine yields `NeedHuman(PickAttackers{…})` instead of
    /// running `select_attackers`. Vanilla turn-1 board is empty, so
    /// `eligible` is `[]` — the prompt still fires so the human can
    /// confirm "no attacks".
    #[test]
    fn step_engine_yields_pickattackers_for_human() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // First yield is the Pattern B PickCard (S4). Pass through it.
        let _ = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickCard"),
            }
        };
        // Pass on Pattern B → cursor advances toward DeclareAttackers.
        match engine.step(Some(HumanAction::Pass)) {
            StepResult::Continue => {}
            other => panic!("expected Continue after Pass, got {other:?}"),
        }
        // Drive forward to the next yield — that should be PickAttackers.
        let prompt = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickAttackers"),
            }
        };
        match *prompt {
            HumanPrompt::PickAttackers {
                player,
                ref eligible,
                ..
            } => {
                assert_eq!(player, PlayerId::A);
                assert!(eligible.is_empty(), "turn-1 vanilla board: no creatures yet");
            }
            ref other => panic!("expected PickAttackers, got {other:?}"),
        }
    }

    /// S5: `Attackers{iids: vec![]}` resumes the engine into the
    /// end-of-turn cursor without declaring any attackers.
    #[test]
    fn step_engine_human_attackers_empty_advances_to_endturn() {
        use crate::sim::human::{HumanAction, HumanInterface};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // Drive past PickCard with Pass.
        loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(_) => break,
                StepResult::Done(_) => panic!("game ended early"),
            }
        }
        engine.step(Some(HumanAction::Pass));
        // Drive to PickAttackers.
        loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(_) => break,
                StepResult::Done(_) => panic!("game ended before PickAttackers"),
            }
        }
        // Empty attackers list → PostCombatActivations → (since the
        // active player is the human A) Main2Pick (S10). No EndTurn
        // until the human passes Main2.
        match engine.step(Some(HumanAction::Attackers { iids: vec![] })) {
            StepResult::Continue => {}
            other => panic!("expected Continue after Attackers, got {other:?}"),
        }
        assert!(
            matches!(engine.cursor, EngineCursor::PostCombatActivations),
            "post-Attackers cursor should be PostCombatActivations, got {:?}",
            engine.cursor
        );
        match engine.step(None) {
            StepResult::Continue => {}
            other => panic!("expected Continue from PostCombatActivations, got {other:?}"),
        }
        assert!(
            matches!(engine.cursor, EngineCursor::Main2Pick { .. }),
            "PostCombatActivations should advance into Main2Pick for human-active turn, got {:?}",
            engine.cursor
        );
        assert_eq!(engine.stats.a_attacks, 0, "no attacks bumped");
    }

    /// Drive the engine until a `NeedHuman(prompt)` matching `pick`
    /// fires. Any other NeedHuman (e.g. B's turn-N PickCard) gets a
    /// `Pass` response so the loop keeps making progress. Returns the
    /// matched prompt; panics if the step budget runs out.
    fn drive_to_prompt<F>(engine: &mut StepEngine, mut pick: F) -> Box<HumanPrompt>
    where
        F: FnMut(&HumanPrompt) -> bool,
    {
        use crate::sim::human::{HumanAction, HumanPrompt};
        let mut budget = 5_000u32;
        let mut pending: Option<HumanAction> = None;
        loop {
            budget = budget.checked_sub(1).expect("step budget exhausted");
            match engine.step(pending.take()) {
                StepResult::Continue => {}
                StepResult::Done(_) => panic!("game ended before matching prompt"),
                StepResult::NeedHuman(p) => {
                    if pick(&p) {
                        return p;
                    }
                    // Not the prompt we wanted: pass on PickCard, send
                    // empty Attackers for PickAttackers. Other variants
                    // (PickBlocks, ChooseCard, etc.) are unexpected
                    // inside the drive helper.
                    pending = Some(match *p {
                        HumanPrompt::PickCard { .. } => HumanAction::Pass,
                        HumanPrompt::PickAttackers { .. } => HumanAction::Attackers { iids: vec![] },
                        ref other => panic!(
                            "drive_to_prompt: unexpected intermediate prompt {other:?}"
                        ),
                    });
                }
            }
        }
    }

    /// S5: with `AiKind::Human` on side B (defender) and A=Heuristic
    /// (which Pattern-B-plays creatures that rig + attack via haste),
    /// the engine eventually yields `NeedHuman(PickBlocks{…})` against
    /// B. The `attackers` field of the prompt holds A's declared iids.
    #[test]
    fn step_engine_yields_pickblocks_for_human_defender() {
        use crate::sim::human::{HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Human(Arc::new(iface))],
            registry,
            0xCAFE,
        );

        let prompt = drive_to_prompt(&mut engine, |p| {
            matches!(p, HumanPrompt::PickBlocks { .. })
        });
        match *prompt {
            HumanPrompt::PickBlocks {
                defender,
                ref attackers,
                ..
            } => {
                assert_eq!(defender, PlayerId::B);
                assert!(
                    !attackers.is_empty(),
                    "PickBlocks prompt should carry the declared attackers"
                );
            }
            ref other => panic!("expected PickBlocks, got {other:?}"),
        }
    }

    /// S5: `Blocks{pairs: vec![]}` resumes the engine, runs
    /// `confirm_blocks` (which mills B's deck for the unblocked
    /// attacker), and transitions the cursor into `EndTurn`.
    #[test]
    fn step_engine_human_blocks_empty_advances_to_endturn() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Human(Arc::new(iface))],
            registry,
            0xCAFE,
        );

        let _ = drive_to_prompt(&mut engine, |p| {
            matches!(p, HumanPrompt::PickBlocks { .. })
        });
        let deck_b_before = engine.state.b.deck.len();
        match engine.step(Some(HumanAction::Blocks { pairs: vec![] })) {
            StepResult::Continue => {}
            other => panic!("expected Continue after Blocks, got {other:?}"),
        }
        assert!(
            matches!(engine.cursor, EngineCursor::PostCombatActivations),
            "post-Blocks cursor should be PostCombatActivations, got {:?}",
            engine.cursor
        );
        match engine.step(None) {
            StepResult::Continue => {}
            other => panic!("expected Continue from PostCombatActivations, got {other:?}"),
        }
        assert!(
            matches!(engine.cursor, EngineCursor::EndTurn),
            "PostCombatActivations should advance into EndTurn, got {:?}",
            engine.cursor
        );
        assert!(
            engine.state.b.deck.len() < deck_b_before,
            "unblocked attack should have milled B's deck"
        );
    }

    /// S10: a human-active turn yields a second `PickCard` prompt
    /// after combat — the Main2 main phase. Phase distinguishes it
    /// from the opening Pattern B PickCard: the `state.phase` field
    /// in the prompt is `"Main2"` for this one, `"Main1"` for the
    /// first one of the turn. Frontend uses the same Pass / PlayCard
    /// / Activate action set.
    #[test]
    fn step_engine_yields_main2_pickcard_for_human() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // First yield: Pattern B PickCard (Main1). Pass to enter combat.
        let first = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before Main1 PickCard"),
            }
        };
        match *first {
            HumanPrompt::PickCard { ref state, .. } => {
                assert_eq!(
                    state.phase, "Main1",
                    "first PickCard should be in Main1, got phase {:?}",
                    state.phase
                );
            }
            ref other => panic!("expected PickCard, got {other:?}"),
        }
        engine.step(Some(HumanAction::Pass));

        // Next yield: PickAttackers. Empty.
        let attackers = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickAttackers"),
            }
        };
        assert!(
            matches!(*attackers, HumanPrompt::PickAttackers { .. }),
            "expected PickAttackers, got {attackers:?}"
        );
        engine.step(Some(HumanAction::Attackers { iids: vec![] }));

        // S10: next yield is Main2 PickCard.
        let second = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before Main2 PickCard"),
            }
        };
        match *second {
            HumanPrompt::PickCard { ref state, .. } => {
                assert_eq!(
                    state.phase, "Main2",
                    "second PickCard should be in Main2, got phase {:?}",
                    state.phase
                );
            }
            ref other => panic!("expected PickCard, got {other:?}"),
        }
    }

    /// S11 scanner: probe seeds for one that triggers
    /// `preview_retry_rescued` under run_game_continue. The seed (or
    /// a list of seeds) found here becomes the fixture for the
    /// parity assertion above. `#[ignore]` so it doesn't run by
    /// default — invoke with `cargo test ... -- --include-ignored
    /// step_engine_finds_rescue_seed --nocapture` when hunting.
    #[test]
    #[ignore]
    fn step_engine_finds_rescue_seed() {
        use crate::game::Journal;
        use crate::sim::genome::to_deck;
        use crate::sim::run::run_game_continue;
        use rand::SeedableRng;

        let registry = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let pool_ids: Vec<String> = registry
            .cards()
            .iter()
            .filter(|c| {
                matches!(c.kind, CardType::Creature | CardType::Spell | CardType::Artifact)
            })
            .map(|c| c.id.clone())
            .collect();
        let deck_ids: Vec<String> =
            (0..50).map(|i| pool_ids[i % pool_ids.len()].clone()).collect();

        for seed in 0u64..64 {
            let deck_a = to_deck(registry.as_ref(), &deck_ids).unwrap();
            let deck_b = to_deck(registry.as_ref(), &deck_ids).unwrap();
            let mut state = GameState::new(deck_a, deck_b);
            state.replay_journal = Some(Journal::new());
            let mut rng = StdRng::seed_from_u64(seed);
            let mut log: Vec<String> = Vec::new();
            let ais = [AiKind::Heuristic, AiKind::Heuristic];
            let _stats = run_game_continue(&mut state, &mut rng, &mut log, &registry, &ais);
            let rescued = state
                .action_counts
                .get("preview_retry_rescued")
                .map(|v| v[0] + v[1])
                .unwrap_or(0);
            if rescued > 0 {
                eprintln!("[rescue-seed] seed={seed:#x} rescued={rescued}");
            }
        }
    }

    /// S11: AI-vs-AI parity on the suicide-rescue counter. Runs
    /// `run_game_continue` and `StepEngine::run_to_end` over the same
    /// full-corpus deck on the same seed; asserts they produce the
    /// same `action_counts["preview_retry_rescued"]` totals. The
    /// guarantee: even if no rescue fires for this seed (counter ==
    /// 0 on both sides), the assertion still pins them together —
    /// any future divergence in rescue behavior surfaces immediately.
    #[test]
    fn step_engine_matches_run_game_continue_preview_retry_rescued() {
        use crate::game::Journal;
        use crate::sim::genome::to_deck;
        use crate::sim::run::run_game_continue;
        use rand::SeedableRng;

        let seed: u64 = 0xD15EA5E;
        let registry_a = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        // Full-corpus random mix — 50 distinct ids that include the
        // choose_player carriers (field-notes, azure-recursion,
        // bci-megafly) so the recording can actually have a Player
        // entry to flip.
        let pool_ids: Vec<String> = registry_a
            .cards()
            .iter()
            .filter(|c| matches!(c.kind, CardType::Creature | CardType::Spell | CardType::Artifact))
            .map(|c| c.id.clone())
            .collect();
        let deck_ids: Vec<String> =
            (0..50).map(|i| pool_ids[i % pool_ids.len()].clone()).collect();

        // Path 1: legacy run_game_continue. Decks loaded from
        // registry_a's Lua VM.
        let deck_a_1 = to_deck(registry_a.as_ref(), &deck_ids).expect("deck A build");
        let deck_b_1 = to_deck(registry_a.as_ref(), &deck_ids).expect("deck B build");
        let mut state1 = GameState::new(deck_a_1, deck_b_1);
        state1.replay_journal = Some(Journal::new());
        let mut rng1 = StdRng::seed_from_u64(seed);
        let mut log1: Vec<String> = Vec::new();
        let ais1 = [AiKind::Heuristic, AiKind::Heuristic];
        let _stats1 = run_game_continue(&mut state1, &mut rng1, &mut log1, &registry_a, &ais1);

        // Path 2: StepEngine. Fresh registry so the StepEngine owns
        // its own Lua VM; rebuild the deck against THIS registry to
        // avoid mixing Lua functions across VMs.
        let registry_b = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let deck_a_2 = to_deck(&registry_b, &deck_ids).expect("deck A build (b)");
        let deck_b_2 = to_deck(&registry_b, &deck_ids).expect("deck B build (b)");
        let state2 = GameState::new(deck_a_2, deck_b_2);
        let mut engine = StepEngine::new(
            state2,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry_b,
            seed,
        );
        let _stats2 = engine.run_to_end();

        let rescued_1 = state1
            .action_counts
            .get("preview_retry_rescued")
            .map(|v| v[0] + v[1])
            .unwrap_or(0);
        let rescued_2 = engine
            .state
            .action_counts
            .get("preview_retry_rescued")
            .map(|v| v[0] + v[1])
            .unwrap_or(0);
        eprintln!(
            "[s11] preview_retry_rescued: run_game_continue={rescued_1}, StepEngine={rescued_2}"
        );
        assert_eq!(
            rescued_1, rescued_2,
            "preview_retry_rescued counter must match between paths"
        );
    }

    /// S9: AI-side activation pass fires for cards with activated
    /// abilities on the board. Uses blue-monkey (1H cost, 2H-pay →
    /// draw 1 ability). After a few turns the rig+haste path puts
    /// at least one blue-monkey on each side's board; with hand sizes
    /// at 6+ the AI auto-fires its activation, which calls
    /// `state.bump_action("activate", …)` (the key set by
    /// `state.activate_ability` on successful resolution).
    #[test]
    fn step_engine_runs_ai_activation_pass() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| c.id == "blue-monkey")
            .expect("blue-monkey present in corpus")
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry,
            0xCAFE,
        );
        let _ = engine.run_to_end();

        let total: u32 = engine
            .state
            .action_counts
            .get("activate")
            .map(|v| v[0] + v[1])
            .unwrap_or(0);
        assert!(
            total > 0,
            "AI activation pass should have fired at least once across the game (blue-monkey 2H-pay → draw 1); got total={total}"
        );
    }

    /// GameOver cursor → `Done` repeatedly, no panic. Verifies the
    /// only "real" branch in S1's step() doesn't accidentally
    /// regress when we extend the match in S2+.
    #[test]
    fn step_at_gameover_returns_done() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| matches!(c.kind, CardType::Creature) && c.handlers.is_empty())
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry,
            0xCAFE,
        );
        engine.cursor = EngineCursor::GameOver;

        match engine.step(None) {
            StepResult::Done(_) => {}
            other => panic!("expected Done at GameOver, got {other:?}"),
        }
        // Second call: still Done (idempotent terminal).
        match engine.step(None) {
            StepResult::Done(_) => {}
            other => panic!("expected Done on second call, got {other:?}"),
        }
    }
