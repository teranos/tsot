// The canonical Teranos. Locked.
//
// One byte changed in INVOCATION = a different world. Do not edit
// INVOCATION; do not re-derive WORLD_SEED. Both are part of the
// world's identity from launch and immutable for the life of the
// canonical Teranos.
//
// Lives here until TSOT is wired as a path dependency, then relocates
// to `tsot::teranos` unchanged. The bytes are the bytes; the module
// that exposes them is incidental.

pub const INVOCATION: &str = "And I hereby invoke the Existence of Teranos, the many Symbols that give Meaning to, and the many Colors that are this World.";

// SHA-256(INVOCATION) = 5adea7cf0c0a1b2bdd3a86c65f41ea81f724adb6b891436774d153a82f0c9649
// First 8 bytes interpreted as little-endian u64.
// Verify:
//   printf '%s' "$INVOCATION" | shasum -a 256
// The literal is checked against an in-test re-derivation; compile-time
// SHA-256 derivation lands when const_sha2 is added (separate slice).
pub const WORLD_SEED: u64 = 0x2b1b_0a0c_cfa7_de5a;

// World topology: a cylinder. X wraps; Y is bounded by polar ocean.
// These bounds become load-bearing the moment cards spawn at (x, y, z)
// and a player picks one up — pickup provenance hashes canonical
// coordinates, so changing CIRC_X or Y_LAT after card launch invalidates
// every collection. Lock before v0.4 ships.
pub const WORLD_CIRC_X: i32 = 4096;
pub const WORLD_Y_LAT: i32 = 2048;

// Voxel vertical range. Surface elevation lives in a narrower band
// inside this range. Above SURFACE_Z_MAX is always air (sky);
// below SURFACE_Z_MIN is always rock (deep underground).
pub const Z_MIN: i32 = -32;
pub const Z_MAX: i32 = 32;
pub const SURFACE_Z_MIN: i32 = -16;
pub const SURFACE_Z_MAX: i32 = 16;

// Terracing gates on the LOCAL RAW-ELEVATION GRADIENT, not an
// independent mask. Only columns where the smooth heightmap is already
// steep get snapped to plateaus, so cliffs align with natural ridge
// lines and can't appear as isolated islands in flat terrain. Plateau
// islands require sharp spikes in raw elevation; the noise is smooth
// enough that those don't exist.
//
// GRADIENT_THRESHOLD is the sum of forward-x + forward-y |Δraw| at
// which we switch from smooth-rounded to terraced. Lower = more cliffs.
pub const TERRACE_STEP: i32 = 3;
pub const GRADIENT_THRESHOLD: f32 = 0.65;

// Noise frequencies in inverse-tile units. `SURFACE_NOISE_FREQUENCY =
// 1/64` means one full surface-noise period spans 64 tiles. Break and
// cave frequencies are tuned to the visual features they generate
// (cliff segmentation, cave size).
pub const SURFACE_NOISE_FREQUENCY: f32 = 1.0 / 64.0;
pub const BREAK_NOISE_FREQUENCY: f32 = 1.0 / 24.0;
pub const CAVE_NOISE_FREQUENCY: f32 = 1.0 / 24.0;
pub const SURFACE_NOISE_OCTAVES: u32 = 4;
pub const BREAK_NOISE_OCTAVES: u32 = 1;
pub const CAVE_NOISE_OCTAVES: u32 = 3;
pub const CAVE_OPEN_THRESHOLD: f32 = 0.45;

// Day length in real seconds. 5 hours = one full day-night cycle.
// Position-dependent phase: longitude shifts local phase.
pub const DAY_LENGTH_SECS: u64 = 18000;

// Vision constants (tiles).
pub const VISION_R_DAY: f32 = 12.0;
pub const VISION_R_NIGHT: f32 = 4.0;
pub const VISION_R_UNDERGROUND: f32 = 3.0;
pub const FLASHLIGHT_CONE_LEN: f32 = 12.0;
pub const FLASHLIGHT_CONE_ANGLE_DEG: f32 = 60.0;

// Voxel kinds. Walkability and rendering are downstream of this enum
// (see roam::world). Keep this set small until there's a reason to split.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TileKind {
    Air = 0,
    Grass = 1,
    Rock = 2,
    ShallowWater = 3,
    DeepWater = 4,
}

impl TileKind {
    /// Single source of truth for tile color. JS / Elm read this via the
    /// FFI color table; never re-invent RGB on the JS side.
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            TileKind::Air => [10, 10, 12],
            TileKind::Grass => [70, 122, 64],
            TileKind::Rock => [110, 100, 90],
            TileKind::ShallowWater => [80, 130, 180],
            TileKind::DeepWater => [40, 70, 130],
        }
    }
}

