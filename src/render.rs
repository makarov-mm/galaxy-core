//! GL renderer (glow). Stars are drawn as soft, additive round point sprites.
//! Overlapping bright cores blow out to white, which gives the galaxy glow with
//! no post-process pass. Color is mapped from per-star speed.

use crate::math::Mat4;
use glow::HasContext;

const VS: &str = r#"#version 330 core
layout(location = 0) in vec3 aPos;
layout(location = 1) in float aSpeed;

uniform mat4 uViewProj;
uniform float uPointScale;   // base size in px at unit clip-w
uniform float uSpeedScale;   // 1 / reference speed

out vec3 vColor;

void main() {
    vec4 clip = uViewProj * vec4(aPos, 1.0);
    gl_Position = clip;

    float w = max(clip.w, 0.001);
    gl_PointSize = clamp(uPointScale / w, 1.0, 48.0);

    float t = clamp(aSpeed * uSpeedScale, 0.0, 1.4);
    vec3 cold = vec3(0.30, 0.45, 1.00);
    vec3 warm = vec3(1.00, 0.95, 0.86);
    vec3 hot  = vec3(1.00, 0.78, 0.45);
    vec3 c = mix(cold, warm, smoothstep(0.0, 0.55, t));
    c = mix(c, hot, smoothstep(0.75, 1.25, t));
    vColor = c;
}
"#;

const FS: &str = r#"#version 330 core
in vec3 vColor;
out vec4 frag;

uniform float uBrightness;

void main() {
    vec2 d = gl_PointCoord - vec2(0.5);
    float r2 = dot(d, d);
    if (r2 > 0.25) discard;            // round sprite
    float a = exp(-r2 * 11.0);         // soft gaussian core
    frag = vec4(vColor * uBrightness * a, a);
}
"#;

pub struct Renderer {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    u_view_proj: Option<glow::UniformLocation>,
    u_point_scale: Option<glow::UniformLocation>,
    u_speed_scale: Option<glow::UniformLocation>,
    u_brightness: Option<glow::UniformLocation>,
    capacity_floats: usize,
    pub point_scale: f32,
    pub speed_scale: f32,
    pub brightness: f32,
}

impl Renderer {
    pub fn new(gl: &glow::Context, speed_scale: f32) -> Renderer {
        unsafe {
            let program = link_program(gl, VS, FS);

            let vao = gl.create_vertex_array().expect("vao");
            gl.bind_vertex_array(Some(vao));

            let vbo = gl.create_buffer().expect("vbo");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));

            // Interleaved layout: [x, y, z, speed] per star (16-byte stride).
            let stride = 4 * 4;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 1, glow::FLOAT, false, stride, 3 * 4);

            gl.bind_vertex_array(None);

            let u_view_proj = gl.get_uniform_location(program, "uViewProj");
            let u_point_scale = gl.get_uniform_location(program, "uPointScale");
            let u_speed_scale = gl.get_uniform_location(program, "uSpeedScale");
            let u_brightness = gl.get_uniform_location(program, "uBrightness");

            gl.enable(glow::PROGRAM_POINT_SIZE);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE); // additive
            gl.disable(glow::DEPTH_TEST);

            Renderer {
                program,
                vao,
                vbo,
                u_view_proj,
                u_point_scale,
                u_speed_scale,
                u_brightness,
                capacity_floats: 0,
                point_scale: 900.0,
                speed_scale,
                brightness: 0.65,
            }
        }
    }

    pub fn resize(&self, gl: &glow::Context, w: i32, h: i32) {
        unsafe {
            gl.viewport(0, 0, w, h);
        }
    }

    /// `interleaved` is [x, y, z, speed] repeated; `count` is the number of stars.
    /// Assumes the caller has bound the target framebuffer and set the viewport.
    pub fn draw(&mut self, gl: &glow::Context, view_proj: &Mat4, interleaved: &[f32], count: usize) {
        unsafe {
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            // post-process passes leave blend disabled, so set our state each frame
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE); // additive
            gl.disable(glow::DEPTH_TEST);

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let bytes = bytes_of(interleaved);
            if interleaved.len() > self.capacity_floats {
                // grow (orphan) the buffer
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
                self.capacity_floats = interleaved.len();
            } else {
                gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
            }

            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u_view_proj.as_ref(), false, view_proj.as_slice());
            gl.uniform_1_f32(self.u_point_scale.as_ref(), self.point_scale);
            gl.uniform_1_f32(self.u_speed_scale.as_ref(), self.speed_scale);
            gl.uniform_1_f32(self.u_brightness.as_ref(), self.brightness);

            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::POINTS, 0, count as i32);
            gl.bind_vertex_array(None);
        }
    }
}

pub(crate) unsafe fn link_program(gl: &glow::Context, vs_src: &str, fs_src: &str) -> glow::Program {
    let program = gl.create_program().expect("program");
    let shaders = [(glow::VERTEX_SHADER, vs_src), (glow::FRAGMENT_SHADER, fs_src)];
    let mut handles = Vec::new();
    for (kind, src) in shaders {
        let sh = gl.create_shader(kind).expect("shader");
        gl.shader_source(sh, src);
        gl.compile_shader(sh);
        if !gl.get_shader_compile_status(sh) {
            panic!("shader compile error: {}", gl.get_shader_info_log(sh));
        }
        gl.attach_shader(program, sh);
        handles.push(sh);
    }
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        panic!("program link error: {}", gl.get_program_info_log(program));
    }
    for sh in handles {
        gl.detach_shader(program, sh);
        gl.delete_shader(sh);
    }
    program
}

fn bytes_of(s: &[f32]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}
