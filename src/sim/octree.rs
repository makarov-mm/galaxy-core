//! Barnes-Hut octree over Morton-ordered bodies.
//!
//! Bodies are sorted along a Morton (Z-order) curve before every build, so each
//! tree node covers a *contiguous* range of bodies. That gives:
//!   - leaves that hold small buckets of bodies (summed exactly when opened);
//!   - nodes laid out in depth-first pre-order with a "skip" index, so the
//!     force walk is a flat loop instead of recursion;
//!   - bodies that are close in space are close in memory (cache-friendly).
//!
//! Forces are evaluated per leaf ("group walk", Barnes 1990): one tree walk
//! for all bodies of a leaf collects an interaction list of point masses --
//! accepted cells plus the bodies of nearby leaves -- and every body of the
//! leaf then sums that list in a tight, vectorizable loop.
//!
//! Opening criterion (Barnes 1994): a cell acts as a point mass when the whole
//! group is farther than `size / theta + |com - cell centre|` from its centre
//! of mass. The offset term guarantees a body is never approximated by a cell
//! that contains it, which removes the classic "exploding cell" error.

use std::ops::Range;

use crate::math::{Vec3, vec3};

/// Gravitational constant in simulation units: G = 1, masses are scaled instead.
pub const G: f32 = 1.0;

/// Bodies per leaf. A leaf is also a force-evaluation group (one tree walk
/// shared by its bodies); 16 measured fastest at equal accuracy.
const LEAF_CAPACITY: usize = 16;

/// Morton resolution per axis (3 x 21 = 63-bit codes). It also caps tree depth:
/// bodies that still share a cell at the finest level form one leaf bucket.
const MORTON_BITS: u32 = 21;

/// Axis-aligned bounding cube of the bodies; defines the root cell.
#[derive(Clone, Copy, Debug)]
pub struct Cube {
    min: Vec3,
    size: f32,
}

impl Cube {
    /// Smallest cube containing all points (slightly padded so every point is
    /// strictly inside).
    pub fn bounding(points: &[Vec3]) -> Cube {
        if points.is_empty() {
            return Cube {
                min: Vec3::ZERO,
                size: 1.0,
            };
        }
        let (lo, hi) = points.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        let center = (lo + hi) * 0.5;
        let half = ((hi - lo).max_element() * 0.5 * 1.0001).max(1e-6);
        Cube {
            min: center - Vec3::splat(half),
            size: 2.0 * half,
        }
    }

    /// 63-bit Morton code of `p`: x, y, z bits interleaved, x most significant.
    pub fn morton_code(&self, p: Vec3) -> u64 {
        const CELLS: f32 = (1u32 << MORTON_BITS) as f32;
        const MAX: u32 = (1 << MORTON_BITS) - 1;
        let scale = CELLS / self.size;
        // `as u32` saturates (NaN -> 0), so out-of-range input cannot panic.
        let q = |v: f32| ((v * scale) as u32).min(MAX);
        let d = p - self.min;
        spread_bits(q(d.x)) << 2 | spread_bits(q(d.y)) << 1 | spread_bits(q(d.z))
    }
}

/// Inserts two zero bits between each of the low 21 bits of `v`.
fn spread_bits(v: u32) -> u64 {
    let mut x = u64::from(v) & 0x1f_ffff;
    x = (x | x << 32) & 0x001f_0000_0000_ffff;
    x = (x | x << 16) & 0x001f_0000_ff00_00ff;
    x = (x | x << 8) & 0x100f_00f0_0f00_f00f;
    x = (x | x << 4) & 0x10c3_0c30_c30c_30c3;
    x = (x | x << 2) & 0x1249_2492_4924_9249;
    x
}

/// One cell. Children of an internal node follow it directly (pre-order), so
/// "descend" is `index + 1` and "skip this subtree" is `next`.
#[derive(Clone, Copy, Debug, Default)]
struct Node {
    com: Vec3,  // centre of mass
    mass: f32,  // total mass
    open2: f32, // squared opening radius: farther bodies see a point mass
    next: u32,  // first node after this subtree
    first: u32, // first body (leaves only)
    count: u32, // bodies in a leaf; 0 for internal nodes
}

