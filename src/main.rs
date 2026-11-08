//! Galaxy: a real-time 3D Barnes-Hut N-body sandbox rendered with raw OpenGL.

mod app;
mod camera;
mod gfx;
mod math;
mod sim;

use std::process::ExitCode;

fn main() -> ExitCode {
    match app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
