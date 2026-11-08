//! OpenGL 3.3 core rendering through `glow`: star sprites, HDR bloom and HUD text.

pub mod bloom;
mod font;
pub mod stars;
pub mod text;

use glow::HasContext;

/// Compiles and links a vertex + fragment shader pair.
fn link_program(gl: &glow::Context, vertex: &str, fragment: &str) -> Result<glow::Program, String> {
    // SAFETY: plain GL calls on objects created here; the context is current.
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::with_capacity(2);
        let mut failure = None;
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                failure = Some(format!(
                    "shader compile error: {}",
                    gl.get_shader_info_log(shader)
                ));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        if failure.is_none() {
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                failure = Some(format!(
                    "program link error: {}",
                    gl.get_program_info_log(program)
                ));
            }
        }
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        match failure {
            Some(message) => {
                gl.delete_program(program);
                Err(message)
            }
            None => Ok(program),
        }
    }
}

/// The bytes of an `f32` slice, for buffer uploads.
fn f32_bytes(data: &[f32]) -> &[u8] {
    // SAFETY: `f32` has no padding or invalid bit patterns, `u8` has alignment
    // 1, and the length covers exactly the same memory.
    unsafe { std::slice::from_raw_parts(data.as_ptr().cast(), std::mem::size_of_val(data)) }
}