pub struct Octree {
    nodes: Vec<Node>,
    leaves: Vec<Range<usize>>, // body ranges of the leaves, in body order
    inv_theta: f32,
    eps2: f32,
}

impl Octree {
    /// `theta` is the opening angle (smaller = more accurate, slower);
    /// `eps` is the Plummer softening length.
    pub fn new(theta: f32, eps: f32) -> Octree {
        // With softening a body's pull on itself is exactly zero (zero
        // separation), which lets the interaction loop skip the self check.
        assert!(eps > 0.0, "the softening length must be positive");
        Octree {
            nodes: Vec::new(),
            leaves: Vec::new(),
            inv_theta: 1.0 / theta,
            eps2: eps * eps,
        }
    }

    /// Body ranges of the leaves. They partition all bodies, in order.
    pub fn leaves(&self) -> &[Range<usize>] {
        &self.leaves
    }

    /// Rebuilds the tree, reusing its allocation. Bodies must already be sorted
    /// by `codes` (Morton codes computed with `cube`).
    pub fn build(&mut self, codes: &[u64], pos: &[Vec3], mass: &[f32], cube: Cube) {
        debug_assert!(codes.len() == pos.len() && pos.len() == mass.len());
        debug_assert!(codes.is_sorted());
        self.nodes.clear();
        self.leaves.clear();
        if !codes.is_empty() {
            let half = cube.size * 0.5;
            let bodies = SortedBodies { codes, pos, mass };
            self.build_node(
                &bodies,
                0..codes.len(),
                0,
                cube.min + Vec3::splat(half),
                half,
            );
        }
    }

    /// Builds the subtree for `range` (bodies sharing all Morton bits above
    /// `level`) and returns its (centre of mass, mass).
    fn build_node(
        &mut self,
        b: &SortedBodies,
        range: Range<usize>,
        level: u32,
        center: Vec3,
        half: f32,
    ) -> (Vec3, f32) {
        let index = self.nodes.len();
        self.nodes.push(Node::default());

        let (com, mass, first, count) = if range.len() <= LEAF_CAPACITY || level == MORTON_BITS {
            let (weighted, mass) = b.pos[range.clone()]
                .iter()
                .zip(&b.mass[range.clone()])
                .fold((Vec3::ZERO, 0.0), |(w, m), (&p, &mi)| (w + p * mi, m + mi));
            self.leaves.push(range.clone());
            (
                weighted / mass,
                mass,
                range.start as u32,
                range.len() as u32,
            )
        } else {
            // Children are the runs of equal 3-bit octant digits at this level.
            let shift = 3 * (MORTON_BITS - 1 - level);
            let quarter = half * 0.5;
            let (mut weighted, mut mass) = (Vec3::ZERO, 0.0);
            let mut start = range.start;
            while start < range.end {
                let octant = (b.codes[start] >> shift) & 7;
                let end = start
                    + b.codes[start..range.end].partition_point(|&c| (c >> shift) & 7 == octant);
                let offset = |bit: u64| if octant & bit != 0 { quarter } else { -quarter };
                let child_center = center + vec3(offset(4), offset(2), offset(1));
                let (c, m) = self.build_node(b, start..end, level + 1, child_center, quarter);
                weighted += c * m;
                mass += m;
                start = end;
            }
            (weighted / mass, mass, 0, 0)
        };

        let open2 = if count == 1 {
            0.0 // a single body is exact as a point mass at any distance
        } else {
            let radius = 2.0 * half * self.inv_theta + (com - center).length();
            radius * radius
        };
        let next = self.nodes.len() as u32;
        self.nodes[index] = Node {
            com,
            mass,
            open2,
            next,
            first,
            count,
        };
        (com, mass)
    }