// ----- splitmix64 -----

// Named constants from Steele/Lea SplitMix64. These ARE the algorithm —
// changing them produces a different RNG. They live as named consts so
// "0x9E3779B97F4A7C15" never appears as a bare literal in the codebase.
pub const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
pub const SPLITMIX_MIX_1: u64 = 0xBF58_476D_1CE4_E5B9;
pub const SPLITMIX_MIX_2: u64 = 0x94D0_49BB_1331_11EB;

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(SPLITMIX_GAMMA);
    x = (x ^ (x >> 30)).wrapping_mul(SPLITMIX_MIX_1);
    x = (x ^ (x >> 27)).wrapping_mul(SPLITMIX_MIX_2);
    x ^ (x >> 31)
}

// ----- named-derivation hash dimensions -----

// Each independent hash off (x, y) used in worldgen gets its own
// dimension. The dimension's salt and multiplier are derived from the
// variant name via FNV-1a-64 at compile time — renaming a dimension
// reshuffles the world; no hand-typed magic hex literals.
//
// HashDimension is internal to teranos.rs because it's a worldgen
// implementation detail, not part of the public Flower surface.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum HashDimension {
    FlowerPresence,
    FlowerPetalEdge,
    FlowerCoreCenter,
    FlowerCoreEdge,
    FlowerPetalCount,
    TerrainBreakNoise,
    TerrainCaveCarve,
}

const FNV_PRIME_64: u64 = 0x0000_0100_0000_01b3;
const FNV_OFFSET_BASIS_SALT: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_OFFSET_BASIS_MULT: u64 = 0xa3b8_c91e_7d6a_2f47;

const fn fnv1a_64(name: &[u8], offset_basis: u64) -> u64 {
    let mut h: u64 = offset_basis;
    let mut i = 0;
    while i < name.len() {
        h ^= name[i] as u64;
        h = h.wrapping_mul(FNV_PRIME_64);
        i += 1;
    }
    h
}

impl HashDimension {
    const fn name(self) -> &'static [u8] {
        match self {
            Self::FlowerPresence => b"FlowerPresence",
            Self::FlowerPetalEdge => b"FlowerPetalEdge",
            Self::FlowerCoreCenter => b"FlowerCoreCenter",
            Self::FlowerCoreEdge => b"FlowerCoreEdge",
            Self::FlowerPetalCount => b"FlowerPetalCount",
            Self::TerrainBreakNoise => b"TerrainBreakNoise",
            Self::TerrainCaveCarve => b"TerrainCaveCarve",
        }
    }

    const fn salt(self) -> u64 {
        fnv1a_64(self.name(), FNV_OFFSET_BASIS_SALT)
    }

    /// OR'd with 1 to guarantee odd — odd multipliers make 64-bit
    /// integer multiply a permutation, which keeps the hash bijective
    /// in its multiplier step.
    const fn mult(self) -> u64 {
        fnv1a_64(self.name(), FNV_OFFSET_BASIS_MULT) | 1
    }
}

/// 64-bit hash of (x, y) parametrized by an independent dimension.
/// Same (x, y, dimension) on every peer = same hash, no coordination
/// required. The cylinder seam is folded by canonicalizing x first.
fn world_hash(x: i32, y: i32, dim: HashDimension) -> u64 {
    let cx = canonical_x(x);
    splitmix64(
        WORLD_SEED
            ^ dim.salt()
            ^ ((cx as i64 as u64).wrapping_mul(dim.mult()))
            ^ (y as i64 as u64),
    )
}

// ----- weighted-pick over typed tables -----

/// Sum of a const weight table. Compile-time so the total can be checked
/// against an expected value via `const _: () = assert!(...)`.
const fn weight_table_sum<T: Copy, const N: usize>(table: &[(T, u16); N]) -> u32 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < N {
        sum += table[i].1 as u32;
        i += 1;
    }
    sum
}

/// Deterministic weighted pick. `r` is a uniform 64-bit hash; modulo
/// `total` selects an index biased by each row's weight.
///
/// `total` is passed in (not recomputed) because callers pin it as a
/// compile-time constant — the const sum-check on the table catches
/// any drift between declared `total` and actual sum at compile time.
fn pick_weighted<T: Copy, const N: usize>(
    r: u64,
    table: &[(T, u16); N],
    total: u32,
) -> T {
    let mut x = (r % total as u64) as u32;
    let mut i = 0;
    while i < N {
        let w = table[i].1 as u32;
        if x < w {
            return table[i].0;
        }
        x -= w;
        i += 1;
    }
    // Unreachable when `total == sum(table)`. The const sum-check on
    // every declared table guarantees this; if we land here it's a
    // logic bug in this function, not a caller-data problem.
    unreachable!("pick_weighted: weight table inconsistent with declared total")
}

