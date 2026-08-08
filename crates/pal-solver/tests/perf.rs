//! Manual performance scenario — heavy search shapes that the TUI can
//! trigger (deep + multiple progenitors + wild mode). Ignored by
//! default; run with:
//!
//! ```sh
//! cargo test --release -p pal-solver --test perf -- --ignored --nocapture
//! ```

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use pal_core::db;
use pal_core::model::{Gender, IvSpread, PalDb, PalName, PassiveName};
use pal_solver::child::ChildIndex;
use pal_solver::iv::{IvOdds, IvThresholds};
use pal_solver::passives::PassiveOdds;
use pal_solver::search::{BreedingGoal, OwnedPal, SearchConfig, Solver};

fn data(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(file);
    fs::read_to_string(path).unwrap()
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A deterministic ~750-profile box shaped like a late-game save:
/// wild-spawning species with a mix of junk and desirable passives,
/// Legend confined to a few legendary individuals, and a couple of
/// already-bred targets so short plans exist.
fn synthetic_box(pal_db: &PalDb, desired: &[PassiveName]) -> Vec<OwnedPal> {
    let junk: Vec<PassiveName> = pal_db
        .passives()
        .filter(|skill| skill.standard && skill.random_inheritance_allowed)
        .filter(|skill| !desired.contains(&skill.name))
        .map(|skill| skill.name.clone())
        .take(20)
        .collect();
    assert!(junk.len() >= 10, "expected a junk-passive supply");
    // Swift / Diamond Body / Eternal Engine circulate in the box;
    // Legend (desired[0]) is deliberately absent from the draw pool.
    let draw_pool: Vec<PassiveName> = desired[1..]
        .iter()
        .flat_map(|name| [name.clone(), name.clone()])
        .chain(junk.iter().cloned())
        .collect();

    let mut species: Vec<&PalName> = pal_db
        .pals()
        .filter(|pal| pal.wild_levels.is_some())
        .map(|pal| &pal.name)
        .collect();
    species.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    species.truncate(60);

    let mut rng = 0x5EED_u64;
    let mut owned = Vec::new();
    for individual in 0..12 {
        for name in &species {
            let count = usize::try_from(splitmix(&mut rng) % 5).expect("small");
            let mut passives = Vec::new();
            for _ in 0..count {
                let pick =
                    usize::try_from(splitmix(&mut rng) % draw_pool.len() as u64).expect("small");
                let passive = draw_pool[pick].clone();
                if !passives.contains(&passive) {
                    passives.push(passive);
                }
            }
            owned.push(OwnedPal {
                species: (*name).clone(),
                gender: if (individual + owned.len()) % 2 == 0 {
                    Gender::Male
                } else {
                    Gender::Female
                },
                passives,
                ivs: IvSpread::default(),
            });
        }
    }
    // Legendaries carrying Legend, and two bred Beakons so 1-3 step
    // finishes exist (the user's box has both).
    for (name, gender, extra) in [
        ("JetDragon", Gender::Male, Some(desired[2].clone())),
        ("JetDragon", Gender::Female, None),
        ("BlackCentaur", Gender::Male, None),
        ("SaintCentaur", Gender::Female, Some(desired[3].clone())),
        ("ThunderBird", Gender::Male, Some(desired[1].clone())),
        ("ThunderBird", Gender::Female, Some(desired[2].clone())),
    ] {
        let mut passives = vec![desired[0].clone()];
        passives.extend(extra);
        owned.push(OwnedPal {
            species: PalName::new(name),
            gender,
            passives,
            ivs: IvSpread::default(),
        });
    }
    owned
}

#[test]
#[ignore = "manual perf scenario; run with --ignored --nocapture in release"]
fn four_passive_goal_timing() {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();
    let breeding = db::parse_breeding_db(&data("breeding.json"), &pal_db).unwrap();
    let index = ChildIndex::build(&breeding).unwrap();
    let odds = PassiveOdds::from_mechanics(pal_db.mechanics()).unwrap();
    let iv_odds = IvOdds::from_mechanics(pal_db.mechanics()).unwrap();
    let solver = Solver::new(&pal_db, &index, &odds, &iv_odds);

    let desired: Vec<PassiveName> = ["Legend", "Swift", "Diamond Body", "Eternal Engine"]
        .into_iter()
        .map(|name| {
            pal_db
                .find_passive(name)
                .expect("known passive")
                .name
                .clone()
        })
        .collect();
    let owned = synthetic_box(&pal_db, &desired);
    println!("box: {} profiles", owned.len());

    let goal = BreedingGoal {
        species: PalName::new("ThunderBird"),
        passives: desired.clone(),
        progenitors: Vec::new(),
        iv_thresholds: IvThresholds::default(),
    };
    let depths: Vec<usize> = std::env::var("PERF_DEPTHS").map_or_else(|_| vec![3, 6, 10, 16, 24], |spec| {
            spec.split(',')
                .map(|depth| depth.trim().parse().expect("PERF_DEPTHS: usize list"))
                .collect()
        });
    for depth in depths {
        let config = SearchConfig {
            max_breeding_steps: depth,
            max_results: 5,
            allow_wild_pals: false,
        };
        let start = Instant::now();
        let plans = solver.find_paths(&owned, &goal, &config).unwrap();
        let best = plans.first().map_or_else(String::new, |plan| {
            format!(
                " best {:.2} eggs / {} steps",
                plan.expected_eggs, plan.steps
            )
        });
        println!(
            "depth {depth}: {:?} — {} plan(s){best}",
            start.elapsed(),
            plans.len(),
        );
    }
}

#[test]
#[ignore = "manual perf scenario; run with --ignored --nocapture in release"]
fn deep_progenitor_search_timing() {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();
    let breeding = db::parse_breeding_db(&data("breeding.json"), &pal_db).unwrap();
    let index = ChildIndex::build(&breeding).unwrap();
    let odds = PassiveOdds::from_mechanics(pal_db.mechanics()).unwrap();
    let iv_odds = IvOdds::from_mechanics(pal_db.mechanics()).unwrap();

    let goal = BreedingGoal {
        species: PalName::new("WingGolem"),
        passives: Vec::new(),
        progenitors: vec![
            PalName::new("SheepBall"),
            PalName::new("PinkCat"),
            PalName::new("BlueThunderHorse"),
        ],
        iv_thresholds: IvThresholds::default(),
    };
    let config = SearchConfig {
        max_breeding_steps: 6,
        max_results: 5,
        allow_wild_pals: true,
    };

    let setup = Instant::now();
    let solver = Solver::new(&pal_db, &index, &odds, &iv_odds);
    println!("solver setup: {:?}", setup.elapsed());

    for depth in [6, 12, 20, 32] {
        let config = SearchConfig {
            max_breeding_steps: depth,
            ..config
        };
        let start = Instant::now();
        let plans = solver.find_paths(&[], &goal, &config).unwrap();
        println!(
            "depth {depth}: {:?} — {} plan(s), best {:.2} eggs / {} steps",
            start.elapsed(),
            plans.len(),
            plans[0].expected_eggs,
            plans[0].steps
        );
    }
}
