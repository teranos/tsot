//! `tsot serve` subcommand: HTTP shim for playing against the AI in a
//! browser. Engine runs on a dedicated thread (mlua's `Lua` is `!Send`,
//! so the VM has to be built and stay on that thread). The HTTP server
//! runs on the main thread and bridges between the browser and the
//! engine via [`crate::sim::human::HumanInterface`].
//!
//! Endpoints (all localhost-only by default):
//! - `GET /` — the play page HTML (embedded via `include_str!`)
//! - `GET /state` — current snapshot + pending prompt as JSON
//! - `POST /action` — submit a [`crate::sim::human::HumanAction`] as
//!   JSON body; returns the next state snapshot
//!
//! Single concurrent game per server instance. No login, no rooms.
//! Restart the process to start a fresh game.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use clap::Parser;
use rand::rngs::StdRng;
use rand::SeedableRng;

use tsot::card::{Card, CardRegistry};
use tsot::game::{GameState, Journal, PlayerId};

use crate::parse_u64_hex_or_dec;
use crate::sim::evolved_deck::EvolvedDeck;
use crate::sim::genome::{random_genome, to_deck};
use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
use crate::sim::mcts::MctsConfig;
use crate::sim::run::run_game_continue;
use crate::sim::AiKind;

const PLAY_HTML: &str = include_str!("../assets/play.html");

#[derive(Parser)]
pub struct ServeArgs {
    /// Listen port. Default 8080.
    #[arg(long, default_value_t = 8080)]
    pub port: u16,
    /// Which side you play. Default a (you go first).
    #[arg(long, default_value = "a")]
    pub side: String,
    /// Opponent AI. heuristic | mcts. Default mcts.
    #[arg(long, default_value = "mcts")]
    pub opponent: String,
    /// MCTS rollouts per candidate (only if --opponent=mcts).
    #[arg(long, default_value_t = 5)]
    pub rollouts_per_candidate: u32,
    /// MCTS max candidates (only if --opponent=mcts).
    #[arg(long, default_value_t = 10)]
    pub max_candidates: u32,
    /// MCTS search depth (only if --opponent=mcts). `1` = one-ply
    /// (default); `2` = adds one deeper MCTS pick per rollout.
    #[arg(long, default_value_t = 1)]
    pub mcts_depth: u32,
    /// UCT iterations per pick (only if --opponent=uct). 50 ≈
    /// one-ply MCTS finish budget; UCT measured to beat one-ply
    /// MCTS 100% at matched budget on mirror-match deck.
    #[arg(long, default_value_t = 50)]
    pub uct_iterations: u32,
    /// UCT exploration constant (only if --opponent=uct).
    #[arg(long, default_value_t = std::f64::consts::SQRT_2)]
    pub uct_c: f64,
    /// Your deck (EvolvedDeck JSON). If unset, picks a random baseline.
    #[arg(long)]
    pub deck: Option<PathBuf>,
    /// Opponent's deck (EvolvedDeck JSON). If unset, picks a random
    /// baseline (different from yours).
    #[arg(long)]
    pub opponent_deck: Option<PathBuf>,
    /// Master seed. If unset, defaults to a fresh value derived from
    /// the system clock — each session plays a different game.
    #[arg(long, value_parser = parse_u64_hex_or_dec)]
    pub seed: Option<u64>,
}