// ----- noise primitives -----

#[inline]
fn lattice_value(seed: u64, x: i32, y: i32, z: i32) -> f32 {
    let mut h = seed;
    h = splitmix64(h ^ (x as i64 as u64));
    h = splitmix64(h ^ (y as i64 as u64));
    h = splitmix64(h ^ (z as i64 as u64));
    // Map top 32 bits to [-1, 1].
    let u = (h >> 32) as u32;
    (u as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// 2D value noise on (xf, yf), x-periodic. `x_period` is the lattice
// wrap width in integer lattice points; 0 means no wrap. Required for
// the cylindrical world — without it, tile WORLD_CIRC_X-1 and tile 0
// sample uncorrelated lattice points and the seam is a discontinuity.
fn value_noise_2d(seed: u64, xf: f32, yf: f32, x_period: i32) -> f32 {
    let xi_raw = xf.floor() as i32;
    let yi = yf.floor() as i32;
    let u = fade(xf - xi_raw as f32);
    let v = fade(yf - yi as f32);
    let (xi, xi_next) = if x_period > 0 {
        (xi_raw.rem_euclid(x_period), (xi_raw + 1).rem_euclid(x_period))
    } else {
        (xi_raw, xi_raw + 1)
    };
    let a00 = lattice_value(seed, xi, yi, 0);
    let a10 = lattice_value(seed, xi_next, yi, 0);
    let a01 = lattice_value(seed, xi, yi + 1, 0);
    let a11 = lattice_value(seed, xi_next, yi + 1, 0);
    lerp(lerp(a00, a10, u), lerp(a01, a11, u), v)
}

// 3D value noise on (xf, yf, zf), x-periodic. Same seam-continuity
// reasoning as value_noise_2d. Caves will hit this when they're
// rendered; pay the cost now to keep the noise contract consistent.
fn value_noise_3d(seed: u64, xf: f32, yf: f32, zf: f32, x_period: i32) -> f32 {
    let xi_raw = xf.floor() as i32;
    let yi = yf.floor() as i32;
    let zi = zf.floor() as i32;
    let u = fade(xf - xi_raw as f32);
    let v = fade(yf - yi as f32);
    let w = fade(zf - zi as f32);
    let (xi, xi_next) = if x_period > 0 {
        (xi_raw.rem_euclid(x_period), (xi_raw + 1).rem_euclid(x_period))
    } else {
        (xi_raw, xi_raw + 1)
    };
    let a000 = lattice_value(seed, xi, yi, zi);
    let a100 = lattice_value(seed, xi_next, yi, zi);
    let a010 = lattice_value(seed, xi, yi + 1, zi);
    let a110 = lattice_value(seed, xi_next, yi + 1, zi);
    let a001 = lattice_value(seed, xi, yi, zi + 1);
    let a101 = lattice_value(seed, xi_next, yi, zi + 1);
    let a011 = lattice_value(seed, xi, yi + 1, zi + 1);
    let a111 = lattice_value(seed, xi_next, yi + 1, zi + 1);
    lerp(
        lerp(lerp(a000, a100, u), lerp(a010, a110, u), v),
        lerp(lerp(a001, a101, u), lerp(a011, a111, u), v),
        w,
    )
}

// Fractal sum: octaves of value noise at increasing frequency.
// `x_period_base` is the lattice wrap width at octave 0; doubles each
// octave (since each octave doubles the frequency, the lattice spans
// twice as many points across the same world distance).
//
// Per-octave seed offset uses SPLITMIX_GAMMA so consecutive octaves
// land at uncorrelated noise tables — same constant, named role.
fn fractal_2d(seed: u64, x: f32, y: f32, octaves: u32, x_period_base: i32) -> f32 {
    let mut total = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut max_amp = 0.0;
    let mut period = x_period_base;
    for o in 0..octaves {
        let oseed = seed.wrapping_add((o as u64).wrapping_mul(SPLITMIX_GAMMA));
        total += value_noise_2d(oseed, x * freq, y * freq, period) * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
        period = period.saturating_mul(2);
    }
    total / max_amp
}

fn fractal_3d(seed: u64, x: f32, y: f32, z: f32, octaves: u32, x_period_base: i32) -> f32 {
    let mut total = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut max_amp = 0.0;
    let mut period = x_period_base;
    for o in 0..octaves {
        let oseed = seed.wrapping_add((o as u64).wrapping_mul(SPLITMIX_GAMMA));
        total += value_noise_3d(oseed, x * freq, y * freq, z * freq, period) * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
        period = period.saturating_mul(2);
    }
    total / max_amp
}

// ----- world geometry -----

#[inline]
fn canonical_x(x: i32) -> i32 {
    x.rem_euclid(WORLD_CIRC_X)
}

// Topmost solid tile for column (x, y). Sea level is z=0.
// Polar oceans (|y| > WORLD_Y_LAT): floor of the world.
// Raw smooth elevation at (x, y). Clamps y into the playable band so
// the gradient computation near the polar boundary doesn't see the
// artificial Z_MIN jump.
fn raw_elevation(x: i32, y: i32) -> f32 {
    let cy = y.clamp(-WORLD_Y_LAT, WORLD_Y_LAT);
    let cx = canonical_x(x);
    let x_period_base = (WORLD_CIRC_X as f32 * SURFACE_NOISE_FREQUENCY) as i32;
    let n = fractal_2d(
        WORLD_SEED,
        cx as f32 * SURFACE_NOISE_FREQUENCY,
        cy as f32 * SURFACE_NOISE_FREQUENCY,
        SURFACE_NOISE_OCTAVES,
        x_period_base,
    );
    let amplitude = (SURFACE_Z_MAX - SURFACE_Z_MIN) as f32 * 0.5;
    let mid = (SURFACE_Z_MAX + SURFACE_Z_MIN) as f32 * 0.5;
    n * amplitude + mid
}

pub fn surface_z(x: i32, y: i32) -> i32 {
    if y.abs() > WORLD_Y_LAT {
        return Z_MIN;
    }

    let raw = raw_elevation(x, y);
    let raw_e = raw_elevation(x + 1, y);
    let raw_s = raw_elevation(x, y + 1);
    let gradient = (raw_e - raw).abs() + (raw_s - raw).abs();

    // Break noise: high-frequency mask that interrupts long ridges into
    // short cliff segments. Period 24 tiles → contiguous "cliff allowed"
    // patches of ~12 tiles separated by ~12-tile smooth gaps. The hash
    // dimension carries the named seed; no hand-typed magic salt.
    let bs_period = (WORLD_CIRC_X as f32 * BREAK_NOISE_FREQUENCY) as i32;
    let cx = canonical_x(x);
    let break_noise = fractal_2d(
        WORLD_SEED ^ HashDimension::TerrainBreakNoise.salt(),
        cx as f32 * BREAK_NOISE_FREQUENCY,
        y as f32 * BREAK_NOISE_FREQUENCY,
        BREAK_NOISE_OCTAVES,
        bs_period,
    );
    let break_active = break_noise > 0.0;

    let z = if gradient > GRADIENT_THRESHOLD && break_active {
        (raw / TERRACE_STEP as f32).round() as i32 * TERRACE_STEP
    } else {
        raw.round() as i32
    };
    z.clamp(SURFACE_Z_MIN, SURFACE_Z_MAX)
}

// ----- flowers -----

/// Petal-edge / petal-center color. Rarity weights are declared in
/// `FLOWER_COLOR_WEIGHTS` below; the const sum-check guards them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FlowerColor {
    Red = 0,
    Yellow = 1,
    Blue = 2,
    Purple = 3,
    Azure = 4,
    Pink = 5,
    Glow = 6,
}

impl FlowerColor {
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            FlowerColor::Red => [0xee, 0x44, 0x44],
            FlowerColor::Yellow => [0xff, 0xdd, 0x44],
            FlowerColor::Blue => [0x44, 0x88, 0xff],
            FlowerColor::Purple => [0xaa, 0x44, 0xff],
            FlowerColor::Azure => [0x44, 0xcc, 0xff],
            FlowerColor::Pink => [0xff, 0x99, 0xcc],
            FlowerColor::Glow => [0xff, 0xff, 0xff],
        }
    }
}

