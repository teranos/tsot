//! `tsot replay` subcommand: re-run one game from its `game_seed`
//! (as printed by `[HEARTBEAT]` / `[GAME TIMEOUT]` in a sim run).
//! Takes the two champion JSONs that were playing + the seed + AI
//! kind; produces a byte-identical replay.
//!
//! The seed alone is not enough — an EA run produces thousands of
//! games and only the (deck_a, deck_b, ai, seed) tuple identifies
//! one. The heartbeat prints the seed; the operator supplies the
//! two decks and the AI kind used.

use clap::Parser;
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

use tsot::card::CardRegistry;
use tsot::game::{DeckUnit, GameState, Phase};
use tsot::sim::evolved_deck::EvolvedDeck;
use tsot::sim::game_trace::TracedAiKind;
use tsot::sim::genome::to_units;
use tsot::sim::{self, AiKind, GameStats};

use crate::parse_u64_hex_or_dec;

#[derive(Parser)]
pub struct ReplayArgs {
    /// game_seed to reproduce (hex `0x…` or decimal), as printed by
    /// `[GAME TIMEOUT] game_seed=…`. Optional when `--to <token>` is
    /// given (the seed comes from the token). Required otherwise.
    #[arg(long, value_parser = parse_u64_hex_or_dec)]
    pub seed: Option<u64>,
    /// JSONL trace produced by `tsot evolve --trace-games PATH`.
    /// When given, `--seed` looks up the row directly — no need to
    /// supply `--deck-a` / `--deck-b` / `--ai` (they're all in the row).
    /// Any of those flags, if also given, overrides the trace.
    ///
    /// Default: `./ccg-trace.jsonl` (CWD) when neither `--from-trace`
    /// nor `--deck-a`/`--deck-b` are supplied. So the reproduce
    /// workflow after `make evolve-shallow TRACE=1` reduces to
    /// `tsot replay --seed 0x…`.
    #[arg(long = "from-trace", value_name = "PATH")]
    pub from_trace: Option<String>,
    /// Champion JSON path for side A. Required unless `--from-trace` is set.
    #[arg(long, value_name = "PATH")]
    pub deck_a: Option<String>,
    /// Champion JSON path for side B. Required unless `--from-trace` is set.
    #[arg(long, value_name = "PATH")]
    pub deck_b: Option<String>,
    /// AI kind for the `--deck-a` / `--deck-b` path (`game` = heuristic,
    /// `fast`, `stress`). **Ignored with `--from-trace`** — the trace
    /// carries the exact `AiKind` including full UCT/MCTS config, and
    /// replay uses that verbatim. Overriding `--ai` on a trace row
    /// would produce a different game — errors out.
    #[arg(long)]
    pub ai: Option<String>,
    /// Stop-at-state token (from `[HEARTBEAT] state=<token>`). Format:
    /// `<seed_hex>@t<turn>p<phase>` (e.g. `50311316ea91daa2@t20pMain1`).
    /// When set, the seed is decoded from the token — `--seed` becomes
    /// optional and is overridden if both are given. The game runs up
    /// to `(turn, phase)`, snapshots state, then halts and dumps the
    /// board / hands / graveyards for inspection.
    #[arg(long, value_name = "TOKEN")]
    pub to: Option<String>,
    /// Print the full per-turn log after the game finishes.
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}

/// One JSONL row emitted by `tsot evolve --trace-games`. Field names
/// match the writer in `sim::game_trace::JsonlFileTrace::record`.
/// The `ai` field is the full `TracedAiKind` (kind + config), not a
/// mere kind-name string — this is the load-bearing invariant of
/// exact replay. Any UCT/MCTS config knob (iterations, exploration_c,
/// per_pick_wall_ms, etc.) round-trips verbatim.
///
/// `shuffle_seed_a` / `shuffle_seed_b` are the two `StdRng::seed_from_u64`
/// values used to shuffle the pre-game decks in `fitness_breakdown_with_trace`.
/// Without them, replay reconstructs a DIFFERENT game (same seed, same
/// pre-shuffle deck order → different post-shuffle draw order → divergent
/// state from turn 1).
#[derive(Deserialize)]
struct TraceRecord {
    seed: String,
    shuffle_seed_a: String,
    shuffle_seed_b: String,
    seat: String,
    ai: TracedAiKind,
    genome: Vec<String>,
    opponent: Vec<String>,
}

