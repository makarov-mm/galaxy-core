//! HDR bloom post-process.
//!
//! Stars are rendered into a floating-point scene texture. We extract the bright
//! parts, blur them separably at half resolution (ping-pong), and add them back
//! over the scene with a Reinhard tonemap. Toggle off to composite the raw scene.

use crate::render::link_program;
use glow::HasContext;

const FS_QUAD_VS: &str = r#"#version 330 core
out vec2 vUV;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    vUV = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const BRIGHT_FS: &str = r#"#version 330 core
in vec2 vUV;
out vec4 frag;
uniform sampler2D uScene;
uniform float uThreshold;
void main() {
    vec3 c = texture(uScene, vUV).rgb;
    float l = max(c.r, max(c.g, c.b));
    float k = smoothstep(uThreshold, uThreshold + 0.6, l);
    frag = vec4(c * k, 1.0);
}
"#;

const BLUR_FS: &str = r#"#version 330 core
in vec2 vUV;
out vec4 frag;
uniform sampler2D uTex;
uniform vec2 uDir;          // one texel step along x or y
void main() {
    float w0 = 0.227027;
    float w[4] = float[](0.1945946, 0.1216216, 0.054054, 0.016216);
    vec3 r = texture(uTex, vUV).rgb * w0;
    for (int i = 0; i < 4; i++) {
        vec2 o = uDir * float(i + 1);
        r += texture(uTex, vUV + o).rgb * w[i];
        r += texture(uTex, vUV - o).rgb * w[i];
    }
    frag = vec4(r, 1.0);
}
"#;

const COMPOSITE_FS: &str = r#"#version 330 core
in vec2 vUV;
out vec4 frag;
uniform sampler2D uScene;
uniform sampler2D uBloom;
uniform float uBloomIntensity;
void main() {
    vec3 c = texture(uScene, vUV).rgb + texture(uBloom, vUV).rgb * uBloomIntensity;
    c = c / (c + vec3(1.0));   // Reinhard tonemap; black stays black
    frag = vec4(c, 1.0);
}
"#;

struct Target {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    w: i32,
    h: i32,
}

impl Target {
    unsafe fn new(gl: &glow::Context, w: i32, h: i32) -> Target {
        let tex = gl.create_texture().expect("tex");
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA16F as i32,
            w,
            h,
            0,
            glow::RGBA,
            glow::FLOAT,
            None,
        );
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);

        let fbo = gl.create_framebuffer().expect("fbo");
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
            panic!("framebuffer incomplete ({}x{})", w, h);
        }
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        Target { fbo, tex, w, h }
    }

    unsafe fn destroy(&self, gl: &glow::Context) {
        gl.delete_framebuffer(self.fbo);
        gl.delete_texture(self.tex);
    }
}

pub struct Bloom {
    scene: Target,
    ping: Target, // half resolution
    pong: Target, // half resolution
    dummy_vao: glow::VertexArray,
    bright: glow::Program,
    blur: glow::Program,
    composite: glow::Program,
    w: i32,
    h: i32,
    pub threshold: f32,
    pub intensity: f32,
    pub iterations: u32,
}

impl Bloom {
    pub fn new(gl: &glow::Context, w: i32, h: i32) -> Bloom {
        unsafe {
            let dummy_vao = gl.create_vertex_array().expect("dummy vao");
            let bright = link_program(gl, FS_QUAD_VS, BRIGHT_FS);
            let blur = link_program(gl, FS_QUAD_VS, BLUR_FS);
            let composite = link_program(gl, FS_QUAD_VS, COMPOSITE_FS);

            let (hw, hh) = (w.max(1) / 2, h.max(1) / 2);
            Bloom {
                scene: Target::new(gl, w.max(1), h.max(1)),
                ping: Target::new(gl, hw.max(1), hh.max(1)),
                pong: Target::new(gl, hw.max(1), hh.max(1)),
                dummy_vao,
                bright,
                blur,
                composite,
                w: w.max(1),
                h: h.max(1),
                threshold: 0.8,
                intensity: 0.6,
                iterations: 2,
            }
        }
    }

    pub fn resize(&mut self, gl: &glow::Context, w: i32, h: i32) {
        let (w, h) = (w.max(1), h.max(1));
        if w == self.w && h == self.h {
            return;
        }
        unsafe {
            self.scene.destroy(gl);
            self.ping.destroy(gl);
            self.pong.destroy(gl);
            self.scene = Target::new(gl, w, h);
            self.ping = Target::new(gl, w / 2, h / 2);
            self.pong = Target::new(gl, w / 2, h / 2);
        }
        self.w = w;
        self.h = h;
    }

    /// Bind the HDR scene framebuffer so the star renderer draws into it.
    pub fn begin_scene(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.scene.fbo));
            gl.viewport(0, 0, self.scene.w, self.scene.h);
        }
    }

    /// Run bright-pass + blur + composite to the default framebuffer.
    pub fn composite(&self, gl: &glow::Context, bloom_on: bool) {
        unsafe {
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.bind_vertex_array(Some(self.dummy_vao));

            if bloom_on {
                // bright-pass: scene (full) -> ping (half)
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.ping.fbo));
                gl.viewport(0, 0, self.ping.w, self.ping.h);
                gl.use_program(Some(self.bright));
                self.bind_tex(gl, self.bright, "uScene", 0, self.scene.tex);
                gl.uniform_1_f32(
                    gl.get_uniform_location(self.bright, "uThreshold").as_ref(),
                    self.threshold,
                );
                self.draw_quad(gl);

                // separable blur, ping <-> pong
                gl.use_program(Some(self.blur));
                let texel_x = 1.0 / self.ping.w as f32;
                let texel_y = 1.0 / self.ping.h as f32;
                for _ in 0..self.iterations {
                    // horizontal: ping -> pong
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.pong.fbo));
                    gl.viewport(0, 0, self.pong.w, self.pong.h);
                    self.set_dir(gl, texel_x, 0.0);
                    self.bind_tex(gl, self.blur, "uTex", 0, self.ping.tex);
                    self.draw_quad(gl);
                    // vertical: pong -> ping
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.ping.fbo));
                    gl.viewport(0, 0, self.ping.w, self.ping.h);
                    self.set_dir(gl, 0.0, texel_y);
                    self.bind_tex(gl, self.blur, "uTex", 0, self.pong.tex);
                    self.draw_quad(gl);
                }
            }

            // composite to screen
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, self.w, self.h);
            gl.use_program(Some(self.composite));
            self.bind_tex(gl, self.composite, "uScene", 0, self.scene.tex);
            self.bind_tex(gl, self.composite, "uBloom", 1, self.ping.tex);
            gl.uniform_1_f32(
                gl.get_uniform_location(self.composite, "uBloomIntensity").as_ref(),
                if bloom_on { self.intensity } else { 0.0 },
            );
            self.draw_quad(gl);

            gl.bind_vertex_array(None);
        }
    }

    unsafe fn set_dir(&self, gl: &glow::Context, x: f32, y: f32) {
        gl.uniform_2_f32(gl.get_uniform_location(self.blur, "uDir").as_ref(), x, y);
    }

    unsafe fn bind_tex(
        &self,
        gl: &glow::Context,
        program: glow::Program,
        name: &str,
        unit: u32,
        tex: glow::Texture,
    ) {
        gl.active_texture(glow::TEXTURE0 + unit);
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
    }

    unsafe fn draw_quad(&self, gl: &glow::Context) {
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
    }
}
