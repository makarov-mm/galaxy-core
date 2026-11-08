//! Initial conditions.
//!
//! A rotating disk around a heavy central bulge. Each star gets the circular
//! speed for the mass enclosed at its radius, plus a little velocity dispersion
//! so the disk doesn't settle into a cold ring -- the residual instabilities are
//! what grow into spiral arms.

use std::f32::consts::TAU;

use super::Bodies;
use super::octree::G;
use crate::math::{Vec3, vec3};

/// Tiny deterministic RNG (SplitMix64): reproducible runs, no `rand` dependency.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// Standard normal (Box-Muller, one value of the pair).
    fn normal(&mut self) -> f32 {
        let u1 = f64::from(self.unit()).max(1e-12);
        let u2 = f64::from(self.unit());
        ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
    }
}

#[derive(Clone, Debug)]
pub struct DiskParams {
    pub stars: usize,
    pub radius: f32,     // outer disk radius
    pub thickness: f32,  // vertical scale
    pub bulge_mass: f32, // central point mass
    pub disk_mass: f32,  // total mass of all stars combined
    pub dispersion: f32, // velocity dispersion as a fraction of the circular speed
    pub softening: f32,  // must match the simulation's softening length
    pub seed: u64,
}

impl Default for DiskParams {
    fn default() -> DiskParams {
        DiskParams {
            stars: 50_000,
            radius: 30.0,
            thickness: 0.6,
            bulge_mass: 6000.0,
            disk_mass: 3000.0,
            dispersion: 0.30,
            softening: 0.5,
            seed: 0x00C0_FFEE,
        }
    }
}

/// A disk centred at `origin`, moving with `bulk_vel`, spinning about
/// `spin_axis` (the disk normal). The first body is the bulge, at rest relative
/// to the disk; the rest are equal-mass stars.
pub fn disk(p: &DiskParams, origin: Vec3, bulk_vel: Vec3, spin_axis: Vec3) -> Bodies {
    let mut rng = Rng(p.seed);
    let mut bodies = Bodies::with_capacity(p.stars + 1);

    // Orthonormal basis (u, w) of the disk plane.
    let axis = spin_axis.normalized();
    let helper = if axis.x.abs() < 0.9 {
        vec3(1.0, 0.0, 0.0)
    } else {
        Vec3::Y
    };
    let u = axis.cross(helper).normalized();
    let w = axis.cross(u);

    bodies.push(origin, bulk_vel, p.bulge_mass);

    let star_mass = p.disk_mass / p.stars as f32;
    let eps2 = p.softening * p.softening;
    for _ in 0..p.stars {
        // Areal-uniform radius: r = R * sqrt(uniform).
        let r = p.radius * rng.unit().sqrt();
        let angle = TAU * rng.unit();
        let height = p.thickness * rng.normal();
        let in_plane = u * (r * angle.cos()) + w * (r * angle.sin());

        // Mass enclosed within r: bulge + the disk's areal fraction (r/R)^2.
        let enclosed = p.bulge_mass + p.disk_mass * (r / p.radius).powi(2);
        // Circular speed under the *softened* force the simulation uses:
        // v^2 / r = G M r / (r^2 + eps^2)^(3/2). With the bare Newtonian v^2 = GM/r,
        // stars inside the softening length start far faster than circular
        // and are flung out of the core.
        let v_circ = (G * enclosed * r * r / (r * r + eps2).powf(1.5)).sqrt();

        let tangent = axis.cross(in_plane.normalized());
        let jitter = vec3(rng.normal(), rng.normal(), rng.normal()) * (p.dispersion * v_circ);
        bodies.push(
            origin + in_plane + axis * height,
            bulk_vel + tangent * v_circ + jitter,
            star_mass,
        );
    }
    bodies
}

/// The built-in scenes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// One spiral disk.
    Disk,
    /// Two disks on a grazing, bound encounter.
    Merger,
}

impl Scene {
    pub fn toggled(self) -> Scene {
        match self {
            Scene::Disk => Scene::Merger,
            Scene::Merger => Scene::Disk,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Scene::Disk => "single disk",
            Scene::Merger => "merger",
        }
    }

    pub fn bodies(self, softening: f32) -> Bodies {
        match self {
            Scene::Disk => {
                let p = DiskParams {
                    stars: 40_000,
                    softening,
                    ..Default::default()
                };
                disk(&p, Vec3::ZERO, Vec3::ZERO, Vec3::Y)
            }
            Scene::Merger => {
                let a = DiskParams {
                    stars: 20_000,
                    softening,
                    seed: 1,
                    ..Default::default()
                };
                let b = DiskParams {
                    stars: 20_000,
                    softening,
                    seed: 2,
                    radius: 22.0,
                    bulge_mass: 3000.0,
                    disk_mass: 4500.0,
                    ..Default::default()
                };
                let separation = vec3(70.0, 6.0, 0.0);
                let approach = 6.0; // relative speed for a bound, grazing encounter
                let mut bodies = disk(
                    &a,
                    separation * -0.5,
                    vec3(approach * 0.5, 0.0, 0.0),
                    vec3(0.1, 1.0, 0.15),
                );
                bodies.append(disk(
                    &b,
                    separation * 0.5,
                    vec3(-approach * 0.5, 0.0, 0.0),
                    vec3(-0.2, 1.0, -0.3),
                ));
                bodies
            }
        }
    }
}
