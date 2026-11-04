//! Random `adjective-noun` session slugs.

use rand::seq::SliceRandom;

const ADJECTIVES: &[&str] = &[
    "brave", "calm", "clever", "cosmic", "crisp", "eager", "gentle", "golden", "happy", "hidden",
    "lucky", "mighty", "neon", "nimble", "proud", "quiet", "rapid", "shiny", "silent", "swift",
    "tidy", "witty",
];
const NOUNS: &[&str] = &[
    "canyon", "comet", "falcon", "forest", "galaxy", "harbor", "island", "lagoon", "meadow",
    "nebula", "orchid", "otter", "panda", "pixel", "planet", "river", "rocket", "summit", "tiger",
    "valley", "willow", "wizard",
];

pub fn create() -> String {
    let mut rng = rand::thread_rng();
    format!(
        "{}-{}",
        ADJECTIVES.choose(&mut rng).unwrap_or(&"calm"),
        NOUNS.choose(&mut rng).unwrap_or(&"river")
    )
}
