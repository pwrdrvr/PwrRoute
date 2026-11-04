use super::tests::record;
use super::*;
use crate::link::Source;
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Op {
    Push(i8, bool),
    Retain(usize, u64),
    Take(usize),
    Clear,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        8 => (-2i8..30, any::<bool>()).prop_map(|(age, link)| Op::Push(age, link)),
        2 => (0usize..8, 0u64..30).prop_map(|(count, hours)| Op::Retain(count, hours)),
        2 => (0usize..8).prop_map(Op::Take),
        1 => Just(Op::Clear),
    ]
}

fn run(start: Retention, ops: &[Op]) -> Result<(), TestCaseError> {
    let now = SystemTime::now();
    let recent = RecentDispatches::new(start);
    let mut retention = start;
    let mut kept = Vec::<(SystemTime, u64, bool)>::new();
    for (i, op) in ops.iter().enumerate() {
        match *op {
            Op::Push(age, waiting) => {
                // Half-hour margins keep real clock execution away from an
                // age boundary; equal receipt times exercise the id tie break.
                let delta = Duration::from_secs(u64::from(age.unsigned_abs()) * 3600 + 1800);
                let at = if age >= 0 { now - delta } else { now + delta };
                let id = i as u64 + 1;
                let link = Link::new(format!("https://example.org/{id}"), Source::CommandLine);
                recent.push_waiting(record(id, at), waiting.then_some(link));
                if retention.count != 0 {
                    kept.push((at, id, waiting));
                }
            }
            Op::Retain(count, hours) => {
                retention = Retention { count, hours };
                recent.set_retention(retention);
            }
            Op::Take(n) => {
                let pending: Vec<_> = kept.iter().filter(|r| r.2).map(|r| r.1).collect();
                if !pending.is_empty() {
                    let id = pending[n % pending.len()];
                    prop_assert_eq!(
                        recent.take_pending(DispatchId(id)).map(|l| l.url),
                        Some(format!("https://example.org/{id}"))
                    );
                    kept.iter_mut()
                        .filter(|r| r.1 == id)
                        .for_each(|r| r.2 = false);
                }
            }
            Op::Clear => {
                recent.clear();
                kept.clear();
            }
        }
        // Independent oracle: sort all retained receipts, then take the
        // newest count and discard expired records and their waiting links.
        kept.sort_by_key(|r| std::cmp::Reverse((r.0, r.1)));
        kept.truncate(retention.count);
        if retention.hours != 0 {
            let cutoff = now - Duration::from_secs(retention.hours * 3600);
            kept.retain(|r| r.0 >= cutoff);
        }
        prop_assert_eq!(
            recent.snapshot().iter().map(|r| r.id.0).collect::<Vec<_>>(),
            kept.iter().map(|r| r.1).collect::<Vec<_>>(),
            "after {:?}",
            op
        );
        prop_assert_eq!(
            recent.pending().iter().map(|id| id.0).collect::<Vec<_>>(),
            kept.iter().filter(|r| r.2).map(|r| r.1).collect::<Vec<_>>(),
            "after {:?}",
            op
        );
    }
    Ok(())
}

proptest! {
    #[test]
    fn completion_order_does_not_change_receipt_retention(
        count in 0usize..8, hours in 0u64..30,
        ops in prop::collection::vec(op(), 1..40),
    ) { run(Retention { count, hours }, &ops)?; }
}

#[test]
fn a_late_old_completion_cannot_evict_a_newer_receipt() {
    run(
        Retention { count: 1, hours: 0 },
        &[Op::Push(0, true), Op::Push(1, true)],
    )
    .unwrap();
}

fn receipts() -> impl Strategy<Value = (Vec<(i8, bool)>, Vec<usize>)> {
    prop::collection::vec((-2i8..30, any::<bool>()), 1..32).prop_flat_map(|records| {
        let order = (0..records.len()).collect::<Vec<_>>();
        (Just(records), Just(order).prop_shuffle())
    })
}

proptest! {
    #[test]
    fn receipt_ids_and_timestamps_survive_arbitrary_completion_permutations(
        receipts in receipts(), count in 0usize..16, hours in 0u64..30,
    ) {
        let now = SystemTime::now();
        let recent = RecentDispatches::new(Retention { count, hours });
        let (receipts, order) = receipts;
        let mut delivered = Vec::new();
        for i in order {
            let (age, waits) = receipts[i];
            let delta = Duration::from_secs(u64::from(age.unsigned_abs()) * 3600 + 1800);
            let at = if age >= 0 { now - delta } else { now + delta };
            let id = i as u64 + 1;
            let rec = record(id, at);
            let link = waits.then(|| rec.link.clone());
            recent.push_waiting(rec, link);
            delivered.push((at, id, waits));
            delivered.sort_by_key(|r| std::cmp::Reverse((r.0, r.1)));
            delivered.truncate(count);
            if hours != 0 { delivered.retain(|r| r.0 >= now - Duration::from_secs(hours * 3600)); }
            prop_assert_eq!(recent.snapshot().iter().map(|r| r.id.0).collect::<Vec<_>>(),
                delivered.iter().map(|r| r.1).collect::<Vec<_>>());
            prop_assert_eq!(recent.pending().iter().map(|r| r.0).collect::<Vec<_>>(),
                delivered.iter().filter(|r| r.2).map(|r| r.1).collect::<Vec<_>>());
        }
    }
}

#[test]
fn equal_receipt_times_keep_the_newer_id_when_it_completes_first() {
    let recent = RecentDispatches::new(Retention { count: 1, hours: 0 });
    let now = SystemTime::now();
    recent.push(record(2, now));
    recent.push(record(1, now));
    assert_eq!(
        recent.snapshot().iter().map(|r| r.id.0).collect::<Vec<_>>(),
        vec![2]
    );
}
