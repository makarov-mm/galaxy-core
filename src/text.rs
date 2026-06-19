//! Screen-space text. Draws the embedded monospace atlas as textured quads in
//! pixel coordinates, on top of the composited frame. No external font crate.

use crate::font;
use crate::render::link_program;
use glow::HasContext;

const VS: &str = r#"#version 330 core
layout(location = 0) in vec2 aPos;   // pixels, origin top-left
layout(location = 1) in vec2 aUV;
uniform vec2 uViewport;
out vec2 vUV;
void main() {
    vec2 ndc = vec2(aPos.x / uViewport.x * 2.0 - 1.0,
                    1.0 - aPos.y / uViewport.y * 2.0);
    gl_Position = vec4(ndc, 0.0, 1.0);
    vUV = aUV;
}
"#;

const FS: &str = r#"#version 330 core
in vec2 vUV;
out vec4 frag;
uniform sampler2D uAtlas;
uniform vec4 uColor;
void main() {
    float a = texture(uAtlas, vUV).r;
    frag = vec4(uColor.rgb, uColor.a * a);
}
"#;

pub struct TextRenderer {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    atlas: glow::Texture,
    u_viewport: Option<glow::UniformLocation>,
    u_color: Option<glow::UniformLocation>,
    u_atlas: Option<glow::UniformLocation>,
    verts: Vec<f32>,
    cap_floats: usize,
}

impl TextRenderer {
    pub fn new(gl: &glow::Context) -> TextRenderer {
        unsafe {
            let program = link_program(gl, VS, FS);

            let vao = gl.create_vertex_array().expect("text vao");
            gl.bind_vertex_array(Some(vao));
            let vbo = gl.create_buffer().expect("text vbo");
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = 4 * 4;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 2 * 4);
            gl.bind_vertex_array(None);

            let atlas = gl.create_texture().expect("atlas tex");
            gl.bind_texture(glow::TEXTURE_2D, Some(atlas));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                font::ATLAS_W as i32,
                font::ATLAS_H as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                Some(&font::ATLAS),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);

            TextRenderer {
                program,
                vao,
                vbo,
                atlas,
                u_viewport: gl.get_uniform_location(program, "uViewport"),
                u_color: gl.get_uniform_location(program, "uColor"),
                u_atlas: gl.get_uniform_location(program, "uAtlas"),
                verts: Vec::new(),
                cap_floats: 0,
            }
        }
    }

    pub fn line_height(scale: f32) -> f32 {
        font::GLYPH_H as f32 * scale
    }

    /// Draw a (possibly multi-line) string with its top-left at (x, y) pixels.
    pub fn draw(
        &mut self,
        gl: &glow::Context,
        vw: f32,
        vh: f32,
        x: f32,
        y: f32,
        scale: f32,
        color: [f32; 4],
        text: &str,
    ) {
        self.build(x, y, scale, text);
        if self.verts.is_empty() {
            return;
        }
        unsafe {
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.disable(glow::DEPTH_TEST);

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let bytes = bytes_of(&self.verts);
            if self.verts.len() > self.cap_floats {
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
                self.cap_floats = self.verts.len();
            } else {
                gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
            }

            gl.use_program(Some(self.program));
            gl.uniform_2_f32(self.u_viewport.as_ref(), vw, vh);
            gl.uniform_4_f32(self.u_color.as_ref(), color[0], color[1], color[2], color[3]);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            gl.uniform_1_i32(self.u_atlas.as_ref(), 0);

            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, (self.verts.len() / 4) as i32);
            gl.bind_vertex_array(None);
        }
    }

    fn build(&mut self, x: f32, y: f32, scale: f32, text: &str) {
        self.verts.clear();
        let gw = font::GLYPH_W as f32;
        let gh = font::GLYPH_H as f32;
        let aw = font::ATLAS_W as f32;
        let ah = font::ATLAS_H as f32;
        let mut cx = x;
        let mut cy = y;
        for ch in text.chars() {
            if ch == '\n' {
                cx = x;
                cy += gh * scale;
                continue;
            }
            let b = ch as u32;
            if b < font::FIRST_CHAR as u32 || b > font::LAST_CHAR as u32 {
                cx += gw * scale; // unknown char -> blank advance
                continue;
            }
            let idx = (b - font::FIRST_CHAR as u32) as usize;
            let col = (idx % font::COLS) as f32;
            let row = (idx / font::COLS) as f32;
            let u0 = col * gw / aw;
            let v0 = row * gh / ah;
            let u1 = u0 + gw / aw;
            let v1 = v0 + gh / ah;

            let px0 = cx;
            let py0 = cy;
            let px1 = cx + gw * scale;
            let py1 = cy + gh * scale;

            // two triangles, [px, py, u, v]
            self.verts.extend_from_slice(&[
                px0, py0, u0, v0,
                px1, py0, u1, v0,
                px1, py1, u1, v1,
                px0, py0, u0, v0,
                px1, py1, u1, v1,
                px0, py1, u0, v1,
            ]);
            cx += gw * scale;
        }
    }
}

fn bytes_of(s: &[f32]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}
