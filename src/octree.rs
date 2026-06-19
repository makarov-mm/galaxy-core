//! Barnes-Hut octree.
//!
//! Arena of nodes in a flat `Vec`. Each node is a cube. A node is either:
//!   - empty            (count == 0)
//!   - a single-body leaf (body >= 0)
//!   - an internal node / merged bucket (body < 0, has children or hit depth cap)
//!
//! Center of mass and total mass are accumulated incrementally during insertion,
//! so there is no separate finalize pass.

use crate::math::Vec3;

/// Gravitational constant in simulation units. We pick G = 1 and scale masses.
pub const G: f32 = 1.0;

/// Hard cap on tree depth. Protects against (near-)coincident bodies forcing
/// unbounded subdivision. Beyond it, bodies are merged into a single bucket node.
const MAX_DEPTH: u32 = 24;

#[derive(Clone)]
struct Node {
    center: Vec3,
    half: f32,
    com: Vec3,
    mass: f32,
    body: i32,         // body index if single-body leaf, else -1
    children: [i32; 8],
    count: u32,
}

impl Node {
    fn new(center: Vec3, half: f32) -> Node {
        Node {
            center,
            half,
            com: Vec3::ZERO,
            mass: 0.0,
            body: -1,
            children: [-1; 8],
            count: 0,
        }
    }
}

pub struct Octree {
    nodes: Vec<Node>,
    theta2: f32,
    eps2: f32,
}

impl Octree {
    /// Build a tree over the given positions/masses.
    /// `theta` is the Barnes-Hut opening angle (smaller = more accurate, slower).
    /// `eps` is the force-softening length.
    pub fn build(pos: &[Vec3], mass: &[f32], theta: f32, eps: f32) -> Octree {
        // Bounding cube.
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for p in pos {
            lo.x = lo.x.min(p.x);
            lo.y = lo.y.min(p.y);
            lo.z = lo.z.min(p.z);
            hi.x = hi.x.max(p.x);
            hi.y = hi.y.max(p.y);
            hi.z = hi.z.max(p.z);
        }
        let center = (lo + hi) * 0.5;
        let span = hi - lo;
        // Pad slightly so boundary bodies sit strictly inside.
        let half = (span.x.max(span.y).max(span.z) * 0.5) * 1.0001 + 1e-6;

        let mut t = Octree {
            nodes: Vec::with_capacity(pos.len() * 2),
            theta2: theta * theta,
            eps2: eps * eps,
        };
        t.nodes.push(Node::new(center, half.max(1e-6)));
        for i in 0..pos.len() {
            t.insert(0, i as i32, pos[i], mass[i], 0);
        }
        t
    }

    fn octant(center: Vec3, p: Vec3) -> usize {
        let mut o = 0;
        if p.x >= center.x { o |= 1; }
        if p.y >= center.y { o |= 2; }
        if p.z >= center.z { o |= 4; }
        o
    }

    fn child_or_create(&mut self, node: usize, oct: usize) -> usize {
        let existing = self.nodes[node].children[oct];
        if existing >= 0 {
            return existing as usize;
        }
        let center = self.nodes[node].center;
        let half = self.nodes[node].half;
        let q = half * 0.5;
        let cc = Vec3 {
            x: center.x + if oct & 1 != 0 { q } else { -q },
            y: center.y + if oct & 2 != 0 { q } else { -q },
            z: center.z + if oct & 4 != 0 { q } else { -q },
        };
        let idx = self.nodes.len();
        self.nodes.push(Node::new(cc, q));
        self.nodes[node].children[oct] = idx as i32;
        idx
    }

    fn insert(&mut self, node: usize, body_idx: i32, p: Vec3, m: f32, depth: u32) {
        // Empty node: drop the body straight in as a single-body leaf.
        if self.nodes[node].count == 0 {
            let n = &mut self.nodes[node];
            n.body = body_idx;
            n.com = p;
            n.mass = m;
            n.count = 1;
            return;
        }

        // Depth cap: merge as a point-mass bucket, do not subdivide further.
        if depth >= MAX_DEPTH {
            let n = &mut self.nodes[node];
            let total = n.mass + m;
            n.com = (n.com * n.mass + p * m) / total;
            n.mass = total;
            n.count += 1;
            n.body = -1;
            return;
        }

        // Occupied single-body leaf: push the resident body down a level first.
        if self.nodes[node].body >= 0 {
            let ob = self.nodes[node].body;
            let ocom = self.nodes[node].com;
            let omass = self.nodes[node].mass;
            self.nodes[node].body = -1;
            let center = self.nodes[node].center;
            let oct = Octree::octant(center, ocom);
            let child = self.child_or_create(node, oct);
            self.insert(child, ob, ocom, omass, depth + 1);
        }

        // Internal node: fold the new body into this node's COM/mass, then descend.
        {
            let n = &mut self.nodes[node];
            let total = n.mass + m;
            n.com = (n.com * n.mass + p * m) / total;
            n.mass = total;
            n.count += 1;
        }
        let center = self.nodes[node].center;
        let oct = Octree::octant(center, p);
        let child = self.child_or_create(node, oct);
        self.insert(child, body_idx, p, m, depth + 1);
    }

    #[inline]
    fn pair_accel(&self, p: Vec3, com: Vec3, mass: f32) -> Vec3 {
        let d = com - p;
        let r2 = d.length_squared() + self.eps2;
        let inv = 1.0 / r2.sqrt();
        let inv3 = inv * inv * inv;
        d * (G * mass * inv3)
    }

    /// Acceleration on body `idx` located at `p`.
    pub fn accel(&self, idx: usize, p: Vec3) -> Vec3 {
        self.accel_node(0, idx, p)
    }

    fn accel_node(&self, node: usize, idx: usize, p: Vec3) -> Vec3 {
        let n = &self.nodes[node];
        if n.count == 0 {
            return Vec3::ZERO;
        }
        if n.body >= 0 {
            if n.body as usize == idx {
                return Vec3::ZERO; // skip self
            }
            return self.pair_accel(p, n.com, n.mass);
        }
        // Opening criterion: (size / distance) < theta  ->  treat as one point mass.
        let d = n.com - p;
        let dist2 = d.length_squared() + self.eps2;
        let size = n.half * 2.0;
        if size * size < self.theta2 * dist2 {
            return self.pair_accel(p, n.com, n.mass);
        }
        let mut a = Vec3::ZERO;
        for c in n.children {
            if c >= 0 {
                a += self.accel_node(c as usize, idx, p);
            }
        }
        a
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}
