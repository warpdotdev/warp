use std::collections::HashSet;

use rand::seq::SliceRandom;
use rand::Rng;

/// Desert/southwest-themed words used to generate memorable worktree branch names.
/// Deduplicated across six categories.
const WORDS: &[&str] = &[
    // ── Landforms & Terrain ─────────────────────────────────────────────────
    "alcove",
    "arch",
    "arroyo",
    "badlands",
    "bajada",
    "basin",
    "bluff",
    "bolson",
    "butte",
    "caldera",
    "canyon",
    "caprock",
    "chasm",
    "chimney",
    "cinder",
    "coulee",
    "crag",
    "crater",
    "cuesta",
    "dune",
    "escarpment",
    "flats",
    "gap",
    "gorge",
    "gulch",
    "hogback",
    "inselberg",
    "lava",
    "ledge",
    "malpais",
    "mesa",
    "mogote",
    "monolith",
    "notch",
    "outcrop",
    "pass",
    "pediment",
    "pinnacle",
    "plateau",
    "playa",
    "ravine",
    "ridge",
    "rimrock",
    "saddle",
    "scree",
    "spire",
    "switchback",
    "talus",
    "tepui",
    "wash",
    // ── Desert Plants ───────────────────────────────────────────────────────
    "agave",
    "barrel",
    "brittlebush",
    "cactus",
    "candelilla",
    "chamisa",
    "chaparral",
    "cholla",
    "claret",
    "creosote",
    "fishhook",
    "hedgehog",
    "ironwood",
    "jojoba",
    "joshua",
    "juniper",
    "lechuguilla",
    "lupine",
    "madrone",
    "mallow",
    "manzanita",
    "mariposa",
    "mesquite",
    "ocotillo",
    "organ-pipe",
    "palo-verde",
    "pinyon",
    "prickly",
    "rabbitbrush",
    "sagebrush",
    "saguaro",
    "saltbush",
    "sotol",
    "tumbleweed",
    "yucca",
    // ── Desert Animals ──────────────────────────────────────────────────────
    "armadillo",
    "badger",
    "bighorn",
    "bobcat",
    "burrowing-owl",
    "centipede",
    "coachwhip",
    "cottontail",
    "cougar",
    "coyote",
    "falcon",
    "gecko",
    "gila",
    "hawk",
    "horned-toad",
    "jackrabbit",
    "javelina",
    "kingsnake",
    "kit-fox",
    "mule-deer",
    "nighthawk",
    "prairie-dog",
    "pronghorn",
    "quail",
    "racer",
    "rattler",
    "ringtail",
    "roadrunner",
    "scorpion",
    "sidewinder",
    "swift",
    "tarantula",
    "thrasher",
    "tortoise",
    "vulture",
    "wren",
    // ── Minerals, Rocks & Metals ────────────────────────────────────────────
    "agate",
    "basalt",
    "calcite",
    "cinnabar",
    "cobalt",
    "copper",
    "feldspar",
    "flint",
    "garnet",
    "granite",
    "gypsum",
    "iron",
    "jasper",
    "limestone",
    "malachite",
    "mica",
    "obsidian",
    "onyx",
    "opal",
    "petrified",
    "pumice",
    "pyrite",
    "quartz",
    "sandstone",
    "shale",
    "tin",
    "topaz",
    "travertine",
    "turquoise",
    "zinc",
    // ── Southwest Culture & Spanish ─────────────────────────────────────────
    "acequia",
    "adobe",
    "cumbre",
    "equinox",
    "hacienda",
    "latilla",
    "luminaria",
    "metate",
    "mirador",
    "nicho",
    "olla",
    "oz",
    "petroglyph",
    "pictograph",
    "portal",
    "ramada",
    "rio",
    "ristra",
    "sierra",
    "siesta",
    "solstice",
    "tierra",
    "tinaja",
    "viga",
    // ── Weather & Sky ───────────────────────────────────────────────────────
    "brushfire",
    "corona",
    "dawn",
    "drought",
    "dry-lightning",
    "dusk",
    "dust-devil",
    "ember",
    "firestorm",
    "flash-flood",
    "haze",
    "mirage",
    "monsoon",
    "moonrise",
    "shimmer",
    "smoke",
    "starlight",
    "sundog",
    "sundowner",
    "thermal",
    "twilight",
    "wildfire",
    "zephyr",
];

/// Maximum number of random attempts at a given word count before escalating.
const MAX_RETRIES_PER_LEVEL: usize = 2;

/// Maximum word count before falling back to a numeric suffix.
const MAX_WORD_COUNT: usize = 5;

/// Generates a name with `word_count` distinct random words joined by `-`.
/// Returns `Some(name)` if a name not in `existing` is found within
/// `MAX_RETRIES_PER_LEVEL` attempts, or `None` if all attempts collided.
fn generate_name(
    word_count: usize,
    existing: &HashSet<&str>,
    rng: &mut impl Rng,
) -> Option<String> {
    for _ in 0..MAX_RETRIES_PER_LEVEL {
        let chosen: Vec<&str> = WORDS.choose_multiple(rng, word_count).copied().collect();
        let name = chosen.join("-");
        if !existing.contains(name.as_str()) {
            return Some(name);
        }
    }
    None
}

/// Generates a unique worktree branch name that does not collide with any name
/// in `existing`. Starts with 2-word names and escalates to more words on
/// collision. Accepts an explicit RNG for deterministic testing.
pub fn generate_unique_name(existing: &HashSet<&str>, rng: &mut impl Rng) -> String {
    for word_count in 2..=MAX_WORD_COUNT {
        if let Some(name) = generate_name(word_count, existing, rng) {
            return name;
        }
    }
    // Practically unreachable — 198^5 ≈ 2.9 × 10^11 possibilities.
    // Fall back to a numeric suffix as a safety net.
    let n: u32 = rng.gen();
    format!("worktree-{n}")
}

/// Generates a unique worktree branch name using the thread-local RNG.
/// This is the primary entry point for call sites.
pub fn generate_worktree_branch_name(existing: &HashSet<&str>) -> String {
    generate_unique_name(existing, &mut rand::thread_rng())
}

#[cfg(test)]
#[path = "worktree_names_tests.rs"]
mod tests;
