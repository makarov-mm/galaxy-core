//! Window, GL context, input and the frame loop.
//!
//! The simulation runs on its own thread (`sim::worker`); each frame takes its
//! newest snapshot, draws the stars into an HDR target, composites bloom and
//! puts the HUD on top.
//!
//! Controls:
//!
//! ```text
//! left-drag  orbit              Space    pause / resume
//! wheel      zoom               Up/Down  simulation steps per frame
//! B          toggle bloom       [ / ]    bloom intensity
//! - / =      star brightness    M        single galaxy / merger
//! R          reset the scene    Esc      quit
//! ```

use std::error::Error;
use std::num::NonZeroU32;
use std::time::Instant;

use glutin::config::{Config, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext,
    Version,
};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::raw_window_handle::HasWindowHandle;
use winit::window::{Window, WindowId};

use crate::camera::OrbitCamera;
use crate::gfx::bloom::Bloom;
use crate::gfx::stars::StarRenderer;
use crate::gfx::text::{TextRenderer, TextStyle};
use crate::sim::galaxy::Scene;
use crate::sim::worker::{SimWorker, Snapshot};

type BoxError = Box<dyn Error>;

const TITLE: &str = "Galaxy — Barnes-Hut N-body";
const MAX_SUBSTEPS: u32 = 8;
const CONTROLS: &str = "drag orbit    wheel zoom    space pause    up/down speed\n\
                        B bloom    [ ] intensity    - = brightness    M merger    R reset    Esc quit";

/// Opens the window and runs until it is closed.
pub fn run() -> Result<(), BoxError> {
    let event_loop = EventLoop::new()?;
    // Frames are driven by `request_redraw` + vsync; with nothing to draw
    // (minimized) the loop sleeps.
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(Scene::Disk);
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

fn camera_distance(scene: Scene) -> f32 {
    match scene {
        Scene::Disk => 95.0,
        Scene::Merger => 170.0,
    }
}

struct App {
    gfx: Option<Graphics>, // created once the event loop is running
    sim: SimWorker,
    snapshot: Snapshot,
    scene: Scene,
    camera: OrbitCamera,
    paused: bool,
    substeps: u32,
    bloom_on: bool,
    dragging: bool,
    cursor: Option<PhysicalPosition<f64>>,
    fps: FpsCounter,
    error: Option<BoxError>,
}

impl App {
    fn new(scene: Scene) -> App {
        let substeps = 1;
        App {
            gfx: None,
            sim: SimWorker::spawn(scene, substeps),
            snapshot: Snapshot::default(),
            scene,
            camera: OrbitCamera::new(camera_distance(scene), 1.0),
            paused: false,
            substeps,
            bloom_on: true,
            dragging: false,
            cursor: None,
            fps: FpsCounter::default(),
            error: None,
        }
    }

    /// Records a fatal error and leaves the event loop.
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: BoxError) {
        self.error.get_or_insert(error);
        event_loop.exit();
    }

    fn resize(&mut self, size: PhysicalSize<u32>) -> Result<(), BoxError> {
        let (Some(gfx), Some(width), Some(height)) = (
            self.gfx.as_mut(),
            NonZeroU32::new(size.width),
            NonZeroU32::new(size.height),
        ) else {
            return Ok(()); // minimized: keep the old buffers
        };
        gfx.surface.resize(&gfx.context, width, height);
        gfx.bloom
            .resize(&gfx.gl, size.width as i32, size.height as i32)?;
        self.camera
            .set_aspect(size.width as f32 / size.height as f32);
        Ok(())
    }

    fn reset(&mut self, scene: Scene) {
        self.scene = scene;
        self.sim.reset(scene);
        self.camera.set_distance(camera_distance(scene));
    }

    fn on_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        // Toggles ignore auto-repeat; adjustments repeat while the key is held.
        let first_press = !event.repeat;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => event_loop.exit(),
            Key::Named(NamedKey::Space) if first_press => {
                self.paused = !self.paused;
                self.sim.set_paused(self.paused);
            }
            Key::Named(NamedKey::ArrowUp) => self.set_substeps(self.substeps + 1),
            Key::Named(NamedKey::ArrowDown) => self.set_substeps(self.substeps.saturating_sub(1)),
            Key::Character(c) => match c.to_ascii_lowercase().as_str() {
                "b" if first_press => self.bloom_on = !self.bloom_on,
                "r" if first_press => self.reset(self.scene),
                "m" if first_press => self.reset(self.scene.toggled()),
                "[" | "]" | "-" | "=" | "+" => self.adjust_look(c),
                _ => {}
            },
            _ => {}
        }
    }

    fn set_substeps(&mut self, substeps: u32) {
        self.substeps = substeps.clamp(1, MAX_SUBSTEPS);
        self.sim.set_substeps(self.substeps);
    }

    fn adjust_look(&mut self, key: &str) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let bloom = &mut gfx.bloom.intensity;
        let brightness = &mut gfx.stars.brightness;
        match key {
            "[" => *bloom = (*bloom - 0.1).max(0.0),
            "]" => *bloom = (*bloom + 0.1).min(3.0),
            "-" => *brightness = (*brightness - 0.05).max(0.02),
            _ => *brightness = (*brightness + 0.05).min(2.0),
        }
    }

    fn redraw(&mut self) -> Result<(), BoxError> {
        let Some(gfx) = self.gfx.as_mut() else {
            return Ok(());
        };
        let size = gfx.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }

        self.fps.tick();
        self.sim.tick();
        if self.sim.take_snapshot(&mut self.snapshot) {
            gfx.stars.upload(&gfx.gl, &self.snapshot.stars);
        }

        let gl = &gfx.gl;
        gfx.bloom.begin_scene(gl);
        gfx.stars.draw(
            gl,
            &self.camera.view_proj(),
            size.height as f32,
            self.snapshot.reference_speed,
        );
        gfx.bloom.composite(gl, self.bloom_on);

        // HUD: stats at the top left, key help at the bottom left. Integer
        // scaling keeps the bitmap font crisp on high-DPI displays.
        let viewport = (size.width as f32, size.height as f32);
        let scale = gfx.window.scale_factor().round().max(1.0) as f32;
        let margin = 12.0 * scale;
        let stats = format!(
            "FPS    {:>4.0}\nstars  {}\nscene  {}\nsim    {:.1} ms/step   x{} per frame{}\nbloom  {}   intensity {:.1}   bright {:.2}",
            self.fps.value,
            self.snapshot.stars.len(),
            self.scene.name(),
            self.snapshot.step_ms,
            self.substeps,
            if self.paused { "   [PAUSED]" } else { "" },
            if self.bloom_on { "on " } else { "off" },
            gfx.bloom.intensity,
            gfx.stars.brightness,
        );
        let top = TextStyle {
            x: margin,
            y: margin,
            scale,
            color: [0.75, 0.85, 1.0, 0.95],
        };
        gfx.text.draw(gl, viewport, top, &stats);
        let help_y = viewport.1 - 2.0 * TextRenderer::line_height(scale) - margin;
        let bottom = TextStyle {
            x: margin,
            y: help_y,
            scale,
            color: [0.55, 0.6, 0.7, 0.9],
        };
        gfx.text.draw(gl, viewport, bottom, CONTROLS);

        gfx.surface.swap_buffers(&gfx.context)?;
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        match Graphics::new(event_loop) {
            Ok(gfx) => {
                let size = gfx.window.inner_size();
                self.camera
                    .set_aspect(size.width as f32 / size.height.max(1) as f32);
                self.gfx = Some(gfx);
            }
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Err(e) = self.resize(size) {
                    self.fail(event_loop, e);
                }
            }
            WindowEvent::Focused(false) => self.dragging = false,
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let (true, Some(last)) = (self.dragging, self.cursor) {
                    self.camera
                        .rotate((position.x - last.x) as f32, (position.y - last.y) as f32);
                }
                self.cursor = Some(position);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(d) => d.y as f32 / 50.0,
                };
                self.camera.zoom(notches);
            }
            WindowEvent::KeyboardInput { event, .. } => self.on_key(event_loop, &event),
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.redraw() {
                    self.fail(event_loop, e);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gfx) = &self.gfx {
            let size = gfx.window.inner_size();
            if size.width > 0 && size.height > 0 {
                gfx.window.request_redraw();
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gfx) = self.gfx.take() {
            gfx.destroy();
        }
    }
}