/// Core-center color. Black is super-mega-rare.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FlowerCore {
    White = 0,
    Yellow = 1,
    Black = 2,
}

impl FlowerCore {
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            FlowerCore::White => [0xff, 0xff, 0xff],
            FlowerCore::Yellow => [0xff, 0xdd, 0x44],
            FlowerCore::Black => [0x10, 0x10, 0x10],
        }
    }
}

/// Outer-edge color of the core's radial gradient. White (most common),
/// or matches one of the petal colors.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CoreEdge {
    White = 0,
    MatchPetalCenter = 1,
    MatchPetalEdge = 2,
}

// ----- flower weight tables -----

const FLOWER_COLOR_WEIGHTS: [(FlowerColor, u16); 7] = [
    (FlowerColor::Red, 350),
    (FlowerColor::Yellow, 350),
    (FlowerColor::Blue, 70),
    (FlowerColor::Purple, 70),
    (FlowerColor::Azure, 70),
    (FlowerColor::Pink, 60),
    (FlowerColor::Glow, 30),
];
const FLOWER_COLOR_WEIGHTS_TOTAL: u32 = 1000;
const _: () = assert!(
    weight_table_sum(&FLOWER_COLOR_WEIGHTS) == FLOWER_COLOR_WEIGHTS_TOTAL,
    "FLOWER_COLOR_WEIGHTS rows must sum to FLOWER_COLOR_WEIGHTS_TOTAL",
);