    /// Accelerations of the bodies in `leaf` (one of `leaves()`), written to
    /// `out` (one entry per body of the leaf). `list` is scratch space.
    pub fn leaf_accelerations(
        &self,
        leaf: Range<usize>,
        pos: &[Vec3],
        mass: &[f32],
        list: &mut Interactions,
        out: &mut [Vec3],
    ) {
        let group = &pos[leaf];
        let (lo, hi) = group.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        let center = (lo + hi) * 0.5;
        let radius = group
            .iter()
            .map(|&p| (p - center).length())
            .fold(0.0, f32::max);

        // One walk for the whole group: a cell is accepted only if it passes
        // the opening test for every point of the group's bounding sphere.
        list.clear();
        let mut n = 0;
        while let Some(node) = self.nodes.get(n) {
            let gap = (node.com - center).length() - radius;
            if gap > 0.0 && gap * gap > node.open2 {
                list.push(node.com, node.mass);
                n = node.next as usize;
            } else if node.count > 0 {
                // Opened leaf (including the group's own): its bodies, exactly.
                let bodies = node.first as usize..(node.first + node.count) as usize;
                for (&q, &m) in pos[bodies.clone()].iter().zip(&mass[bodies]) {
                    list.push(q, m);
                }
                n = node.next as usize;
            } else {
                n += 1; // open: descend into the first child
            }
        }

        list.pad_to_lanes(center);
        for (a, &p) in out.iter_mut().zip(group) {
            *a = list.accel_at(p, self.eps2) * G;
        }
    }
}

/// Point masses acting on one group, as separate coordinate arrays so the
/// summation loop vectorizes.
#[derive(Default)]
pub struct Interactions {
    x: Vec<f32>,
    y: Vec<f32>,
    z: Vec<f32>,
    m: Vec<f32>,
}

impl Interactions {
    /// Summation width: independent partial sums per lane let the compiler use
    /// SIMD without reassociating floating-point adds.
    const LANES: usize = 8;

    fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
        self.z.clear();
        self.m.clear();
    }

    #[inline]
    fn push(&mut self, p: Vec3, m: f32) {
        self.x.push(p.x);
        self.y.push(p.y);
        self.z.push(p.z);
        self.m.push(m);
    }

    /// Appends massless entries up to a whole number of lanes.
    fn pad_to_lanes(&mut self, at: Vec3) {
        while self.m.len() % Self::LANES != 0 {
            self.push(at, 0.0);
        }
    }

    /// Softened acceleration at `p` from every point mass (G = 1). A body in
    /// the list pulls on itself with zero force (zero separation), so self
    /// interaction needs no special case.
    fn accel_at(&self, p: Vec3, eps2: f32) -> Vec3 {
        let mut ax = [0.0f32; Self::LANES];
        let mut ay = [0.0f32; Self::LANES];
        let mut az = [0.0f32; Self::LANES];
        let chunks = self
            .x
            .chunks_exact(Self::LANES)
            .zip(self.y.chunks_exact(Self::LANES))
            .zip(
                self.z
                    .chunks_exact(Self::LANES)
                    .zip(self.m.chunks_exact(Self::LANES)),
            );
        for ((x, y), (z, m)) in chunks {
            for l in 0..Self::LANES {
                let (dx, dy, dz) = (x[l] - p.x, y[l] - p.y, z[l] - p.z);
                let inv = 1.0 / (dx * dx + dy * dy + dz * dz + eps2).sqrt();
                let s = m[l] * inv * inv * inv;
                ax[l] += dx * s;
                ay[l] += dy * s;
                az[l] += dz * s;
            }
        }
        Vec3 {
            x: ax.iter().sum(),
            y: ay.iter().sum(),
            z: az.iter().sum(),
        }
    }
}

/// The Morton-sorted body arrays a build reads from.
struct SortedBodies<'a> {
    codes: &'a [u64],
    pos: &'a [Vec3],
    mass: &'a [f32],
}