pub fn run_serve(
    _registry: &CardRegistry,
    playable_pool: &[Card],
    args: &ServeArgs,
) -> mlua::Result<()> {
    let your_side = match args.side.as_str() {
        "a" | "A" => PlayerId::A,
        "b" | "B" => PlayerId::B,
        other => {
            eprintln!("--side must be 'a' or 'b' (got {other:?})");
            std::process::exit(2);
        }
    };
    let seed = args.seed.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        #[allow(clippy::disallowed_methods)]
        let now = SystemTime::now();
        now.duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xCAFE_F00D)
    });
    eprintln!("[serve] seed = 0x{seed:016x}");
    let opponent_ai_template = match args.opponent.as_str() {
        "heuristic" => AiKind::Heuristic,
        "mcts" => AiKind::Mcts(MctsConfig {
            rollouts_per_candidate: args.rollouts_per_candidate,
            max_candidates: args.max_candidates,
            base_seed: seed.wrapping_add(0xCAFE_BABE),
            max_depth: args.mcts_depth,
        }),
        "uct" => AiKind::Uct(crate::sim::uct::UctConfig {
            iterations: args.uct_iterations,
            exploration_c: args.uct_c,
            base_seed: seed.wrapping_add(0xC0FF_EE_BA),
            max_candidates: args.max_candidates,
        }),
        other => {
            eprintln!("--opponent must be 'heuristic' | 'mcts' | 'uct' (got {other:?})");
            std::process::exit(2);
        }
    };

    // Resolve deck card-id lists in the main thread (no Lua needed for
    // string-only deck lists). Engine thread will rebuild Vec<Card>
    // from these via its own registry.
    //
    // When both sides fall back to a random baseline pick, avoid
    // duplicates: hand opp resolution the id list the player got so
    // it can skip that baseline.
    let mut rng = StdRng::seed_from_u64(seed);
    let your_ids = resolve_deck(args.deck.as_deref(), playable_pool, None, &mut rng);
    let opp_ids = resolve_deck(
        args.opponent_deck.as_deref(),
        playable_pool,
        if args.deck.is_none() { Some(&your_ids) } else { None },
        &mut rng,
    );

    // The engine's outer turn loop is timed (`TSOT_GAME_TIMEOUT_SECS`,
    // default 30s) so headless sims can't hang forever. That timer
    // counts time spent blocked on `action_rx.recv()` — i.e. time
    // waiting for the human to click. For an interactive session 30s
    // is absurd; bump the cap to ~12h so a single game can take as
    // long as the user wants. Only overrides if the user didn't set
    // the env var themselves.
    if std::env::var_os("TSOT_GAME_TIMEOUT_SECS").is_none() {
        std::env::set_var("TSOT_GAME_TIMEOUT_SECS", "43200");
    }

    eprintln!("[serve] you = side {:?}, opponent = {}", your_side, args.opponent);
    eprintln!("[serve] your deck: {} unique ids", count_unique(&your_ids));
    eprintln!("[serve] opp deck:  {} unique ids", count_unique(&opp_ids));

    // Build channel pair: engine sends prompts, frontend sends actions.
    let (iface, prompt_rx, action_tx) = HumanInterface::new();
    let iface = Arc::new(iface);
    let iface_engine = iface.clone();
    let game_seed = seed.wrapping_add(0xDEAD_BEEF);

    // Engine thread. Owns Lua VM + CardRegistry + GameState.
    let engine_handle = thread::spawn(move || {
        let registry = match CardRegistry::load_embedded() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[engine] failed to load card registry: {e}");
                return;
            }
        };
        let your_deck = match to_deck(&registry, &your_ids) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[engine] your deck rebuild failed: {e:?}");
                return;
            }
        };
        let opp_deck = match to_deck(&registry, &opp_ids) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[engine] opp deck rebuild failed: {e:?}");
                return;
            }
        };

        let (deck_a, deck_b) = if your_side == PlayerId::A {
            (your_deck, opp_deck)
        } else {
            (opp_deck, your_deck)
        };
        let mut state = GameState::new(deck_a, deck_b);
        state.replay_journal = Some(Journal::new());

        let human_kind = AiKind::Human(iface_engine.clone());
        let ais: [AiKind; 2] = if your_side == PlayerId::A {
            [human_kind, opponent_ai_template]
        } else {
            [opponent_ai_template, human_kind]
        };

        let mut rng = StdRng::seed_from_u64(game_seed);
        let mut log: Vec<String> = Vec::new();
        let _stats = run_game_continue(&mut state, &mut rng, &mut log, registry.lua(), &ais);
        // Game ended. Send a GameOver prompt so the frontend can render
        // the result.
        iface_engine.notify_game_over(&state, your_side);
        eprintln!("[engine] game finished; winner={:?}", state.winner);
    });

    // Wait for the engine to produce its first prompt before we start
    // serving requests — the frontend's initial GET /state needs
    // something to return.
    let initial_prompt = match prompt_rx.recv() {
        Ok(p) => p,
        Err(_) => {
            eprintln!("[serve] engine died before producing initial prompt");
            let _ = engine_handle.join();
            std::process::exit(1);
        }
    };

    let cached_prompt = Arc::new(Mutex::new(initial_prompt));
    serve_http(args.port, cached_prompt, prompt_rx, action_tx)
}