const FLOWER_CORE_WEIGHTS: [(FlowerCore, u16); 3] = [
    (FlowerCore::White, 495),
    (FlowerCore::Yellow, 495),
    (FlowerCore::Black, 10),
];
const FLOWER_CORE_WEIGHTS_TOTAL: u32 = 1000;
const _: () = assert!(
    weight_table_sum(&FLOWER_CORE_WEIGHTS) == FLOWER_CORE_WEIGHTS_TOTAL,
    "FLOWER_CORE_WEIGHTS rows must sum to FLOWER_CORE_WEIGHTS_TOTAL",
);

const CORE_EDGE_WEIGHTS: [(CoreEdge, u16); 3] = [
    (CoreEdge::White, 70),
    (CoreEdge::MatchPetalCenter, 15),
    (CoreEdge::MatchPetalEdge, 15),
];
const CORE_EDGE_WEIGHTS_TOTAL: u32 = 100;
const _: () = assert!(
    weight_table_sum(&CORE_EDGE_WEIGHTS) == CORE_EDGE_WEIGHTS_TOTAL,
    "CORE_EDGE_WEIGHTS rows must sum to CORE_EDGE_WEIGHTS_TOTAL",
);

const PETAL_COUNT_WEIGHTS: [(u8, u16); 4] = [
    (5, 9939),
    (6, 50),
    (7, 10),
    (8, 1),
];
const PETAL_COUNT_WEIGHTS_TOTAL: u32 = 10000;
const _: () = assert!(
    weight_table_sum(&PETAL_COUNT_WEIGHTS) == PETAL_COUNT_WEIGHTS_TOTAL,
    "PETAL_COUNT_WEIGHTS rows must sum to PETAL_COUNT_WEIGHTS_TOTAL",
);

/// Density of flowers on land: roughly one flower per N candidate
/// tiles. The presence-hash gate fires when the dimension's hash for
/// (x, y) is a multiple of this denominator.
pub const FLOWER_DENSITY_DENOM: u64 = 600;

/// A flower at a specific (x, y). Every field is derived from its own
/// hash dimension off the same (x, y) so peers compute identical
/// appearance without coordination.
///
/// 7 (petal_center) × 7 (petal_edge) × 3 (core_center) × 3 (core_edge)
/// × 4 (petal_count) = 1764 distinct kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Flower {
    pub petal_center: FlowerColor,
    pub petal_edge: FlowerColor,
    pub core_center: FlowerCore,
    pub core_edge: CoreEdge,
    pub petal_count: u8,
}

/// What a tile can carry that a player walking over it picks up.
///
/// Generic over content kind so the pickup mechanic (try_pickup,
/// canonical-vs-sandbox routing, gossip) stays kind-agnostic. Today
/// only `Flower`; v0.4 adds `Card`. Same enum doubles as inventory
/// item shape — picked-from-the-ground and held-by-the-player are
/// the same data; if a divergence ever appears, split then.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Pickup {
    Flower(Flower),
}

/// Generic pickup probe: what (if anything) is on tile (x, y) for the
/// player to pick up. For this slice: a thin wrapper around `flower_at`
/// — adding `Card` later is one more `or_else` line + a new variant.
pub fn pickup_at(x: i32, y: i32) -> Option<Pickup> {
    flower_at(x, y).map(Pickup::Flower)
}

