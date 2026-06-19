//! Orbital camera. Pure math, no GL — orbits a target point, mouse drag rotates,
//! wheel dollies in/out.

use crate::math::{vec3, Mat4, Vec3};

pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl OrbitCamera {
    pub fn new(distance: f32, aspect: f32) -> OrbitCamera {
        OrbitCamera {
            target: Vec3::ZERO,
            distance,
            yaw: 0.7,
            pitch: 0.55,
            fov: 55f32.to_radians(),
            aspect,
            near: 0.5,
            far: 5000.0,
        }
    }

    pub fn eye(&self) -> Vec3 {
        let cp = self.pitch.cos();
        let dir = vec3(cp * self.yaw.cos(), self.pitch.sin(), cp * self.yaw.sin());
        self.target + dir * self.distance
    }

    pub fn view_proj(&self) -> Mat4 {
        let view = Mat4::look_at(self.eye(), self.target, vec3(0.0, 1.0, 0.0));
        let proj = Mat4::perspective(self.fov, self.aspect.max(1e-3), self.near, self.far);
        proj.mul(&view)
    }

    /// Mouse drag in pixels -> rotation.
    pub fn rotate(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.005;
        self.pitch += dy * 0.005;
        let lim = 1.55;
        self.pitch = self.pitch.clamp(-lim, lim);
    }

    /// Wheel notches -> dolly. Positive zooms in.
    pub fn zoom(&mut self, amount: f32) {
        self.distance *= (-amount * 0.12).exp();
        self.distance = self.distance.clamp(2.0, 4000.0);
    }

    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
    }
}
