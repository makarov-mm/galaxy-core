//! Minimal scoped parallel-for. No external crate — just `std::thread::scope`.
//! Splits the output slice into one contiguous chunk per worker thread.

use crate::math::Vec3;

pub fn n_threads() -> usize {
    std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1)
}

/// Fill `out[i] = f(i)` for all i, across worker threads.
pub fn parallel_fill<F>(out: &mut [Vec3], f: F)
where
    F: Fn(usize) -> Vec3 + Sync,
{
    let n = out.len();
    if n == 0 {
        return;
    }
    let threads = n_threads().min(n);
    if threads <= 1 {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = f(i);
        }
        return;
    }
    let chunk = (n + threads - 1) / threads;
    let f = &f;
    std::thread::scope(|s| {
        let mut start = 0usize;
        for piece in out.chunks_mut(chunk) {
            let base = start;
            start += piece.len();
            s.spawn(move || {
                for (k, slot) in piece.iter_mut().enumerate() {
                    *slot = f(base + k);
                }
            });
        }
    });
}

/// Parallel sum of `f(i)` over 0..n.
pub fn parallel_sum<F>(n: usize, f: F) -> f64
where
    F: Fn(usize) -> f64 + Sync,
{
    if n == 0 {
        return 0.0;
    }
    let threads = n_threads().min(n);
    if threads <= 1 {
        return (0..n).map(f).sum();
    }
    let chunk = (n + threads - 1) / threads;
    let f = &f;
    let mut partials = vec![0.0f64; threads];
    std::thread::scope(|s| {
        for (t, slot) in partials.iter_mut().enumerate() {
            let base = t * chunk;
            let end = (base + chunk).min(n);
            s.spawn(move || {
                let mut acc = 0.0f64;
                for i in base..end {
                    acc += f(i);
                }
                *slot = acc;
            });
        }
    });
    partials.iter().sum()
}
