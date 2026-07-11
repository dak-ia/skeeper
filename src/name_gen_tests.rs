use super::*;
use rand::SeedableRng;
use rand::rngs::StdRng;

#[test]
fn same_seed_produces_same_name() {
    let mut rng1 = StdRng::seed_from_u64(42);
    let mut rng2 = StdRng::seed_from_u64(42);
    assert_eq!(random_name(&mut rng1), random_name(&mut rng2));
}

#[test]
fn different_seeds_produce_different_names() {
    let mut rng1 = StdRng::seed_from_u64(1);
    let mut rng2 = StdRng::seed_from_u64(2);
    // 同一seed同士は同一だが、異なるseedは同一名を返す可能性が理論上あるので、
    // 大量にサンプルして全てが同じにならないことを確認
    let n1: Vec<_> = (0..20).map(|_| random_name(&mut rng1)).collect();
    let n2: Vec<_> = (0..20).map(|_| random_name(&mut rng2)).collect();
    assert_ne!(n1, n2);
}

#[test]
fn format_is_lowercase_alpha_dash_alpha() {
    let mut rng = StdRng::seed_from_u64(42);
    for _ in 0..100 {
        let name = random_name(&mut rng);
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2, "expected 2 parts joined by '-': {name}");
        assert!(!parts[0].is_empty() && !parts[1].is_empty());
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "unexpected chars in name: {name}"
        );
    }
}

#[test]
fn word_lists_are_populated() {
    assert!(ADJECTIVES.len() >= 50);
    assert!(ANIMALS.len() >= 50);
}
