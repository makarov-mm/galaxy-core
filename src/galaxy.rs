//! Initial conditions.
//!
//! A rotating exponential-ish disk around a heavy central bulge. Each star gets
//! the circular speed for the mass enclosed at its radius, plus a little velocity
//! dispersion so the disk doesn't collapse into a cold ring — the residual
//! instabilities are what grow into spiral arms.

use crate::math::{vec3, Vec3};
use crate::octree::G;

/// Tiny deterministic RNG (SplitMix64) so runs are reproducible and we pull in
/// no `rand` dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }
    /// Standard normal via Box-Muller (one of the pair).
    pub fn normal(&mut self) -> f32 {
        let u1 = (self.unit() as f64).max(1e-12);
        let u2 = self.unit() as f64;
        ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
    }
}

pub struct DiskParams {
    pub n: usize,
    pub radius: f32,      // outer disk radius
    pub thickness: f32,   // vertical scale
    pub bulge_mass: f32,  // central point mass
    pub disk_mass: f32,   // total mass of all stars combined
    pub dispersion: f32,  // velocity dispersion as fraction of v_circ
    pub seed: u64,
}

impl Default for DiskParams {
    fn default() -> DiskParams {
        DiskParams {
            n: 50_000,
            radius: 30.0,
            thickness: 0.6,
            bulge_mass: 6000.0,
            disk_mass: 3000.0,
            dispersion: 0.30,
            seed: 0xC0FFEE,
        }
    }
}

/// Returns (positions, velocities, masses). Index 0 is the central bulge mass at
/// rest; the rest are disk stars of equal mass.
pub fn make_disk(p: &DiskParams) -> (Vec<Vec3>, Vec<Vec3>, Vec<f32>) {
    make_disk_at(p, Vec3::ZERO, Vec3::ZERO, vec3(0.0, 1.0, 0.0))
}

/// Same disk but translated/boosted and tilted, for composing mergers.
/// `spin_axis` is the disk normal (will be normalized).
pub fn make_disk_at(
    p: &DiskParams,
    origin: Vec3,
    bulk_vel: Vec3,
    spin_axis: Vec3,
) -> (Vec<Vec3>, Vec<Vec3>, Vec<f32>) {
    let mut rng = Rng::new(p.seed);
    let n = p.n;
    let mut pos = Vec::with_capacity(n + 1);
    let mut vel = Vec::with_capacity(n + 1);
    let mut mass = Vec::with_capacity(n + 1);

    let star_mass = p.disk_mass / n as f32;

    // Orthonormal basis (u, w) for the disk plane, with `axis` as normal.
    let axis = spin_axis.normalized();
    let seed_vec = if axis.x.abs() < 0.9 { vec3(1.0, 0.0, 0.0) } else { vec3(0.0, 1.0, 0.0) };
    let u = axis.cross(seed_vec).normalized();
    let w = axis.cross(u);

    // Central bulge.
    pos.push(origin);
    vel.push(bulk_vel);
    mass.push(p.bulge_mass);

    for _ in 0..n {
        // Areal-uniform radius: r = R * sqrt(uniform).
        let r = p.radius * p.unit_r(&mut rng);
        let ang = std::f32::consts::TAU * rng.unit();
        let z = p.thickness * rng.normal();

        let in_plane = u * (r * ang.cos()) + w * (r * ang.sin());
        let position = origin + in_plane + axis * z;

        // Mass enclosed within r: bulge + disk fraction (areal-uniform -> ~ (r/R)^2).
        let frac = (r / p.radius) * (r / p.radius);
        let m_enc = p.bulge_mass + p.disk_mass * frac;
        let v_circ = (G * m_enc / r.max(1e-3)).sqrt();

        // Tangential direction = axis x radial.
        let radial = in_plane.normalized();
        let tangent = axis.cross(radial);

        let mut velocity = bulk_vel + tangent * v_circ;
        // Isotropic dispersion.
        let s = p.dispersion * v_circ;
        velocity += vec3(rng.normal(), rng.normal(), rng.normal()) * s;

        pos.push(position);
        vel.push(velocity);
        mass.push(star_mass);
    }

    (pos, vel, mass)
}

impl DiskParams {
    #[inline]
    fn unit_r(&self, rng: &mut Rng) -> f32 {
        rng.unit().sqrt()
    }
}

/// Two disks set on a parabolic-ish encounter — the showpiece merger.
pub fn make_merger(per_galaxy: usize) -> (Vec<Vec3>, Vec<Vec3>, Vec<f32>) {
    let a = DiskParams { n: per_galaxy, seed: 1, ..Default::default() };
    let b = DiskParams { n: per_galaxy, seed: 2, radius: 22.0, bulge_mass: 3000.0, disk_mass: 4500.0, ..Default::default() };

    let sep = vec3(70.0, 6.0, 0.0);
    // Rough relative speed for a bound, grazing encounter.
    let approach = 6.0;
    let (mut p1, mut v1, mut m1) = make_disk_at(&a, sep * -0.5, vec3(approach * 0.5, 0.0, 0.0), vec3(0.1, 1.0, 0.15));
    let (p2, v2, m2) = make_disk_at(&b, sep * 0.5, vec3(-approach * 0.5, 0.0, 0.0), vec3(-0.2, 1.0, -0.3));

    p1.extend(p2);
    v1.extend(v2);
    m1.extend(m2);
    (p1, v1, m1)
}
