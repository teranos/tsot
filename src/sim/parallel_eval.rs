//! Thread-pool parallelism for the EA's fitness step. The bottleneck of
//! a single evolve run is ~1500 sequential fitness evaluations against
//! the gauntlet; each evaluation is embarrassingly parallel (no shared
//! state with the others). This module fans them out across rayon's
//! global thread pool.
//!
//! The catch: `mlua::Lua` is not `Send`, so a single `CardRegistry`
//! cannot be shared across threads. Each worker thread owns its own
//! lazily-initialized `CardRegistry` + a materialized gauntlet built
//! from the same registry (so handlers belong to that thread's Lua).
//! First touch on a thread loads cards (~100ms); every subsequent
//! evaluation reuses the cached state.
//!
//! Determinism: child generation in evolve.rs remains fully sequential
//! (one master `StdRng`). This module only parallelizes the fitness
//! scoring step, which is a pure function of `(genome, gauntlet,
//! fit_seed)` — same inputs → same output regardless of which thread
//! ran it. The output `Vec<f64>` preserves the input order.

use std::cell::RefCell;

#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

use crate::card::{Card, CardRegistry};

use crate::sim::fitness::fitness;
use crate::sim::genome::to_deck;

thread_local! {
    static WORKER_CTX: RefCell<Option<WorkerCtx>> = const { RefCell::new(None) };
}

struct WorkerCtx {
    registry: std::sync::Arc<CardRegistry>,
    gauntlet: Vec<Vec<Card>>,
    /// Identity of the gauntlet this WorkerCtx cached. If a later
    /// call uses a different gauntlet, we rebuild — supports running
    /// multiple evolve()s in the same process without poisoning.
    gauntlet_signature: Vec<Vec<String>>,
}

/// Evaluate a batch of genomes in parallel. Each job is `(genome,
/// fit_seed)`; the returned `Vec<f64>` is fitness scores in the same
/// order as input. `gauntlet_ids[i]` is the card-id list for the i-th
/// gauntlet deck — materialized inside each worker from the worker's
/// own registry (handlers belong to that thread's Lua VM, never crossed).
pub fn parallel_evaluate_genomes(
    gauntlet_ids: &[Vec<String>],
    jobs: &[(Vec<String>, u64)],
    n_per_side: u32,
    opponent_ai: &super::AiKind,
) -> Vec<f64> {
    let eval = |(genome, fit_seed): &(Vec<String>, u64)| -> f64 {
        WORKER_CTX.with(|cell| {
            let mut ctx_ref = cell.borrow_mut();
            let needs_rebuild = match ctx_ref.as_ref() {
                None => true,
                Some(ctx) => ctx.gauntlet_signature != gauntlet_ids,
            };
            if needs_rebuild {
                let registry = std::sync::Arc::new(
                    CardRegistry::load_embedded().expect("worker: failed to load cards"),
                );
                let gauntlet: Vec<Vec<Card>> = gauntlet_ids
                    .iter()
                    .map(|ids| {
                        to_deck(registry.as_ref(), ids)
                            .expect("gauntlet contains unknown card id")
                    })
                    .collect();
                *ctx_ref = Some(WorkerCtx {
                    registry,
                    gauntlet,
                    gauntlet_signature: gauntlet_ids.to_vec(),
                });
            }
            let ctx = ctx_ref.as_ref().unwrap();
            fitness(&ctx.registry, genome, &ctx.gauntlet, n_per_side, *fit_seed, opponent_ai)
                .expect("worker fitness: genome contains unknown card id")
        })
    };

    #[cfg(not(target_arch = "wasm32"))]
    {
        jobs.par_iter().map(eval).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        jobs.iter().map(eval).collect()
    }
}

/// Extract `Vec<Vec<String>>` of card ids from a gauntlet of
/// materialized decks — the cross-thread wire format. Cards' handlers
/// don't travel; only string ids do.
pub fn gauntlet_to_ids(gauntlet: &[Vec<Card>]) -> Vec<Vec<String>> {
    gauntlet
        .iter()
        .map(|deck| deck.iter().map(|c| c.id.clone()).collect())
        .collect()
}
