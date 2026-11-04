use super::*;
use proptest::prelude::*;

fn release() -> impl Strategy<Value = Release> {
    (0u8..4, 0u8..4, 0u8..6, 1u8..5, any::<bool>()).prop_map(
        |(minor, patch, suffix, n, prerelease)| {
            let suffix = ["", "alpha", "beta", "rc", "prerelease", "nightly"][suffix as usize];
            let tag = if suffix.is_empty() {
                format!("v0.{minor}.{patch}")
            } else {
                format!("v0.{minor}.{patch}-{suffix}.{n}")
            };
            Release::new(tag, prerelease)
        },
    )
}

fn release_list() -> impl Strategy<Value = (Vec<Release>, Vec<usize>)> {
    prop::collection::vec(release(), 0..24)
        .prop_map(|releases| {
            // GitHub has one release per tag; conflicting promotion flags on
            // duplicate tags are not a reachable published release list.
            let mut seen = std::collections::HashSet::new();
            releases
                .into_iter()
                .filter(|r| seen.insert(r.tag.clone()))
                .collect::<Vec<_>>()
        })
        .prop_flat_map(|releases| {
            let order = (0..releases.len()).collect::<Vec<_>>();
            (Just(releases), Just(order).prop_shuffle())
        })
}

proptest! {
    #[test]
    fn slots_obey_membership_precedence_and_publish_order(
        case in release_list(),
    ) {
        let (releases, order) = case;
        let shuffled: Vec<_> = order.into_iter().map(|i| releases[i].clone()).collect();
        for slot in Slot::ALL {
            let chosen = select(&releases, slot);
            prop_assert!(chosen.as_ref().is_none_or(|r| releases.contains(r)));
            prop_assert_eq!(select(&shuffled, slot), chosen);
        }
        if let Some(r) = select(&releases, Slot::STABLE_LATEST) {
            prop_assert!(!r.prerelease);
        }
        let version = |slot| select(&releases, slot).and_then(|r| r.version());
        for (floor, upper) in [(Slot::STABLE_LATEST, Slot::STABLE_PRERELEASE),
                               (Slot::BETA_LATEST, Slot::BETA_PRERELEASE)] {
            if let Some(floor) = version(floor) {
                prop_assert!(version(upper).is_some_and(|v| v >= floor));
            }
        }
        if let Some(promoted) = version(Slot::STABLE_LATEST).filter(|v| v.pre.is_empty()) {
            for slot in [Slot::BETA_LATEST, Slot::BETA_PRERELEASE] {
                prop_assert!(version(slot).is_some_and(|v| v >= promoted));
            }
        }
    }
}
