//! A lock-free parallel map, used to run cases and to build candidates.

use std::num::NonZero;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// `items.iter().map(f)`, computed on one thread per core, in the order of `items`.
///
/// Workers claim items one at a time from a shared atomic counter (no locks), so a few slow
/// items, such as cases that take many steps, do not leave the other workers idle.
pub fn map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = thread::available_parallelism()
        .map_or(1, NonZero::get)
        .min(items.len());
    let next = AtomicUsize::new(0);
    let mut results: Vec<Option<R>> = items.iter().map(|_| None).collect();
    thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else {
                            break done;
                        };
                        done.push((i, f(item)));
                    }
                })
            })
            .collect();
        for handle in handles {
            for (i, r) in handle.join().expect("worker thread panicked") {
                results[i] = Some(r);
            }
        }
    });
    results
        .into_iter()
        .map(|r| r.expect("every item is claimed by exactly one worker"))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_the_order_of_the_items() {
        let items: Vec<u64> = (0..1000).collect();
        assert_eq!(
            super::map(&items, |&i| i * i),
            items.iter().map(|&i| i * i).collect::<Vec<_>>()
        );
        assert!(super::map(&[] as &[u64], |&i| i).is_empty());
    }
}