/// Deterministic flower at (x, y). Returns None for water columns,
/// polar ocean, or tiles where the presence-hash gate doesn't fire.
/// Once Some, every field of the returned Flower is fully determined —
/// no further None-checking and no defaults required by callers.
pub fn flower_at(x: i32, y: i32) -> Option<Flower> {
    if y.abs() > WORLD_Y_LAT {
        return None;
    }
    if surface_z(x, y) < 0 {
        return None;
    }
    let presence_hash = world_hash(x, y, HashDimension::FlowerPresence);
    if !presence_hash.is_multiple_of(FLOWER_DENSITY_DENOM) {
        return None;
    }
    // Petal center: an extra splitmix round on the presence hash so
    // it decorrelates from the density gate but shares its derivation.
    let petal_center = pick_weighted(
        splitmix64(presence_hash) >> 32,
        &FLOWER_COLOR_WEIGHTS,
        FLOWER_COLOR_WEIGHTS_TOTAL,
    );
    let petal_edge = pick_weighted(
        world_hash(x, y, HashDimension::FlowerPetalEdge) >> 32,
        &FLOWER_COLOR_WEIGHTS,
        FLOWER_COLOR_WEIGHTS_TOTAL,
    );
    let core_center = pick_weighted(
        world_hash(x, y, HashDimension::FlowerCoreCenter) >> 32,
        &FLOWER_CORE_WEIGHTS,
        FLOWER_CORE_WEIGHTS_TOTAL,
    );
    let core_edge = pick_weighted(
        world_hash(x, y, HashDimension::FlowerCoreEdge) >> 32,
        &CORE_EDGE_WEIGHTS,
        CORE_EDGE_WEIGHTS_TOTAL,
    );
    let petal_count = pick_weighted(
        world_hash(x, y, HashDimension::FlowerPetalCount) >> 32,
        &PETAL_COUNT_WEIGHTS,
        PETAL_COUNT_WEIGHTS_TOTAL,
    );
    Some(Flower {
        petal_center,
        petal_edge,
        core_center,
        core_edge,
        petal_count,
    })
}

// ----- tiles -----

pub fn tile_at(x: i32, y: i32, z: i32) -> TileKind {
    if !(Z_MIN..=Z_MAX).contains(&z) {
        return TileKind::Air;
    }
    let sz = surface_z(x, y);

    // Below sea level: column has water from sz+1 up to 0.
    // Water-surface tile (z=0) is Shallow if depth ≤ 1, else Deep.
    if sz < 0 {
        let water_depth = -sz;
        if z == 0 {
            return if water_depth <= 1 {
                TileKind::ShallowWater
            } else {
                TileKind::DeepWater
            };
        }
        if z > 0 {
            return TileKind::Air;
        }
        if z > sz {
            return TileKind::DeepWater; // underwater body
        }
        // z <= sz: lake bed and below. Caves apply.
    } else {
        // Land column.
        if z > sz {
            return TileKind::Air;
        }
        if z == sz {
            return TileKind::Grass;
        }
        // z < sz: subsurface. Caves apply.
    }

    // Subsurface: cave-carving 3D noise. Threshold tuned for ~30% open space.
    let cseed = WORLD_SEED ^ HashDimension::TerrainCaveCarve.salt();
    let cx = canonical_x(x);
    let cave_x_period = (WORLD_CIRC_X as f32 * CAVE_NOISE_FREQUENCY) as i32;
    let n = fractal_3d(
        cseed,
        cx as f32 * CAVE_NOISE_FREQUENCY,
        y as f32 * CAVE_NOISE_FREQUENCY,
        z as f32 * CAVE_NOISE_FREQUENCY,
        CAVE_NOISE_OCTAVES,
        cave_x_period,
    );
    if n > CAVE_OPEN_THRESHOLD {
        TileKind::Air
    } else {
        TileKind::Rock
    }
}

// Position-dependent day phase. Returns value in [0, 1):
//   0.0 = dawn, 0.25 = noon, 0.5 = dusk, 0.75 = midnight.
// Longitude shifts the phase — sun sweeps east-to-west.
pub fn day_phase(now_unix_secs: u64, x: i32) -> f32 {
    let global = (now_unix_secs % DAY_LENGTH_SECS) as f32 / DAY_LENGTH_SECS as f32;
    let lon = canonical_x(x) as f32 / WORLD_CIRC_X as f32;
    (global + lon).rem_euclid(1.0)
}

