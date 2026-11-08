//! Stars as soft, additive round point sprites, coloured by speed.

use glow::HasContext;

use super::{f32_bytes, link_program};
use crate::math::Mat4;
use crate::sim::worker::Star;

/// Sprite size in pixels at clip w = 1 for an 800 px tall viewport; scaled
/// with the viewport so the look does not depend on window size or DPI.
const POINT_SCALE: f32 = 450.0;
const REFERENCE_HEIGHT: f32 = 800.0;

pub struct StarRenderer {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    u_view_proj: Option<glow::UniformLocation>,
    u_point_scale: Option<glow::UniformLocation>,
    u_speed_scale: Option<glow::UniformLocation>,
    u_brightness: Option<glow::UniformLocation>,
    count: i32,
    pub brightness: f32,
}

impl StarRenderer {
    pub fn new(gl: &glow::Context) -> Result<StarRenderer, String> {
        let program = link_program(
            gl,
            include_str!("shaders/stars.vert"),
            include_str!("shaders/stars.frag"),
        )?;
        // SAFETY: GL calls on objects created here; the context is current.
        unsafe {
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            // Per star: [x, y, z, speed], 16-byte stride.
            let stride = size_of::<Star>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(
                1,
                1,
                glow::FLOAT,
                false,
                stride,
                3 * size_of::<f32>() as i32,
            );
            gl.bind_vertex_array(None);

            Ok(StarRenderer {
                program,
                vao,
                vbo,
                u_view_proj: gl.get_uniform_location(program, "uViewProj"),
                u_point_scale: gl.get_uniform_location(program, "uPointScale"),
                u_speed_scale: gl.get_uniform_location(program, "uSpeedScale"),
                u_brightness: gl.get_uniform_location(program, "uBrightness"),
                count: 0,
                brightness: 0.35,
            })
        }
    }

    /// Replaces the star data. Respecifying the whole buffer lets the driver
    /// hand out fresh storage instead of waiting for the GPU to finish the
    /// previous frame's draw.
    pub fn upload(&mut self, gl: &glow::Context, stars: &[Star]) {
        // SAFETY: the buffer belongs to this renderer; the context is current.
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                f32_bytes(stars.as_flattened()),
                glow::STREAM_DRAW,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        self.count = stars.len() as i32;
    }

    /// Draws into the currently bound framebuffer (cleared first).
    pub fn draw(
        &self,
        gl: &glow::Context,
        view_proj: &Mat4,
        viewport_height: f32,
        reference_speed: f32,
    ) {
        // SAFETY: GL calls on objects owned by this renderer; the context is current.
        unsafe {
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            if self.count == 0 {
                return;
            }

            gl.enable(glow::PROGRAM_POINT_SIZE);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE); // additive

            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u_view_proj.as_ref(), false, view_proj.as_array());
            gl.uniform_1_f32(
                self.u_point_scale.as_ref(),
                POINT_SCALE * viewport_height / REFERENCE_HEIGHT,
            );
            gl.uniform_1_f32(self.u_speed_scale.as_ref(), 1.0 / reference_speed.max(1e-6));
            gl.uniform_1_f32(self.u_brightness.as_ref(), self.brightness);

            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::POINTS, 0, self.count);
            gl.bind_vertex_array(None);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        // SAFETY: deletes objects owned by this renderer; the context is current.
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
        }
    }
}