/// Outcome of a replay call. When `halt` is `Some`, the game was
/// stopped mid-flight by a `--to` token match — inspect
/// `halt.state` (not `stats.winner`) for game state at the halt
/// coordinates, and `halt.pick_timing` for the per-card wall-clock
/// breakdown of the picks that ran up to the halt.
#[derive(Debug)]
pub struct ReplayResult {
    pub stats: GameStats,
    pub log: Vec<String>,
    pub halt: Option<sim::run::HaltSnapshot>,
}

/// Load, materialize, and run one game deterministically from
/// `args.seed`. Split from the CLI printer so tests can call it
/// without capturing stdout.
///
/// Two mutually-exclusive input modes:
///   - `--from-trace PATH` — read the JSONL row whose `seed` matches
///     `args.seed`, materialize both decks from its id arrays, orient
///     seats per its `seat` field, use its `ai` (or `--ai` if given).
///   - `--deck-a` / `--deck-b` — traditional path; supply both files.
pub fn replay(registry: &Arc<CardRegistry>, args: &ReplayArgs) -> mlua::Result<ReplayResult> {
    // If --to was given, decode it — provides the seed AND a (turn, phase)
    // halt coordinate. --seed overrides the token's seed when both are set
    // (mismatch surfaces as a game that never hits the halt).
    let (stop_at, token_seed): (Option<(u32, Phase)>, Option<u64>) = match &args.to {
        Some(token) => {
            let (seed, turn, phase) = sim::run::parse_state_token(token)
                .map_err(mlua::Error::runtime)?;
            (Some((turn, phase)), Some(seed))
        }
        None => (None, None),
    };
    let seed = args
        .seed
        .or(token_seed)
        .ok_or_else(|| mlua::Error::runtime("either --seed or --to <token> must be set"))?;

    // Resolve the from-trace input:
    //   1. Explicit --from-trace PATH wins.
    //   2. If no deck args + no explicit trace, auto-discover: scan
    //      ./ccg-trace*.jsonl in CWD for a row whose seed matches
    //      `seed`. First file with a match wins. This lets the user
    //      run `tsot replay --to <token>` after ANY number of prior
    //      EA runs (each writing to its own `./ccg-trace-<seed>.jsonl`)
    //      without knowing which file the token belongs to.
    let effective_trace: Option<String> = match &args.from_trace {
        Some(path) => Some(path.clone()),
        None if args.deck_a.is_none() && args.deck_b.is_none() => {
            let candidates = discover_trace_files();
            let winner = candidates.iter().find(|p| trace_file_contains_seed(p, seed));
            match winner {
                Some(path) => Some(path.clone()),
                None if candidates.is_empty() => {
                    return Err(mlua::Error::runtime(
                        "no --from-trace given, no --deck-a/--deck-b given, and no \
                         ./ccg-trace*.jsonl files found in CWD. Run evolve with \
                         --trace-games first (or explicitly pass --from-trace / \
                         --deck-a / --deck-b)."
                            .to_string(),
                    ));
                }
                None => {
                    return Err(mlua::Error::runtime(format!(
                        "seed 0x{seed:016x} not found in any of {} local trace file(s): {:?}",
                        candidates.len(),
                        candidates,
                    )));
                }
            }
        }
        None => None,
    };

    let (state, ai) = if let Some(trace_path) = effective_trace {
        if args.ai.is_some() {
            return Err(mlua::Error::runtime(
                "--ai is not accepted with --from-trace: the trace already carries \
                 the exact AiKind (with full UCT/MCTS config). Overriding would \
                 produce a different game — exact replay refuses to guess."
                    .to_string(),
            ));
        }
        let record = find_trace_record(&trace_path, seed)?;
        let (units_a, units_b) = materialize_from_trace(registry, &record)?;
        // Use the trace's AiKind verbatim — full config, no defaults.
        let ai = record.ai.into_ai_kind();
        (GameState::from_units(units_a, units_b), ai)
    } else {
        let deck_a_path = args.deck_a.as_ref().ok_or_else(|| {
            mlua::Error::runtime("--deck-a is required when --from-trace is not set")
        })?;
        let deck_b_path = args.deck_b.as_ref().ok_or_else(|| {
            mlua::Error::runtime("--deck-b is required when --from-trace is not set")
        })?;
        let a = EvolvedDeck::load(Path::new(deck_a_path))
            .map_err(|e| mlua::Error::runtime(format!("load {}: {e}", deck_a_path)))?;
        let b = EvolvedDeck::load(Path::new(deck_b_path))
            .map_err(|e| mlua::Error::runtime(format!("load {}: {e}", deck_b_path)))?;
        let units_a = to_units(registry, &a.card_ids)
            .map_err(|e| mlua::Error::runtime(format!("{}: {e}", deck_a_path)))?;
        let units_b = to_units(registry, &b.card_ids)
            .map_err(|e| mlua::Error::runtime(format!("{}: {e}", deck_b_path)))?;
        let ai_key = args.ai.as_deref().unwrap_or("game");
        let ai = parse_ai(ai_key)?;
        (GameState::from_units(units_a, units_b), ai)
    };

    let mut rng = StdRng::seed_from_u64(seed);
    let mut log: Vec<String> = Vec::new();
    let ais = [ai.clone(), ai];

    // Arm the stop-at trigger for THIS thread only. `clear_stop_at`
    // runs regardless of how run_game_with_ai returns so a subsequent
    // real game on the same thread isn't polluted.
    if let Some((turn, phase)) = stop_at {
        sim::run::set_stop_at(turn, phase);
    }
    let (stats, _) =
        sim::run::run_game_with_ai(state, &mut rng, &mut log, registry, &ais, seed);
    let halt = if stop_at.is_some() {
        let snap = sim::run::take_halt_snapshot();
        sim::run::clear_stop_at();
        snap
    } else {
        None
    };

    Ok(ReplayResult { stats, log, halt })
}

