//! Screen-space text: glyphs of the embedded monospace atlas drawn as textured
//! quads in pixel coordinates over the finished frame. No font crate.

use glow::HasContext;

use super::{f32_bytes, font, link_program};

/// Pixel origin (top-left) and colour of a text block.
#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub color: [f32; 4],
}

pub struct TextRenderer {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    atlas: glow::Texture,
    u_viewport: Option<glow::UniformLocation>,
    u_color: Option<glow::UniformLocation>,
    vertices: Vec<f32>,
}

impl TextRenderer {
    pub fn new(gl: &glow::Context) -> Result<TextRenderer, String> {
        let program = link_program(
            gl,
            include_str!("shaders/text.vert"),
            include_str!("shaders/text.frag"),
        )?;
        // SAFETY: GL calls on objects created here; the context is current.
        unsafe {
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            // Per vertex: [x, y, u, v].
            let stride = 4 * size_of::<f32>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(
                1,
                2,
                glow::FLOAT,
                false,
                stride,
                2 * size_of::<f32>() as i32,
            );
            gl.bind_vertex_array(None);

            let atlas = gl.create_texture()?;
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
                glow::PixelUnpackData::Slice(Some(font::ATLAS.as_slice())),
            );
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }

            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "uAtlas").as_ref(), 0);
            gl.use_program(None);

            Ok(TextRenderer {
                program,
                vao,
                vbo,
                atlas,
                u_viewport: gl.get_uniform_location(program, "uViewport"),
                u_color: gl.get_uniform_location(program, "uColor"),
                vertices: Vec::new(),
            })
        }
    }

    pub fn line_height(scale: f32) -> f32 {
        font::GLYPH_H as f32 * scale
    }

    /// Draws a (possibly multi-line) string over the default framebuffer of
    /// size `viewport` (pixels).
    pub fn draw(&mut self, gl: &glow::Context, viewport: (f32, f32), style: TextStyle, text: &str) {
        self.layout(style, text);
        if self.vertices.is_empty() {
            return;
        }
        // SAFETY: GL calls on objects owned by this renderer; the context is current.
        unsafe {
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                f32_bytes(&self.vertices),
                glow::STREAM_DRAW,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            gl.use_program(Some(self.program));
            gl.uniform_2_f32(self.u_viewport.as_ref(), viewport.0, viewport.1);
            let [r, g, b, a] = style.color;
            gl.uniform_4_f32(self.u_color.as_ref(), r, g, b, a);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));

            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, (self.vertices.len() / 4) as i32);
            gl.bind_vertex_array(None);
        }
    }

    /// Two triangles per visible glyph, [x, y, u, v] per vertex.
    fn layout(&mut self, style: TextStyle, text: &str) {
        let (glyph_w, glyph_h) = (
            font::GLYPH_W as f32 * style.scale,
            font::GLYPH_H as f32 * style.scale,
        );
        let (du, dv) = (
            font::GLYPH_W as f32 / font::ATLAS_W as f32,
            font::GLYPH_H as f32 / font::ATLAS_H as f32,
        );

        self.vertices.clear();
        let (mut x, mut y) = (style.x, style.y);
        for ch in text.chars() {
            if ch == '\n' {
                (x, y) = (style.x, y + glyph_h);
                continue;
            }
            if ch != ' ' && (font::FIRST_CHAR..=font::LAST_CHAR).contains(&ch) {
                let index = ch as usize - font::FIRST_CHAR as usize;
                let u0 = (index % font::COLUMNS) as f32 * du;
                let v0 = (index / font::COLUMNS) as f32 * dv;
                let (u1, v1) = (u0 + du, v0 + dv);
                let (x1, y1) = (x + glyph_w, y + glyph_h);
                #[rustfmt::skip]
                self.vertices.extend_from_slice(&[
                    x,  y,  u0, v0,
                    x1, y,  u1, v0,
                    x1, y1, u1, v1,
                    x,  y,  u0, v0,
                    x1, y1, u1, v1,
                    x,  y1, u0, v1,
                ]);
            }
            x += glyph_w; // unknown characters advance like a space
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        // SAFETY: deletes objects owned by this renderer; the context is current.
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
            gl.delete_texture(self.atlas);
        }
    }
}
