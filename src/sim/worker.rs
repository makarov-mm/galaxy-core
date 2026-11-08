//! Runs the simulation on a background thread, so rendering and camera input
//! stay smooth even when a step takes longer than a frame.
//!
//! The render thread ticks the worker once per frame; the worker then runs
//! `substeps` steps and publishes a snapshot. A worker faster than the display
//! waits for the next tick (so "substeps per frame" keeps its meaning); a
//! slower one runs flat out and the renderer shows the newest snapshot.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use super::galaxy::Scene;
use super::{Sim, SimParams};

/// Integration settings for every scene. With the group walk, theta = 0.8 is
/// already more accurate than plain per-body Barnes-Hut at theta = 0.6.
pub const SIM_PARAMS: SimParams = SimParams {
    dt: 0.0015,
    theta: 0.8,
    eps: 0.5,
};

/// One star as uploaded to the GPU: position and speed.
pub type Star = [f32; 4];

/// The newest simulation state, ready for upload.
#[derive(Default)]
pub struct Snapshot {
    pub stars: Vec<Star>,
    /// Speed mapped to the hot end of the colour ramp (fixed per scene).
    pub reference_speed: f32,
    /// Smoothed wall time of one simulation step, in milliseconds.
    pub step_ms: f32,
}

/// What the render thread asks of the worker.
struct Control {
    frames: u64, // frame ticks so far
    paused: bool,
    substeps: u32,
    reset: Option<Scene>,
    quit: bool,
}

/// The latest published snapshot and its sequence number.
#[derive(Default)]
struct Published {
    snapshot: Snapshot,
    generation: u64,
}

struct Shared {
    control: Mutex<Control>,
    wake: Condvar,
    published: Mutex<Published>,
}

pub struct SimWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    taken_generation: u64,
}

impl SimWorker {
    pub fn spawn(scene: Scene, substeps: u32) -> SimWorker {
        let shared = Arc::new(Shared {
            control: Mutex::new(Control {
                frames: 0,
                paused: false,
                substeps,
                reset: Some(scene),
                quit: false,
            }),
            wake: Condvar::new(),
            published: Mutex::new(Published::default()),
        });
        let worker_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("simulation".into())
            .spawn(move || run(&worker_shared))
            .expect("failed to spawn the simulation thread");
        SimWorker {
            shared,
            thread: Some(thread),
            taken_generation: 0,
        }
    }

    /// Lets the worker advance by one frame's worth of substeps.
    pub fn tick(&self) {
        self.update(|c| c.frames += 1);
    }

    pub fn set_paused(&self, paused: bool) {
        self.update(|c| c.paused = paused);
    }

    pub fn set_substeps(&self, substeps: u32) {
        self.update(|c| c.substeps = substeps);
    }

    /// Restarts with fresh initial conditions for `scene`.
    pub fn reset(&self, scene: Scene) {
        self.update(|c| c.reset = Some(scene));
    }

    /// If a snapshot newer than the last one taken exists, swaps its star
    /// buffer into `dst` (no copy), updates the metadata and returns true.
    pub fn take_snapshot(&mut self, dst: &mut Snapshot) -> bool {
        let mut latest = lock(&self.shared.published);
        if latest.generation == self.taken_generation {
            return false;
        }
        self.taken_generation = latest.generation;
        std::mem::swap(&mut latest.snapshot.stars, &mut dst.stars);
        dst.reference_speed = latest.snapshot.reference_speed;
        dst.step_ms = latest.snapshot.step_ms;
        true
    }

    fn update(&self, f: impl FnOnce(&mut Control)) {
        f(&mut lock(&self.shared.control));
        self.shared.wake.notify_one();
    }
}

impl Drop for SimWorker {
    fn drop(&mut self) {
        self.update(|c| c.quit = true);
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                eprintln!("the simulation thread panicked");
            }
        }
    }
}

/// Locks even a poisoned mutex: the guarded data is plain state that stays
/// consistent if a holder panicked.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn run(shared: &Shared) {
    let mut sim: Option<Sim> = None;
    let mut reference_speed = 1.0;
    let mut step_ms = 0.0;
    let mut seen_frames = 0;
    let mut stars = Vec::new();

    loop {
        // Sleep until there is something to do.
        let (substeps, reset) = {
            let mut c = lock(&shared.control);
            while !c.quit && c.reset.is_none() && (c.paused || c.frames == seen_frames) {
                c = shared.wake.wait(c).unwrap_or_else(PoisonError::into_inner);
            }
            if c.quit {
                return;
            }
            seen_frames = c.frames;
            (c.substeps, c.reset.take())
        };

        if let Some(scene) = reset {
            let fresh = Sim::new(scene.bodies(SIM_PARAMS.eps), SIM_PARAMS);
            reference_speed = reference_speed_of(&fresh);
            step_ms = 0.0;
            sim = Some(fresh);
        } else if let Some(sim) = sim.as_mut() {
            let start = Instant::now();
            for _ in 0..substeps {
                sim.step();
            }
            if substeps > 0 {
                let ms = start.elapsed().as_secs_f32() * 1000.0 / substeps as f32;
                step_ms = if step_ms == 0.0 {
                    ms
                } else {
                    0.9 * step_ms + 0.1 * ms
                };
            }
        }

        if let Some(sim) = &sim {
            stars.clear();
            stars.extend(
                sim.positions()
                    .iter()
                    .zip(sim.velocities())
                    .map(|(p, v)| [p.x, p.y, p.z, v.length()]),
            );

            let mut published = lock(&shared.published);
            std::mem::swap(&mut published.snapshot.stars, &mut stars);
            published.snapshot.reference_speed = reference_speed;
            published.snapshot.step_ms = step_ms;
            published.generation += 1;
        }
    }
}

/// Speed at the hot end of the colour ramp: ten times the median initial
/// speed, so typical disk stars sit at the cold (blue) end and the fast core
/// glows warm. Unlike the maximum, the median does not hinge on the one or two
/// most extreme stars of a particular random draw.
fn reference_speed_of(sim: &Sim) -> f32 {
    let mut speeds: Vec<f32> = sim.velocities().iter().map(|v| v.length()).collect();
    if speeds.is_empty() {
        return 1.0;
    }
    let mid = speeds.len() / 2;
    let (_, median, _) = speeds.select_nth_unstable_by(mid, f32::total_cmp);
    (10.0 * *median).max(1e-3)
}
