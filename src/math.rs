//! Minimal 3D math: `Vec3` and a column-major `Mat4`. No external math crate by design.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[inline]
pub const fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec3 {
    pub const ZERO: Vec3 = vec3(0.0, 0.0, 0.0);
    pub const Y: Vec3 = vec3(0.0, 1.0, 0.0);

    #[inline]
    pub const fn splat(v: f32) -> Vec3 {
        vec3(v, v, v)
    }

    #[inline]
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    #[inline]
    pub fn cross(self, o: Vec3) -> Vec3 {
        vec3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Unit vector in the same direction, or zero for the zero vector.
    #[inline]
    pub fn normalized(self) -> Vec3 {
        let l = self.length();
        if l > 0.0 { self / l } else { Vec3::ZERO }
    }

    #[inline]
    pub fn min(self, o: Vec3) -> Vec3 {
        vec3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    #[inline]
    pub fn max(self, o: Vec3) -> Vec3 {
        vec3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    #[inline]
    pub fn max_element(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    #[inline]
    fn add(self, o: Vec3) -> Vec3 {
        vec3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    #[inline]
    fn sub(self, o: Vec3) -> Vec3 {
        vec3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, s: f32) -> Vec3 {
        vec3(self.x * s, self.y * s, self.z * s)
    }
}

impl Div<f32> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn div(self, s: f32) -> Vec3 {
        self * (1.0 / s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    #[inline]
    fn neg(self) -> Vec3 {
        vec3(-self.x, -self.y, -self.z)
    }
}

impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl SubAssign for Vec3 {
    #[inline]
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}

/// Column-major 4x4 matrix, OpenGL convention (clip-space z in [-1, 1]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [f32; 16]);

impl Mat4 {
    /// Right-handed perspective projection; `fovy` in radians.
    pub fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let f = 1.0 / (fovy * 0.5).tan();
        let mut m = [0.0; 16];
        m[0] = f / aspect;
        m[5] = f;
        m[10] = (far + near) / (near - far);
        m[11] = -1.0;
        m[14] = 2.0 * far * near / (near - far);
        Mat4(m)
    }

    /// Right-handed look-at view matrix.
    pub fn look_at(eye: Vec3, center: Vec3, up: Vec3) -> Mat4 {
        let f = (center - eye).normalized();
        let s = f.cross(up).normalized();
        let u = s.cross(f);
        #[rustfmt::skip]
        let m = [
            s.x,          u.x,          -f.x,        0.0,
            s.y,          u.y,          -f.y,        0.0,
            s.z,          u.z,          -f.z,        0.0,
            -s.dot(eye),  -u.dot(eye),  f.dot(eye),  1.0,
        ];
        Mat4(m)
    }

    pub fn as_array(&self) -> &[f32; 16] {
        &self.0
    }
}

impl Mul for Mat4 {
    type Output = Mat4;

    /// Matrix product `self * rhs` (both column-major).
    fn mul(self, rhs: Mat4) -> Mat4 {
        let (a, b) = (&self.0, &rhs.0);
        Mat4(std::array::from_fn(|i| {
            let (col, row) = (i / 4, i % 4);
            (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum()
        }))
    }
}