/// The window, its GL context and everything drawn with it.
struct Graphics {
    gl: glow::Context,
    stars: StarRenderer,
    bloom: Bloom,
    text: TextRenderer,
    // Fields drop in declaration order: surface and context before the window.
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    window: Window,
}

impl Graphics {
    fn new(event_loop: &ActiveEventLoop) -> Result<Graphics, BoxError> {
        let attributes = Window::default_attributes()
            .with_title(TITLE)
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let (window, config) = DisplayBuilder::new()
            .with_window_attributes(Some(attributes))
            .build(event_loop, ConfigTemplateBuilder::new(), pick_config)?;
        let window = window.ok_or("could not create the window")?;
        let display = config.display();

        // Request 3.3 core explicitly: the shaders need it, and macOS hands
        // out a legacy 2.1 context otherwise.
        let context_attributes = ContextAttributesBuilder::new()
            .with_profile(GlProfile::Core)
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .build(Some(window.window_handle()?.as_raw()));
        // SAFETY: the window outlives the context and surface (field order).
        let context = unsafe { display.create_context(&config, &context_attributes)? };
        let surface_attributes =
            window.build_surface_attributes(SurfaceAttributesBuilder::default())?;
        // SAFETY: as above.
        let surface = unsafe { display.create_window_surface(&config, &surface_attributes)? };
        let context = context.make_current(&surface)?;
        if let Err(e) = surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::MIN)) {
            eprintln!("vsync unavailable: {e}");
        }

        // SAFETY: the context is current on this thread for the app's lifetime.
        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|name| display.get_proc_address(name).cast())
        };

        let size = window.inner_size();
        Ok(Graphics {
            stars: StarRenderer::new(&gl)?,
            bloom: Bloom::new(&gl, size.width as i32, size.height as i32)?,
            text: TextRenderer::new(&gl)?,
            gl,
            surface,
            context,
            window,
        })
    }

    fn destroy(self) {
        self.stars.destroy(&self.gl);
        self.bloom.destroy(&self.gl);
        self.text.destroy(&self.gl);
    }
}

/// The scene renders into its own HDR framebuffer, so the window needs no
/// multisampling: take the config with the fewest samples.
fn pick_config(configs: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    configs
        .min_by_key(GlConfig::num_samples)
        .expect("no OpenGL framebuffer configuration available")
}

/// Exponentially smoothed frame rate.
#[derive(Default)]
struct FpsCounter {
    last: Option<Instant>,
    value: f32,
}

impl FpsCounter {
    fn tick(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last.replace(now) {
            let fps = 1.0 / now.duration_since(last).as_secs_f32().max(1e-4);
            self.value = if self.value == 0.0 {
                fps
            } else {
                0.92 * self.value + 0.08 * fps
            };
        }
    }
}
