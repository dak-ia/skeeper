use std::collections::HashSet;
use std::hash::BuildHasher;

use rand::Rng;
use rand::seq::IteratorRandom;

#[rustfmt::skip]
pub(crate) const ADJECTIVES: &[&str] = &[
    "brave", "clever", "curious", "eager", "gentle", "happy", "jolly", "kind", "lively", "merry",
    "noble", "proud", "quick", "quiet", "silent", "swift", "tender", "upbeat", "valiant", "warm",
    "wise", "calm", "cheerful", "dandy", "fluffy", "glossy", "honest", "keen", "loyal", "mellow",
    "nimble", "plucky", "refined", "serene", "sturdy", "tidy", "vibrant", "witty", "zesty", "bold",
    "bright", "breezy", "cool", "dashing", "elegant", "fair", "fond", "graceful", "humble", "ideal",
    "lucky", "mighty", "neat", "radiant", "snappy", "sunny", "timid", "unique", "vivid", "whimsical",
    "young", "zealous", "agile", "bubbly", "chipper", "daring", "epic", "fancy", "glad", "hearty",
];

#[rustfmt::skip]
pub(crate) const ANIMALS: &[&str] = &[
    "otter", "sparrow", "fox", "panda", "badger", "beaver", "cat", "dog", "duck", "eagle",
    "falcon", "gecko", "hawk", "iguana", "jaguar", "koala", "lion", "mole", "newt", "owl",
    "parrot", "quail", "rabbit", "seal", "tiger", "urchin", "viper", "wolf", "yak", "zebra",
    "antelope", "bison", "camel", "dove", "elk", "ferret", "gopher", "heron", "ibis", "jay",
    "kangaroo", "lemur", "marten", "narwhal", "ocelot", "penguin", "raccoon", "salamander", "turtle", "walrus",
    "alpaca", "chinchilla", "dolphin", "echidna", "flamingo", "giraffe", "hedgehog", "koi", "lobster", "meerkat",
];

pub(crate) const TOTAL_NAMES: usize = 70 * 60;

pub fn pick_available_name<R: Rng + ?Sized, S: BuildHasher>(
    rng: &mut R,
    taken: &HashSet<String, S>,
) -> Option<String> {
    ADJECTIVES
        .iter()
        .flat_map(|adj| ANIMALS.iter().map(move |animal| format!("{adj}-{animal}")))
        .filter(|name| !taken.contains(name))
        .choose(rng)
}

#[cfg(test)]
#[path = "name_gen_tests.rs"]
mod tests;
