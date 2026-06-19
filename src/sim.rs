//! N-body state and the leapfrog (kick-drift-kick) integrator.
//!
//! State is stored structure-of-arrays so the renderer can hand `pos` straight
//! to the GPU as an instance buffer, and so force evaluation is cache-friendly.

use crate::math::Vec3;
use crate::octree::{Octree, G};
use crate::parallel::{parallel_fill, parallel_sum};

pub struct Sim {
    pub pos: Vec<Vec3>,
    pub vel: Vec<Vec3>,
    pub mass: Vec<f32>,
    pub acc: Vec<Vec3>,
    pub dt: f32,
    pub theta: f32,
    pub eps: f32,
}

impl Sim {
    pub fn new(pos: Vec<Vec3>, vel: Vec<Vec3>, mass: Vec<f32>, dt: f32, theta: f32, eps: f32) -> Sim {
        let n = pos.len();
        let mut s = Sim {
            pos,
            vel,
            mass,
            acc: vec![Vec3::ZERO; n],
            dt,
            theta,
            eps,
        };
        s.compute_acc();
        s
    }

    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// Build the tree and evaluate accelerations for every body in parallel.
    pub fn compute_acc(&mut self) {
        let tree = Octree::build(&self.pos, &self.mass, self.theta, self.eps);
        let pos = &self.pos;
        parallel_fill(&mut self.acc, |i| tree.accel(i, pos[i]));
    }

    /// One leapfrog step. Acceleration from the previous step is reused for the
    /// first half-kick, which is what makes leapfrog symplectic (low energy drift).
    pub fn step(&mut self) {
        let dt = self.dt;
        let half = 0.5 * dt;

        for i in 0..self.pos.len() {
            self.vel[i] += self.acc[i] * half; // half kick
        }
        for i in 0..self.pos.len() {
            self.pos[i] += self.vel[i] * dt; // drift
        }
        self.compute_acc(); // forces at new positions
        for i in 0..self.pos.len() {
            self.vel[i] += self.acc[i] * half; // half kick
        }
    }

    /// Direct O(n^2) acceleration. Reference for verifying the tree.
    pub fn brute_acc(&self) -> Vec<Vec3> {
        let eps2 = self.eps * self.eps;
        let pos = &self.pos;
        let mass = &self.mass;
        let mut out = vec![Vec3::ZERO; pos.len()];
        parallel_fill(&mut out, |i| {
            let pi = pos[i];
            let mut a = Vec3::ZERO;
            for j in 0..pos.len() {
                if i == j {
                    continue;
                }
                let d = pos[j] - pi;
                let r2 = d.length_squared() + eps2;
                let inv = 1.0 / r2.sqrt();
                let inv3 = inv * inv * inv;
                a += d * (G * mass[j] * inv3);
            }
            a
        });
        out
    }

    /// Total mechanical energy (softened potential, matching the force law).
    pub fn total_energy(&self) -> f64 {
        let eps2 = self.eps * self.eps;
        let ke: f64 = (0..self.pos.len())
            .map(|i| 0.5 * self.mass[i] as f64 * self.vel[i].length_squared() as f64)
            .sum();
        let n = self.pos.len();
        let pe = parallel_sum(n, |i| {
            let mut e = 0.0f64;
            for j in (i + 1)..n {
                let d = self.pos[j] - self.pos[i];
                let r = (d.length_squared() + eps2).sqrt();
                e -= (G * self.mass[i] * self.mass[j] / r) as f64;
            }
            e
        });
        ke + pe
    }
}