/// Resolve a deck path to a Vec<String> card-ids. If `path` is None,
/// picks a random EvolvedDeck from `baselines/` if any exist, else
/// builds a fresh random genome from the playable pool. `avoid` (if
/// set) is a deck id-list to avoid duplicating — useful when both
/// sides fall back to random baseline picks and we want them to
/// differ.
fn resolve_deck(
    path: Option<&std::path::Path>,
    pool: &[Card],
    avoid: Option<&[String]>,
    rng: &mut StdRng,
) -> Vec<String> {
    if let Some(p) = path {
        let deck = EvolvedDeck::load(p).unwrap_or_else(|e| {
            eprintln!("failed to load deck {}: {e:?}", p.display());
            std::process::exit(2);
        });
        return deck.card_ids;
    }
    // Try baselines/.
    let baselines_dir = std::path::Path::new("baselines");
    if baselines_dir.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(baselines_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
            .collect();
        entries.sort();
        if !entries.is_empty() {
            use rand::seq::SliceRandom;
            // Up to 8 retries to find a baseline whose card_ids
            // differ from `avoid` — duplicates are extremely common
            // when baselines/ has only one or two files.
            for _ in 0..8 {
                let picked = entries.choose(rng).unwrap();
                if let Ok(deck) = EvolvedDeck::load(picked) {
                    if avoid.is_some_and(|a| a == deck.card_ids.as_slice()) {
                        continue;
                    }
                    eprintln!("[serve] using baseline {}", picked.display());
                    return deck.card_ids;
                }
            }
        }
    }
    // Fallback: random genome from the playable pool.
    random_genome(pool, 50, 3, rng).expect("random_genome should succeed for default pool")
}

fn count_unique(ids: &[String]) -> usize {
    use std::collections::BTreeSet;
    ids.iter().collect::<BTreeSet<_>>().len()
}

fn serve_http(
    port: u16,
    cached_prompt: Arc<Mutex<HumanPrompt>>,
    prompt_rx: Receiver<HumanPrompt>,
    action_tx: Sender<HumanAction>,
) -> mlua::Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[serve] failed to bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("[serve] listening on http://{addr}");

    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();
        let response = match (method.as_str(), url.as_str()) {
            ("GET", "/") => html_response(),
            ("GET", "/state") => json_response(&*cached_prompt.lock().unwrap()),
            ("POST", "/action") => {
                let mut body = String::new();
                let _ = std::io::Read::read_to_string(request.as_reader(), &mut body);
                handle_action(&body, &cached_prompt, &prompt_rx, &action_tx)
            }
            _ => not_found(),
        };
        if let Err(e) = request.respond(response) {
            eprintln!("[serve] respond error: {e}");
        }
    }
    Ok(())
}

fn handle_action(
    body: &str,
    cached_prompt: &Arc<Mutex<HumanPrompt>>,
    prompt_rx: &Receiver<HumanPrompt>,
    action_tx: &Sender<HumanAction>,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let action: HumanAction = match serde_json::from_str(body) {
        Ok(a) => a,
        Err(e) => return error_response(400, &format!("bad action JSON: {e}")),
    };
    // If the engine already finished, don't accept further actions.
    {
        let p = cached_prompt.lock().unwrap();
        if matches!(*p, HumanPrompt::GameOver { .. }) {
            return json_response(&*p);
        }
    }
    if action_tx.send(action).is_err() {
        return error_response(500, "engine has dropped action channel");
    }
    let next = match prompt_rx.recv() {
        Ok(p) => p,
        Err(_) => {
            return error_response(500, "engine dropped prompt channel mid-action");
        }
    };
    *cached_prompt.lock().unwrap() = next;
    json_response(&*cached_prompt.lock().unwrap())
}

fn json_response<T: serde::Serialize>(value: &T) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(value).unwrap_or_else(|e| {
        format!("{{\"error\":\"serialize failed: {e}\"}}").into_bytes()
    });
    let mut r = tiny_http::Response::from_data(body);
    r.add_header(
        "Content-Type: application/json"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    r
}

fn html_response() -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let mut r = tiny_http::Response::from_data(PLAY_HTML.as_bytes().to_vec());
    r.add_header(
        "Content-Type: text/html; charset=utf-8"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    r
}

fn not_found() -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    error_response(404, "not found")
}

fn error_response(status: u16, msg: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = format!("{{\"error\":\"{msg}\"}}");
    let mut r = tiny_http::Response::from_data(body.into_bytes()).with_status_code(status);
    r.add_header(
        "Content-Type: application/json"
            .parse::<tiny_http::Header>()
            .unwrap(),
    );
    r
}