/// List `./ccg-trace*.jsonl` files in CWD, sorted alphabetically for
/// deterministic search order. Silently empty on I/O error.
fn discover_trace_files() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(rd) = std::fs::read_dir(".") else { return out };
    for entry in rd.flatten() {
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else { continue };
        if name.starts_with("ccg-trace") && name.ends_with(".jsonl") {
            if let Some(s) = p.to_str() {
                out.push(s.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Cheap prefilter: `true` iff the file mentions the seed hex string
/// anywhere. Avoids full JSON parse when the row simply isn't there.
fn trace_file_contains_seed(path: &str, seed: u64) -> bool {
    let target = format!("0x{seed:016x}");
    let Ok(f) = std::fs::File::open(path) else { return false };
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        if line.contains(&target) {
            return true;
        }
    }
    false
}

/// Scan `trace_path` line-by-line for the JSONL row whose `seed`
/// matches `seed`. `seed` is hex-formatted `"0x{:016x}"` to match the
/// writer. Returns `NotFound` (as an mlua runtime error) if no row
/// carries that seed — a common operator mistake worth surfacing
/// distinctly from I/O and parse errors.
fn find_trace_record(trace_path: &str, seed: u64) -> mlua::Result<TraceRecord> {
    let file = std::fs::File::open(trace_path)
        .map_err(|e| mlua::Error::runtime(format!("open {trace_path}: {e}")))?;
    let target = format!("0x{seed:016x}");
    for (line_no, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| {
            mlua::Error::runtime(format!("{trace_path}:{}: read: {e}", line_no + 1))
        })?;
        if !line.contains(&target) {
            // Cheap prefilter — parse only lines that at least mention
            // the target hex string. Avoids paying serde cost on the
            // 99.99% of lines that don't match.
            continue;
        }
        let rec: TraceRecord = serde_json::from_str(&line).map_err(|e| {
            mlua::Error::runtime(format!("{trace_path}:{}: parse: {e}", line_no + 1))
        })?;
        if rec.seed == target {
            return Ok(rec);
        }
    }
    Err(mlua::Error::runtime(format!(
        "no trace row matches seed={target} in {trace_path}"
    )))
}

/// Build the (deck_a, deck_b) unit lists from a trace record — decks
/// arrive in genome/opponent order, get placed per the `seat` field,
/// and are shuffled with the trace's stored shuffle seeds. The shuffle
/// is critical: without it every downstream draw diverges and the game
/// is not the live game.
fn materialize_from_trace(
    registry: &Arc<CardRegistry>,
    rec: &TraceRecord,
) -> mlua::Result<(Vec<DeckUnit>, Vec<DeckUnit>)> {
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use tsot::sim::genome::shuffle_units;

    let genome_units = to_units(registry, &rec.genome)
        .map_err(|e| mlua::Error::runtime(format!("genome: {e}")))?;
    let opponent_units = to_units(registry, &rec.opponent)
        .map_err(|e| mlua::Error::runtime(format!("opponent: {e}")))?;

    let shuffle_seed_a = parse_hex_u64(&rec.shuffle_seed_a)
        .map_err(|e| mlua::Error::runtime(format!("shuffle_seed_a: {e}")))?;
    let shuffle_seed_b = parse_hex_u64(&rec.shuffle_seed_b)
        .map_err(|e| mlua::Error::runtime(format!("shuffle_seed_b: {e}")))?;

    // Seat A = whoever played first-seat in the live game; seat B =
    // second-seat. The shuffle seeds are recorded seat-A-first,
    // seat-B-second regardless of who was the genome — matching how
    // fitness_breakdown_with_trace draws them.
    let (mut deck_a, mut deck_b) = match rec.seat.as_str() {
        "A" => (genome_units, opponent_units),
        "B" => (opponent_units, genome_units),
        other => {
            return Err(mlua::Error::runtime(format!(
                "trace seat must be \"A\" or \"B\", got {other:?}"
            )))
        }
    };
    let mut rng_a = StdRng::seed_from_u64(shuffle_seed_a);
    let mut rng_b = StdRng::seed_from_u64(shuffle_seed_b);
    shuffle_units(&mut deck_a, &mut rng_a);
    shuffle_units(&mut deck_b, &mut rng_b);
    Ok((deck_a, deck_b))
}

/// Parse `"0x…"` hex or a bare decimal u64. Used for the shuffle
/// seeds in the trace record (writer format is `"0x{:016x}"`).
fn parse_hex_u64(s: &str) -> Result<u64, std::num::ParseIntError> {
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(rest, 16)
    } else {
        s.parse::<u64>()
    }
}

/// Parse `--ai` for the `--deck-a` / `--deck-b` path only. UCT and
/// MCTS are DELIBERATELY rejected here — their behaviour depends on
/// config (iterations, exploration_c, etc.) that this CLI has no way
/// to supply exactly. For UCT/MCTS games, use `--from-trace` which
/// carries the exact config via the trace record.
fn parse_ai(s: &str) -> mlua::Result<AiKind> {
    match s.to_ascii_lowercase().as_str() {
        "game" | "heuristic" => Ok(AiKind::Game),
        "fast" => Ok(AiKind::Fast),
        "stress" => Ok(AiKind::Stress),
        "uct" | "mcts" => Err(mlua::Error::runtime(format!(
            "--ai {s} requires a full config — use --from-trace (which carries \
             the exact AiKind) instead of --deck-a/--deck-b + --ai"
        ))),
        other => Err(mlua::Error::runtime(format!(
            "unknown --ai '{other}' (use game|fast|stress; uct/mcts require --from-trace)"
        ))),
    }
}

pub fn run_replay(registry: &Arc<CardRegistry>, args: &ReplayArgs) -> mlua::Result<()> {
    // Resolve seed early so the header can print it whether it came
    // from --seed or --to.
    let (token_seed, halt_target) = match &args.to {
        Some(tok) => {
            let (s, t, p) = sim::run::parse_state_token(tok).map_err(mlua::Error::runtime)?;
            (Some(s), Some((t, p)))
        }
        None => (None, None),
    };
    let seed = args
        .seed
        .or(token_seed)
        .ok_or_else(|| mlua::Error::runtime("either --seed or --to <token> must be set"))?;

    // Header source: mirror the resolution `replay` does — explicit
    // --from-trace wins; otherwise scan `./ccg-trace*.jsonl` for the
    // file whose row matches `seed` (same auto-discover replay() uses,
    // so the header can never disagree with the actual replay path).
    let effective_trace_for_header: Option<String> = match &args.from_trace {
        Some(path) => Some(path.clone()),
        None if args.deck_a.is_none() && args.deck_b.is_none() => {
            let candidates = discover_trace_files();
            candidates.iter().find(|p| trace_file_contains_seed(p, seed)).cloned()
        }
        None => None,
    };
    let (a_label, b_label, a_src, b_src, ai_key) = if let Some(trace_path) = effective_trace_for_header {
        let rec = find_trace_record(&trace_path, seed)?;
        let (a_label, b_label) = match rec.seat.as_str() {
            "A" => ("genome".to_string(), "opponent".to_string()),
            _ => ("opponent".to_string(), "genome".to_string()),
        };
        // Header shows the full AI kind+config from the trace so the
        // operator can visually confirm what config the replay is
        // actually using (was the load-bearing bug in approximate mode).
        let ai = format!("{:?}", rec.ai);
        (a_label, b_label, format!("{trace_path} (seed row)"), format!("{trace_path} (seed row)"), ai)
    } else {
        let deck_a = args.deck_a.as_deref().unwrap_or("<missing>");
        let deck_b = args.deck_b.as_deref().unwrap_or("<missing>");
        let a_label = EvolvedDeck::load(Path::new(deck_a))
            .map(|d| d.label)
            .unwrap_or_else(|_| deck_a.to_string());
        let b_label = EvolvedDeck::load(Path::new(deck_b))
            .map(|d| d.label)
            .unwrap_or_else(|_| deck_b.to_string());
        let ai = args.ai.clone().unwrap_or_else(|| "game".to_string());
        (a_label, b_label, deck_a.to_string(), deck_b.to_string(), ai)
    };

    let t0 = std::time::Instant::now();
    let ReplayResult { stats, log, halt } = replay(registry, args)?;
    let elapsed = t0.elapsed();

    println!();
    println!("Replay: seed=0x{seed:016x}");
    println!("  A: {a_label:<20} ({a_src})");
    println!("  B: {b_label:<20} ({b_src})");
    println!("  ai: {ai_key}");
    if let Some((t, p)) = halt_target {
        println!("  halt-at: turn={t} phase={p:?}");
    }
    println!();

    if let Some(halt) = halt {
        // --to path: dump the pre-halt state + per-pick timing via the
        // same body report_game_timeout uses. Winner from stats is a
        // sentinel and not reported.
        println!(
            "Halted at turn={} phase={:?} (wall: {elapsed:.2?})",
            halt.state.turn, halt.state.phase,
        );
        println!();
        sim::run::report_game_state(
            "REPLAY HALT",
            &halt.state,
            "replay_stop_at",
            seed,
            None,
            None,
            Some(&halt.pick_timing),
            0,
        );
        return Ok(());
    }

    if let Some((t, p)) = halt_target {
        eprintln!(
            "warning: halt condition never reached — game ended naturally at turn={} \
             winner={:?} (halt target: turn={t} phase={p:?})",
            stats.turns, stats.winner,
        );
    }

    println!(
        "Winner: {:?}   turns: {}   wall: {:.2?}",
        stats.winner, stats.turns, elapsed
    );
    println!(
        "A: played={} deaths={} milled={}   B: played={} deaths={} milled={}",
        stats.a_played,
        stats.a_deaths,
        stats.a_milled_to_exile,
        stats.b_played,
        stats.b_deaths,
        stats.b_milled_to_exile,
    );

    if args.verbose {
        println!();
        println!("--- log ({} lines) ---", log.len());
        for line in &log {
            println!("{line}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsot::card::CardType;

    fn vanilla_id(registry: &CardRegistry) -> String {
        registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.len() == 1
                    && !c.cost[0].is_x
            })
            .expect("a vanilla creature exists in cards/")
            .id
            .clone()
    }

    fn tmp_champ(tag: &str, ids: Vec<String>) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("tsot-replay-test-{tag}.json"));
        let d = EvolvedDeck {
            label: tag.into(),
            fitness: 0.5,
            base_seed: 0,
            generations_run: 0,
            card_ids: ids,
        };
        d.save(&p).unwrap();
        p.to_string_lossy().to_string()
    }

    /// `replay` loading via EvolvedDeck must produce byte-identical
    /// stats to calling `run_game_with_ai` directly with the same
    /// materialized state + seed. Any drift in the load path (e.g.
    /// silently reordering, silently skipping cardless) becomes a
    /// test failure here.
    #[test]
    fn replay_matches_direct_run_game_with_ai() {
        let registry =
            Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let vanilla = vanilla_id(&registry);
        let ids: Vec<String> = (0..50).map(|_| vanilla.clone()).collect();
        let seed: u64 = 0xC0FFEE_1234_5678;

        // Path 1: via CLI's `replay`.
        let a_path = tmp_champ("cli_a", ids.clone());
        let b_path = tmp_champ("cli_b", ids.clone());
        let args = ReplayArgs {
            seed: Some(seed),
            to: None,
            from_trace: None,
            deck_a: Some(a_path.clone()),
            deck_b: Some(b_path.clone()),
            ai: Some("fast".to_string()),
            verbose: false,
        };
        let ReplayResult { stats: stats_cli, .. } = replay(&registry, &args).unwrap();

        // Path 2: direct — same materialization, same seed, no CLI glue.
        let units_a = to_units(&registry, &ids).unwrap();
        let units_b = to_units(&registry, &ids).unwrap();
        let state = GameState::from_units(units_a, units_b);
        let mut rng = StdRng::seed_from_u64(seed);
        let mut log = Vec::new();
        let ais = [AiKind::Fast, AiKind::Fast];
        let (stats_direct, _) =
            sim::run::run_game_with_ai(state, &mut rng, &mut log, &registry, &ais, seed);

        assert_eq!(stats_cli.winner, stats_direct.winner, "winner drift");
        assert_eq!(stats_cli.turns, stats_direct.turns, "turn count drift");
        assert_eq!(stats_cli.a_played, stats_direct.a_played, "a_played drift");
        assert_eq!(stats_cli.b_played, stats_direct.b_played, "b_played drift");

        let _ = std::fs::remove_file(&a_path);
        let _ = std::fs::remove_file(&b_path);
    }

    /// `--from-trace` must reconstruct the exact game from a JSONL row.
    /// Round-trip: write a trace via `JsonlFileTrace`, then read it
    /// back via `replay(--from-trace)`, and verify the stats match the
    /// direct-call baseline that produced the same seed.
    #[test]
    fn replay_from_trace_reconstructs_the_same_game() {
        use tsot::sim::game_trace::{GameTrace, JsonlFileTrace, TraceSink};

        let registry =
            Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let vanilla = vanilla_id(&registry);
        let ids: Vec<String> = (0..50).map(|_| vanilla.clone()).collect();
        let seed: u64 = 0xBAD_C0DE_CAFE_F00D;

        // Write a trace file with a single row for this seed.
        let mut trace_path = std::env::temp_dir();
        trace_path.push("tsot-replay-from-trace-test.jsonl");
        let sink = JsonlFileTrace::create(&trace_path).unwrap();
        let ai_fast = AiKind::Fast;
        let shuf_a: u64 = 0xAAAA_1111_2222_3333;
        let shuf_b: u64 = 0xBBBB_4444_5555_6666;
        sink.record(GameTrace {
            seed,
            shuffle_seed_a: shuf_a,
            shuffle_seed_b: shuf_b,
            genome: &ids,
            opponent: &ids,
            seat: 'A',
            ai: &ai_fast,
        });
        sink.flush();
        drop(sink);

        // Replay via --from-trace.
        let args = ReplayArgs {
            seed: Some(seed),
            to: None,
            from_trace: Some(trace_path.to_string_lossy().to_string()),
            deck_a: None,
            deck_b: None,
            ai: None, // pick up "fast" from the trace row
            verbose: false,
        };
        let ReplayResult { stats: stats_from_trace, .. } = replay(&registry, &args).unwrap();

        // Baseline: direct call with the same materialized + SHUFFLED
        // decks + seed. Shuffle order matches the seat-A branch of
        // fitness_breakdown_with_trace so the round-trip is byte-exact.
        use tsot::sim::genome::shuffle_units;
        let mut units_a = to_units(&registry, &ids).unwrap();
        let mut units_b = to_units(&registry, &ids).unwrap();
        {
            let mut rng_a = StdRng::seed_from_u64(shuf_a);
            let mut rng_b = StdRng::seed_from_u64(shuf_b);
            shuffle_units(&mut units_a, &mut rng_a);
            shuffle_units(&mut units_b, &mut rng_b);
        }
        let state = GameState::from_units(units_a, units_b);
        let mut rng = StdRng::seed_from_u64(seed);
        let mut log = Vec::new();
        let ais = [AiKind::Fast, AiKind::Fast];
        let (stats_direct, _) =
            sim::run::run_game_with_ai(state, &mut rng, &mut log, &registry, &ais, seed);

        assert_eq!(stats_from_trace.winner, stats_direct.winner);
        assert_eq!(stats_from_trace.turns, stats_direct.turns);
        assert_eq!(stats_from_trace.a_played, stats_direct.a_played);
        assert_eq!(stats_from_trace.b_played, stats_direct.b_played);

        let _ = std::fs::remove_file(&trace_path);
    }

    /// A seed not present in the trace must surface a clear error, not
    /// a silent successful lookup of the wrong row.
    #[test]
    fn replay_from_trace_missing_seed_errors_cleanly() {
        use tsot::sim::game_trace::{GameTrace, JsonlFileTrace, TraceSink};

        let registry =
            Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let vanilla = vanilla_id(&registry);
        let ids: Vec<String> = (0..50).map(|_| vanilla.clone()).collect();

        let mut trace_path = std::env::temp_dir();
        trace_path.push("tsot-replay-missing-seed-test.jsonl");
        let sink = JsonlFileTrace::create(&trace_path).unwrap();
        let ai_fast = AiKind::Fast;
        sink.record(GameTrace {
            seed: 0x1111_1111_1111_1111,
            shuffle_seed_a: 0xDEAD_BEEF_1111_2222,
            shuffle_seed_b: 0xDEAD_BEEF_3333_4444,
            genome: &ids,
            opponent: &ids,
            seat: 'A',
            ai: &ai_fast,
        });
        sink.flush();
        drop(sink);

        let args = ReplayArgs {
            seed: Some(0xDEAD_BEEF_DEAD_BEEF), // not in the trace
            to: None,
            from_trace: Some(trace_path.to_string_lossy().to_string()),
            deck_a: None,
            deck_b: None,
            ai: None,
            verbose: false,
        };
        let err = replay(&registry, &args).unwrap_err();
        assert!(
            err.to_string().contains("no trace row matches"),
            "expected missing-seed error, got: {err}",
        );

        let _ = std::fs::remove_file(&trace_path);
    }

    /// `--to <token>` must halt the game at the encoded (turn, phase)
    /// and expose a snapshot whose coordinates match. Otherwise the
    /// state dump surfaces the wrong moment — a total observability
    /// failure of the feature.
    #[test]
    fn replay_with_to_token_halts_at_encoded_state() {
        let registry =
            Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let vanilla = vanilla_id(&registry);
        let ids: Vec<String> = (0..50).map(|_| vanilla.clone()).collect();
        let seed: u64 = 0xF00D_CAFE_1234_ABCD;

        // Target early in the game so a heuristic run will reach it.
        let target_turn: u32 = 3;
        let target_phase = Phase::Main1;
        let token = sim::run::format_state_token(seed, target_turn, target_phase);

        let a_path = tmp_champ("to_a", ids.clone());
        let b_path = tmp_champ("to_b", ids);

        let args = ReplayArgs {
            seed: None, // encoded in --to
            to: Some(token),
            from_trace: None,
            deck_a: Some(a_path.clone()),
            deck_b: Some(b_path.clone()),
            ai: Some("fast".to_string()),
            verbose: false,
        };
        let result = replay(&registry, &args).unwrap();

        let halt = result.halt.expect("halt should fire at target state");
        assert_eq!(halt.state.turn, target_turn, "snapshot turn matches token");
        assert_eq!(halt.state.phase, target_phase, "snapshot phase matches token");

        let _ = std::fs::remove_file(&a_path);
        let _ = std::fs::remove_file(&b_path);
    }

    #[test]
    fn parse_ai_maps_expected_kinds() {
        assert!(matches!(parse_ai("game").unwrap(), AiKind::Game));
        assert!(matches!(parse_ai("heuristic").unwrap(), AiKind::Game));
        assert!(matches!(parse_ai("fast").unwrap(), AiKind::Fast));
        assert!(matches!(parse_ai("stress").unwrap(), AiKind::Stress));
        assert!(matches!(parse_ai("GAME").unwrap(), AiKind::Game));
        // uct/mcts REJECTED via --ai (need trace's full config).
        assert!(parse_ai("uct").is_err());
        assert!(parse_ai("mcts").is_err());
        assert!(parse_ai("").is_err());
        assert!(parse_ai("nope").is_err());
    }
}
