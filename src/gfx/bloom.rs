//! HDR bloom post-process.
//!
//! Stars render into a floating-point scene texture. The bright parts are
//! extracted, blurred separably at half resolution (ping-pong) and added back
//! over the scene with a Reinhard tone map. With bloom off the scene is just
//! tone mapped.

use glow::HasContext;

use super::link_program;

const FULLSCREEN_VS: &str = include_str!("shaders/fullscreen.vert");

/// An RGBA16F texture with a framebuffer around it.
struct Target {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    width: i32,
    height: i32,
}

impl Target {
    fn new(gl: &glow::Context, width: i32, height: i32) -> Result<Target, String> {
        let (width, height) = (width.max(1), height.max(1));
        // SAFETY: GL calls on objects created here; the context is current.
        unsafe {
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA16F as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelUnpackData::Slice(None),
            );
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }

            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);

            let target = Target {
                fbo,
                tex,
                width,
                height,
            };
            if status != glow::FRAMEBUFFER_COMPLETE {
                target.destroy(gl);
                return Err(format!(
                    "framebuffer incomplete ({width}x{height}, status {status:#x})"
                ));
            }
            Ok(target)
        }
    }

    fn bind(&self, gl: &glow::Context) {
        // SAFETY: binds objects owned by this target; the context is current.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, self.width, self.height);
        }
    }

    fn destroy(&self, gl: &glow::Context) {
        // SAFETY: deletes objects owned by this target; the context is current.
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.tex);
        }
    }
}

/// A fullscreen shader pass with the uniform locations it needs.
struct Pass {
    program: glow::Program,
    param: Option<glow::UniformLocation>,
}

impl Pass {
    /// Links the pass, binds its samplers to texture units 0, 1, ... in
    /// order, and looks up its one parameter uniform.
    fn new(
        gl: &glow::Context,
        fragment: &str,
        samplers: &[&str],
        param: &str,
    ) -> Result<Pass, String> {
        let program = link_program(gl, FULLSCREEN_VS, fragment)?;
        // SAFETY: GL calls on the program created here; the context is current.
        unsafe {
            gl.use_program(Some(program));
            for (unit, name) in samplers.iter().enumerate() {
                gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
            }
            gl.use_program(None);
            Ok(Pass {
                program,
                param: gl.get_uniform_location(program, param),
            })
        }
    }
}

pub struct Bloom {
    scene: Target,
    ping: Target, // half resolution
    pong: Target, // half resolution
    empty_vao: glow::VertexArray,
    bright: Pass,
    blur: Pass,
    composite: Pass,
    pub threshold: f32,
    pub intensity: f32,
    pub iterations: u32,
}

impl Bloom {
    pub fn new(gl: &glow::Context, width: i32, height: i32) -> Result<Bloom, String> {
        let bright = Pass::new(
            gl,
            include_str!("shaders/bloom_bright.frag"),
            &["uScene"],
            "uThreshold",
        )?;
        let blur = Pass::new(
            gl,
            include_str!("shaders/bloom_blur.frag"),
            &["uTex"],
            "uDir",
        )?;
        let composite = Pass::new(
            gl,
            include_str!("shaders/bloom_composite.frag"),
            &["uScene", "uBloom"],
            "uBloomIntensity",
        )?;
        let (scene, ping, pong) = Self::targets(gl, width, height)?;
        // SAFETY: plain object creation; the context is current.
        let empty_vao = unsafe { gl.create_vertex_array()? };
        Ok(Bloom {
            scene,
            ping,
            pong,
            empty_vao,
            bright,
            blur,
            composite,
            threshold: 0.8,
            intensity: 0.6,
            iterations: 2,
        })
    }

    fn targets(
        gl: &glow::Context,
        width: i32,
        height: i32,
    ) -> Result<(Target, Target, Target), String> {
        let (half_w, half_h) = ((width / 2).max(1), (height / 2).max(1));
        Ok((
            Target::new(gl, width, height)?,
            Target::new(gl, half_w, half_h)?,
            Target::new(gl, half_w, half_h)?,
        ))
    }

    pub fn resize(&mut self, gl: &glow::Context, width: i32, height: i32) -> Result<(), String> {
        if (width.max(1), height.max(1)) == (self.scene.width, self.scene.height) {
            return Ok(());
        }
        let (scene, ping, pong) = Self::targets(gl, width, height)?;
        for old in [&self.scene, &self.ping, &self.pong] {
            old.destroy(gl);
        }
        (self.scene, self.ping, self.pong) = (scene, ping, pong);
        Ok(())
    }

    /// Binds the HDR scene framebuffer for the star renderer.
    pub fn begin_scene(&self, gl: &glow::Context) {
        self.scene.bind(gl);
    }

    /// Bright pass + blur (if enabled), then composite to the default framebuffer.
    pub fn composite(&self, gl: &glow::Context, enabled: bool) {
        // SAFETY: GL calls on objects owned by this struct; the context is current.
        unsafe {
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.empty_vao));

            if enabled {
                // Bright pass: scene (full res) -> ping (half res).
                self.ping.bind(gl);
                gl.use_program(Some(self.bright.program));
                gl.uniform_1_f32(self.bright.param.as_ref(), self.threshold);
                bind_texture(gl, 0, self.scene.tex);
                draw_fullscreen(gl);

                // Separable blur: ping -> pong (horizontal) -> ping (vertical).
                gl.use_program(Some(self.blur.program));
                let texel = (1.0 / self.ping.width as f32, 1.0 / self.ping.height as f32);
                for _ in 0..self.iterations {
                    for (src, dst, dir) in [
                        (&self.ping, &self.pong, (texel.0, 0.0)),
                        (&self.pong, &self.ping, (0.0, texel.1)),
                    ] {
                        dst.bind(gl);
                        gl.uniform_2_f32(self.blur.param.as_ref(), dir.0, dir.1);
                        bind_texture(gl, 0, src.tex);
                        draw_fullscreen(gl);
                    }
                }
            }

            // Composite to the screen.
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, self.scene.width, self.scene.height);
            gl.use_program(Some(self.composite.program));
            gl.uniform_1_f32(
                self.composite.param.as_ref(),
                if enabled { self.intensity } else { 0.0 },
            );
            bind_texture(gl, 0, self.scene.tex);
            bind_texture(gl, 1, self.ping.tex);
            draw_fullscreen(gl);

            gl.bind_vertex_array(None);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        for target in [&self.scene, &self.ping, &self.pong] {
            target.destroy(gl);
        }
        // SAFETY: deletes objects owned by this struct; the context is current.
        unsafe {
            for pass in [&self.bright, &self.blur, &self.composite] {
                gl.delete_program(pass.program);
            }
            gl.delete_vertex_array(self.empty_vao);
        }
    }
}

fn bind_texture(gl: &glow::Context, unit: u32, tex: glow::Texture) {
    // SAFETY: binds an existing texture; the context is current.
    unsafe {
        gl.active_texture(glow::TEXTURE0 + unit);
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    }
}

fn draw_fullscreen(gl: &glow::Context) {
    // SAFETY: draws the 3-vertex fullscreen triangle with the bound program.
    unsafe { gl.draw_arrays(glow::TRIANGLES, 0, 3) }
}
