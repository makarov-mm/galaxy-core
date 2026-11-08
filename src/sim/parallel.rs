//! Minimal parallel-for on `std::thread::scope`, no external crate.
//!
//! Work is handed out in small batches from a shared queue, so threads that
//! finish early pick up more. Force evaluation is far from uniform (bodies in
//! dense regions open many more cells), and static equal splits would leave
//! cores idle waiting for the slowest one.

use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::{Mutex, OnceLock, PoisonError};

/// Items per work batch: large enough to amortise the queue lock, small
/// enough to balance load.
const CHUNK: usize = 256;

/// Worker count, queried once (on Linux `available_parallelism` reads cgroup
/// files, which is too slow to repeat every step).
pub fn thread_count() -> usize {
    static COUNT: OnceLock<usize> = OnceLock::new();
    *COUNT.get_or_init(|| std::thread::available_parallelism().map_or(1, NonZeroUsize::get))
}

/// Calls `f(scratch, range, &mut out[range])` for each of `ranges`, which must
/// partition `out` in order, using all cores. Every thread creates its own
/// `scratch` once with `init`.
pub fn parallel_ranges<T, S, I, F>(out: &mut [T], ranges: &[Range<usize>], init: I, f: F)
where
    T: Send,
    I: Fn() -> S + Sync,
    F: Fn(&mut S, Range<usize>, &mut [T]) + Sync,
{
    // Pair every range with its disjoint slice of `out`.
    let mut items = Vec::with_capacity(ranges.len());
    let mut rest = out;
    for range in ranges {
        let (head, tail) = std::mem::take(&mut rest).split_at_mut(range.len());
        items.push((range.clone(), head));
        rest = tail;
    }
    debug_assert!(rest.is_empty(), "ranges must cover the output");

    let total: usize = ranges.iter().map(Range::len).sum();
    let threads = thread_count().min(total.div_ceil(CHUNK));
    if threads <= 1 {
        let mut scratch = init();
        for (range, slots) in items {
            f(&mut scratch, range, slots);
        }
        return;
    }

    // Hand out batches of ranges worth roughly CHUNK items each.
    let per_batch = (CHUNK * ranges.len() / total.max(1)).max(1);
    let queue = Mutex::new(items.chunks_mut(per_batch));
    let work = || {
        let mut scratch = init();
        loop {
            let next = queue.lock().unwrap_or_else(PoisonError::into_inner).next();
            let Some(batch) = next else { break };
            for (range, slots) in batch {
                f(&mut scratch, range.clone(), slots);
            }
        }
    };
    std::thread::scope(|s| {
        for _ in 1..threads {
            s.spawn(work);
        }
        work();
    });
}
