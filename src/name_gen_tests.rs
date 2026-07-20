use super::*;

use rand::SeedableRng;
use rand::rngs::StdRng;

fn all_names() -> HashSet<String> {
    ADJECTIVES
        .iter()
        .flat_map(|adj| ANIMALS.iter().map(move |animal| format!("{adj}-{animal}")))
        .collect()
}

#[test]
fn word_lists_are_populated() {
    assert!(ADJECTIVES.len() >= 50);
    assert!(ANIMALS.len() >= 50);
}

#[test]
fn pool_size_matches_declared_total() {
    assert_eq!(all_names().len(), TOTAL_NAMES);
}

#[test]
fn pick_returns_adj_animal_format() {
    let mut rng = StdRng::seed_from_u64(0);
    let name = pick_available_name(&mut rng, &HashSet::new()).unwrap();
    let parts: Vec<&str> = name.split('-').collect();
    assert_eq!(parts.len(), 2);
    assert!(ADJECTIVES.contains(&parts[0]));
    assert!(ANIMALS.contains(&parts[1]));
}

#[test]
fn pick_returns_none_when_all_taken() {
    let mut rng = StdRng::seed_from_u64(0);
    assert!(pick_available_name(&mut rng, &all_names()).is_none());
}

#[test]
fn pick_returns_the_last_free_deterministically() {
    let target = "brave-otter";
    let mut taken = all_names();
    taken.remove(target);
    let mut rng = StdRng::seed_from_u64(0);
    assert_eq!(
        pick_available_name(&mut rng, &taken).as_deref(),
        Some(target)
    );
}

#[test]
fn pick_is_deterministic_with_same_seed() {
    let taken = HashSet::new();
    let mut rng1 = StdRng::seed_from_u64(42);
    let mut rng2 = StdRng::seed_from_u64(42);
    assert_eq!(
        pick_available_name(&mut rng1, &taken),
        pick_available_name(&mut rng2, &taken)
    );
}

#[test]
fn pick_produces_variety_across_seeds() {
    let taken = HashSet::new();
    let names: HashSet<String> = (0..50)
        .filter_map(|seed| {
            let mut rng = StdRng::seed_from_u64(seed);
            pick_available_name(&mut rng, &taken)
        })
        .collect();
    // 50 seedsでも同一名だけになるほど偏らない事だけ確認(閾値は緩めで良い)
    assert!(names.len() > 5, "expected variety, got {names:?}");
}

#[test]
fn pick_never_returns_taken_name() {
    let mut rng = StdRng::seed_from_u64(0);
    // 4199名を埋めた状態で1000回試して残り1個以外に当たらない事を確認
    let mut taken = all_names();
    let target = "brave-otter";
    taken.remove(target);
    for _ in 0..1000 {
        let picked = pick_available_name(&mut rng, &taken).unwrap();
        assert_eq!(picked, target);
    }
}