// Brightness from phase: cosine wave, 1.0 at noon, 0.0 at midnight.
pub fn brightness(phase: f32) -> f32 {
    use core::f32::consts::TAU;
    let theta = (phase - 0.25) * TAU;
    (theta.cos() + 1.0) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_seed_matches_invocation() {
        // The locked SHA-256 of INVOCATION must produce WORLD_SEED.
        // This test re-derives it; if anyone edits INVOCATION, this fails.
        let want_hex = "5adea7cf0c0a1b2bdd3a86c65f41ea81f724adb6b891436774d153a82f0c9649";
        // Cheap re-derivation: hardcode the first 8 LE bytes from `want_hex`.
        let first_8: [u8; 8] = [0x5a, 0xde, 0xa7, 0xcf, 0x0c, 0x0a, 0x1b, 0x2b];
        let derived = u64::from_le_bytes(first_8);
        assert_eq!(derived, WORLD_SEED);
        // Confirm the hex starts with the expected bytes.
        assert!(want_hex.starts_with("5adea7cf0c0a1b2b"));
    }

    #[test]
    fn surface_z_is_deterministic() {
        let a = surface_z(123, -456);
        let b = surface_z(123, -456);
        assert_eq!(a, b);
    }

    #[test]
    fn surface_z_continuous_across_seam() {
        // The seam diff must be no worse than the rest of the world:
        // with terracing, adjacent tiles step by 0 or TERRACE_STEP.
        for y in -200..200 {
            let left = surface_z(WORLD_CIRC_X - 1, y);
            let right = surface_z(0, y);
            let diff = (left - right).abs();
            assert!(
                diff <= TERRACE_STEP,
                "seam discontinuity at y={y}: surface_z({},y)={left}, surface_z(0,y)={right}, |Δ|={diff}",
                WORLD_CIRC_X - 1
            );
        }
    }

    #[test]
    fn most_terrain_is_walkable() {
        let mut walkable = 0;
        let mut total = 0;
        for x in -100..100 {
            for y in -100..100 {
                let a = surface_z(x, y);
                let b = surface_z(x + 1, y);
                if (a - b).abs() <= 1 { walkable += 1; }
                total += 1;
            }
        }
        let frac = walkable as f32 / total as f32;
        assert!(frac > 0.85, "only {:.1}% walkable adjacency in 200x200; expected > 85%", frac * 100.0);
    }

    #[test]
    fn cliffs_exist_but_sparse() {
        let mut cliffs = 0;
        let mut total = 0;
        for x in -200..200 {
            for y in -200..200 {
                let a = surface_z(x, y);
                let b = surface_z(x + 1, y);
                if (a - b).abs() > 1 { cliffs += 1; }
                total += 1;
            }
        }
        let frac = cliffs as f32 / total as f32;
        assert!(cliffs > 0, "no cliffs in 400x400 sample — mask never triggers");
        assert!(frac < 0.10, "cliff fraction {:.2}% too high; should be sparse", frac * 100.0);
    }

    #[test]
    fn surface_z_wraps_in_x() {
        let here = surface_z(7, 100);
        let wrapped = surface_z(7 + WORLD_CIRC_X, 100);
        assert_eq!(here, wrapped);
        let wrapped_neg = surface_z(7 - WORLD_CIRC_X, 100);
        assert_eq!(here, wrapped_neg);
    }

    #[test]
    fn surface_z_within_range() {
        for x in -200..200 {
            for y in -200..200 {
                let z = surface_z(x, y);
                if y.abs() > WORLD_Y_LAT {
                    assert_eq!(z, Z_MIN);
                } else {
                    assert!((SURFACE_Z_MIN..=SURFACE_Z_MAX).contains(&z), "surface_z({x},{y}) = {z}");
                }
            }
        }
    }

    #[test]
    fn tile_at_top_of_air_above_surface() {
        for x in -50..50 {
            for y in -50..50 {
                let sz = surface_z(x, y);
                let air_z = sz.max(0) + 1;
                assert_eq!(tile_at(x, y, air_z), TileKind::Air, "expected Air at ({x},{y},{air_z}) sz={sz}");
            }
        }
    }

    #[test]
    fn tile_at_water_at_sea_level_below_sea() {
        let mut found = false;
        'outer: for x in 0..200 {
            for y in 0..200 {
                let sz = surface_z(x, y);
                if sz < 0 {
                    let t = tile_at(x, y, 0);
                    assert!(matches!(t, TileKind::ShallowWater | TileKind::DeepWater),
                        "expected water at ({x},{y},0) sz={sz}, got {t:?}");
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "no water column found in 200x200 — surface noise tuned wrong");
    }

    #[test]
    fn tile_at_polar_zone_is_ocean() {
        let y = WORLD_Y_LAT + 100;
        let t = tile_at(0, y, 0);
        assert_eq!(t, TileKind::DeepWater);
    }

    #[test]
    fn day_phase_wraps_with_longitude() {
        let a = day_phase(1_700_000_000, 100);
        let b = day_phase(1_700_000_000, 100 + WORLD_CIRC_X);
        assert!((a - b).abs() < 1e-6);
    }

    #[test]
    fn brightness_peaks_at_noon() {
        let noon = brightness(0.25);
        let dawn = brightness(0.0);
        let dusk = brightness(0.5);
        let midnight = brightness(0.75);
        assert!(noon > 0.99);
        assert!(midnight < 0.01);
        assert!((dawn - 0.5).abs() < 1e-4);
        assert!((dusk - 0.5).abs() < 1e-4);
    }

    #[test]
    fn hash_dimensions_have_distinct_salts() {
        // If two dimensions hash to the same salt, their values are
        // perfectly correlated — a worldgen bug. FNV-1a over distinct
        // variant names should never collide.
        let dims = [
            HashDimension::FlowerPresence,
            HashDimension::FlowerPetalEdge,
            HashDimension::FlowerCoreCenter,
            HashDimension::FlowerCoreEdge,
            HashDimension::FlowerPetalCount,
            HashDimension::TerrainBreakNoise,
            HashDimension::TerrainCaveCarve,
        ];
        for i in 0..dims.len() {
            for j in (i + 1)..dims.len() {
                assert_ne!(
                    dims[i].salt(),
                    dims[j].salt(),
                    "salt collision between {:?} and {:?}",
                    dims[i],
                    dims[j]
                );
                assert_ne!(
                    dims[i].mult(),
                    dims[j].mult(),
                    "mult collision between {:?} and {:?}",
                    dims[i],
                    dims[j]
                );
            }
        }
    }

    #[test]
    fn hash_dimension_mult_is_odd() {
        // Odd multiplier keeps the multiply step bijective. The `| 1`
        // in HashDimension::mult guarantees this.
        for dim in [
            HashDimension::FlowerPresence,
            HashDimension::FlowerPetalEdge,
            HashDimension::FlowerCoreCenter,
            HashDimension::FlowerCoreEdge,
            HashDimension::FlowerPetalCount,
            HashDimension::TerrainBreakNoise,
            HashDimension::TerrainCaveCarve,
        ] {
            assert_eq!(dim.mult() & 1, 1, "{:?}.mult() must be odd", dim);
        }
    }

    #[test]
    fn pick_weighted_distribution_matches_weights() {
        // Sample 100k uniform hashes and check that the empirical
        // distribution matches FLOWER_COLOR_WEIGHTS within ±2%.
        let mut counts = [0u32; 7];
        for i in 0..100_000_u64 {
            let r = splitmix64(i);
            let c = pick_weighted(r, &FLOWER_COLOR_WEIGHTS, FLOWER_COLOR_WEIGHTS_TOTAL);
            counts[c as usize] += 1;
        }
        for (color, weight) in FLOWER_COLOR_WEIGHTS {
            let observed = counts[color as usize] as f32 / 100_000.0;
            let expected = weight as f32 / FLOWER_COLOR_WEIGHTS_TOTAL as f32;
            let diff = (observed - expected).abs();
            assert!(
                diff < 0.02,
                "color {:?}: observed {:.3} expected {:.3}",
                color,
                observed,
                expected,
            );
        }
    }

    #[test]
    fn flower_at_deterministic() {
        let a = flower_at(123, -456);
        let b = flower_at(123, -456);
        assert_eq!(a, b);
    }

    /// `pickup_at` is the v0.4 generic surface: a tile may carry a
    /// `Pickup` (flower today, card next). For this slice — flowers
    /// only — `pickup_at` must agree with `flower_at` everywhere:
    /// flower tile → `Some(Pickup::Flower(f))` with the same `f`;
    /// empty tile → `None`. Falsifies the regression where the
    /// abstraction silently picks a different presence rule or loses
    /// fields off the wrapped `Flower`.
    #[test]
    fn pickup_at_parity_with_flower_at() {
        for ty in -20..=20 {
            for tx in 0..100 {
                let flower = flower_at(tx, ty);
                let pickup = pickup_at(tx, ty);
                match (flower, pickup) {
                    (Some(f), Some(Pickup::Flower(g))) => assert_eq!(
                        f, g,
                        "pickup_at({tx}, {ty}) carried a different Flower than flower_at"
                    ),
                    (None, None) => {}
                    (Some(_), None) => panic!("pickup_at({tx}, {ty}) lost a flower"),
                    (None, Some(_)) => panic!("pickup_at({tx}, {ty}) invented a pickup"),
                }
            }
        }
    }

    #[test]
    fn flower_at_density_in_range() {
        // Over a 200x200 land sample, density should be near 1/60.
        // Allow generous slack since the gate is non-uniform per column.
        let mut flowers = 0;
        let mut land = 0;
        for x in 0..200 {
            for y in 0..200 {
                if surface_z(x, y) >= 0 {
                    land += 1;
                    if flower_at(x, y).is_some() {
                        flowers += 1;
                    }
                }
            }
        }
        let density = flowers as f32 / land as f32;
        let expected = 1.0 / FLOWER_DENSITY_DENOM as f32;
        assert!(
            (density - expected).abs() < expected,
            "flower density {density:.4} far from expected {expected:.4}"
        );
    }
}
