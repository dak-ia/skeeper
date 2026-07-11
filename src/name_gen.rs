use rand::Rng;
use rand::seq::IndexedRandom;

#[rustfmt::skip]
const ADJECTIVES: &[&str] = &[
    "brave", "clever", "curious", "eager", "gentle", "happy", "jolly", "kind", "lively", "merry",
    "noble", "proud", "quick", "quiet", "silent", "swift", "tender", "upbeat", "valiant", "warm",
    "wise", "calm", "cheerful", "dandy", "fluffy", "glossy", "honest", "keen", "loyal", "mellow",
    "nimble", "plucky", "refined", "serene", "sturdy", "tidy", "vibrant", "witty", "zesty", "bold",
    "bright", "breezy", "cool", "dashing", "elegant", "fair", "fond", "graceful", "humble", "ideal",
    "lucky", "mighty", "neat", "radiant", "snappy", "sunny", "timid", "unique", "vivid", "whimsical",
    "young", "zealous", "agile", "bubbly", "chipper", "daring", "epic", "fancy", "glad", "hearty",
];

#[rustfmt::skip]
const ANIMALS: &[&str] = &[
    "otter", "sparrow", "fox", "panda", "badger", "beaver", "cat", "dog", "duck", "eagle",
    "falcon", "gecko", "hawk", "iguana", "jaguar", "koala", "lion", "mole", "newt", "owl",
    "parrot", "quail", "rabbit", "seal", "tiger", "urchin", "viper", "wolf", "yak", "zebra",
    "antelope", "bison", "camel", "dove", "elk", "ferret", "gopher", "heron", "ibis", "jay",
    "kangaroo", "lemur", "marten", "narwhal", "ocelot", "penguin", "raccoon", "salamander", "turtle", "walrus",
    "alpaca", "chinchilla", "dolphin", "echidna", "flamingo", "giraffe", "hedgehog", "koi", "lobster", "meerkat",
];

pub fn random_name<R: Rng + ?Sized>(rng: &mut R) -> String {
    let adj = ADJECTIVES.choose(rng).expect("ADJECTIVES is non-empty");
    let animal = ANIMALS.choose(rng).expect("ANIMALS is non-empty");
    format!("{adj}-{animal}")
}

#[cfg(test)]
#[path = "name_gen_tests.rs"]
mod tests;
