//! Orbit camera: circles a target point; drag rotates, wheel dollies.

use crate::math::{Mat4, Vec3};

pub struct OrbitCamera {
    target: Vec3,
    distance: f32,
    yaw: f32,
    pitch: f32,
    aspect: f32,
}

impl OrbitCamera {
    const FOV_Y: f32 = 55.0 * std::f32::consts::PI / 180.0;
    const NEAR: f32 = 0.5;
    const FAR: f32 = 5000.0;
    const MAX_PITCH: f32 = 1.55; // just short of straight up/down
    const DRAG_SPEED: f32 = 0.005; // radians per pixel
    const ZOOM_SPEED: f32 = 0.12; // log-distance per wheel notch

    pub fn new(distance: f32, aspect: f32) -> OrbitCamera {
        OrbitCamera {
            target: Vec3::ZERO,
            distance,
            yaw: 0.7,
            pitch: 0.55,
            aspect,
        }
    }

    pub fn eye(&self) -> Vec3 {
        let cp = self.pitch.cos();
        let dir = Vec3 {
            x: cp * self.yaw.cos(),
            y: self.pitch.sin(),
            z: cp * self.yaw.sin(),
        };
        self.target + dir * self.distance
    }

    pub fn view_proj(&self) -> Mat4 {
        let view = Mat4::look_at(self.eye(), self.target, Vec3::Y);
        let proj = Mat4::perspective(Self::FOV_Y, self.aspect, Self::NEAR, Self::FAR);
        proj * view
    }

    /// Mouse drag in pixels -> rotation.
    pub fn rotate(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * Self::DRAG_SPEED;
        self.pitch = (self.pitch + dy * Self::DRAG_SPEED).clamp(-Self::MAX_PITCH, Self::MAX_PITCH);
    }

    /// Wheel notches -> dolly; positive zooms in.
    pub fn zoom(&mut self, notches: f32) {
        self.distance = (self.distance * (-notches * Self::ZOOM_SPEED).exp()).clamp(2.0, 4000.0);
    }

    pub fn set_distance(&mut self, distance: f32) {
        self.distance = distance;
    }

    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect.max(1e-3);
    }
}
