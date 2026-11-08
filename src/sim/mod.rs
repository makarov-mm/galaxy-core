//! Gravitational N-body simulation. Pure Rust, no external crates.

pub mod galaxy;
mod octree;
mod parallel;
pub mod worker;

use crate::math::Vec3;
use octree::{Cube, Interactions, Octree};
use parallel::parallel_ranges;

/// Body state as separate arrays (structure of arrays).
#[derive(Clone, Debug, Default)]
pub struct Bodies {
    pub pos: Vec<Vec3>,
    pub vel: Vec<Vec3>,
    pub mass: Vec<f32>,
}

impl Bodies {
    pub fn with_capacity(n: usize) -> Bodies {
        Bodies {
            pos: Vec::with_capacity(n),
            vel: Vec::with_capacity(n),
            mass: Vec::with_capacity(n),
        }
    }

    pub fn push(&mut self, pos: Vec3, vel: Vec3, mass: f32) {
        self.pos.push(pos);
        self.vel.push(vel);
        self.mass.push(mass);
    }

    pub fn append(&mut self, mut other: Bodies) {
        self.pos.append(&mut other.pos);
        self.vel.append(&mut other.vel);
        self.mass.append(&mut other.mass);
    }

    pub fn len(&self) -> usize {
        self.pos.len()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SimParams {
    pub dt: f32,    // time step
    pub theta: f32, // Barnes-Hut opening angle
    pub eps: f32,   // Plummer softening length
}

/// N-body state integrated with kick-drift-kick leapfrog.
///
/// Bodies are kept in Morton order (re-sorted every force evaluation), so body
/// indices are not stable across steps; nothing outside needs them to be.
pub struct Sim {
    bodies: Bodies,
    acc: Vec<Vec3>,
    params: SimParams,
    tree: Octree,
    // Scratch buffers reused every step.
    keys: Vec<(u64, u32)>,
    codes: Vec<u64>,
    scratch_vec: Vec<Vec3>,
    scratch_f32: Vec<f32>,
}

impl Sim {
    pub fn new(bodies: Bodies, params: SimParams) -> Sim {
        let n = bodies.len();
        let mut sim = Sim {
            bodies,
            acc: vec![Vec3::ZERO; n],
            params,
            tree: Octree::new(params.theta, params.eps),
            keys: Vec::with_capacity(n),
            codes: Vec::with_capacity(n),
            scratch_vec: Vec::with_capacity(n),
            scratch_f32: Vec::with_capacity(n),
        };
        sim.compute_acc();
        sim
    }

    pub fn positions(&self) -> &[Vec3] {
        &self.bodies.pos
    }

    pub fn velocities(&self) -> &[Vec3] {
        &self.bodies.vel
    }

    /// One leapfrog step. The acceleration from the previous step serves the
    /// first half-kick; that reuse is what makes leapfrog symplectic and keeps
    /// the energy error bounded instead of drifting.
    pub fn step(&mut self) {
        let dt = self.params.dt;
        let half = 0.5 * dt;
        let b = &mut self.bodies;
        for ((p, v), &a) in b.pos.iter_mut().zip(&mut b.vel).zip(&self.acc) {
            *v += a * half; // half kick
            *p += *v * dt; // drift
        }
        self.compute_acc(); // forces at the new positions
        for (v, &a) in self.bodies.vel.iter_mut().zip(&self.acc) {
            *v += a * half; // half kick
        }
    }

    /// Re-sorts the bodies along a Morton curve, rebuilds the tree and
    /// evaluates every acceleration in parallel, one tree leaf at a time.
    fn compute_acc(&mut self) {
        let cube = self.sort_bodies();
        let Bodies { pos, mass, .. } = &self.bodies;
        self.tree.build(&self.codes, pos, mass, cube);
        let tree = &self.tree;
        parallel_ranges(
            &mut self.acc,
            tree.leaves(),
            Interactions::default,
            |list, leaf, out| {
                tree.leaf_accelerations(leaf, pos, mass, list, out);
            },
        );
    }

    /// Puts the bodies in Morton order (bodies move little per step, so this
    /// mostly re-sorts an already ordered sequence) and fills `codes`.
    fn sort_bodies(&mut self) -> Cube {
        let cube = Cube::bounding(&self.bodies.pos);
        self.keys.clear();
        self.keys.extend(
            self.bodies
                .pos
                .iter()
                .enumerate()
                .map(|(i, &p)| (cube.morton_code(p), i as u32)),
        );
        self.keys.sort_unstable();

        let order = &self.keys;
        gather(&mut self.bodies.pos, order, &mut self.scratch_vec);
        gather(&mut self.bodies.vel, order, &mut self.scratch_vec);
        gather(&mut self.bodies.mass, order, &mut self.scratch_f32);
        self.codes.clear();
        self.codes.extend(order.iter().map(|&(code, _)| code));
        cube
    }
}

/// Reorders `data` so that `data[k] = old_data[order[k].1]`.
fn gather<T: Copy>(data: &mut Vec<T>, order: &[(u64, u32)], scratch: &mut Vec<T>) {
    scratch.clear();
    scratch.extend(order.iter().map(|&(_, i)| data[i as usize]));
    std::mem::swap(data, scratch);
}
