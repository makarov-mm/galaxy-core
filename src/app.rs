//! Window, GL context and the main loop. Steps the simulation, uploads star
//! positions/speeds, draws into an HDR target and composites with bloom.
//!
//! Controls:
//!   left-drag      orbit
//!   wheel          zoom
//!   Space          pause / resume
//!   Up / Down      simulation speed (substeps per frame)
//!   B              toggle bloom
//!   [ / ]          bloom intensity
//!   - / =          star brightness
//!   M              switch single galaxy / merger
//!   R              reset the current scene
//!   Esc            quit

use std::num::NonZeroU32;

use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextAttributesBuilder, NotCurrentGlContext};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasRawWindowHandle;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowBuilder;

use crate::camera::OrbitCamera;
use crate::galaxy::{self, DiskParams};
use crate::post::Bloom;
use crate::render::Renderer;
use crate::sim::Sim;

#[derive(Clone, Copy, PartialEq)]
enum Scene {
    Disk,
    Merger,
}

fn build_sim(scene: Scene) -> Sim {
    let (pos, vel, mass) = match scene {
        Scene::Disk => galaxy::make_disk(&DiskParams { n: 40_000, ..Default::default() }),
        Scene::Merger => galaxy::make_merger(20_000),
    };
    Sim::new(pos, vel, mass, 0.0015, 0.6, 0.50)
}

fn ref_speed(sim: &Sim) -> f32 {
    let mut s = 1.0f32;
    for v in &sim.vel {
        s = s.max(v.length());
    }
    s
}

fn scene_distance(scene: Scene) -> f32 {
    match scene {
        Scene::Disk => 95.0,
        Scene::Merger => 170.0,
    }
}

pub fn run() {
    // --- simulation -------------------------------------------------------
    let mut scene = Scene::Disk;
    let mut sim = build_sim(scene);

    // --- window + GL context ---------------------------------------------
    let event_loop = EventLoop::new().expect("event loop");
    let window_builder = WindowBuilder::new()
        .with_title("Galaxy — Barnes-Hut N-body")
        .with_inner_size(LogicalSize::new(1280.0, 800.0));

    let template = ConfigTemplateBuilder::new().with_depth_size(0);
    let display_builder = DisplayBuilder::new().with_window_builder(Some(window_builder));

    let (window, gl_config) = display_builder
        .build(&event_loop, template, |configs| {
            configs
                .reduce(|a, b| if b.num_samples() > a.num_samples() { b } else { a })
                .unwrap()
        })
        .expect("build display");
    let window = window.expect("window");

    let raw_handle = window.raw_window_handle();
    let gl_display = gl_config.display();
    let context_attributes = ContextAttributesBuilder::new().build(Some(raw_handle));
    let not_current = unsafe {
        gl_display
            .create_context(&gl_config, &context_attributes)
            .expect("create context")
    };

    let attrs = window.build_surface_attributes(Default::default());
    let gl_surface: Surface<WindowSurface> = unsafe {
        gl_display
            .create_window_surface(&gl_config, &attrs)
            .expect("create surface")
    };
    let gl_context = not_current.make_current(&gl_surface).expect("make current");

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            let cs = std::ffi::CString::new(s).unwrap();
            gl_display.get_proc_address(&cs) as *const _
        })
    };

    let _ = gl_surface.set_swap_interval(
        &gl_context,
        SwapInterval::Wait(NonZeroU32::new(1).unwrap()),
    );

    let size = window.inner_size();
    let mut renderer = Renderer::new(&gl, 1.0 / ref_speed(&sim));
    let mut bloom = Bloom::new(&gl, size.width as i32, size.height as i32);
    let mut cam = OrbitCamera::new(scene_distance(scene), size.width as f32 / size.height.max(1) as f32);

    // --- interaction state -----------------------------------------------
    let mut dragging = false;
    let mut last_cursor = (0.0f32, 0.0f32);
    let mut paused = false;
    let mut bloom_on = true;
    let mut substeps: u32 = 1;
    let mut interleaved = vec![0.0f32; sim.len() * 4];

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Poll);
            match event {
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::CloseRequested => elwt.exit(),

                    WindowEvent::Resized(s) => {
                        if let (Some(w), Some(h)) =
                            (NonZeroU32::new(s.width), NonZeroU32::new(s.height))
                        {
                            gl_surface.resize(&gl_context, w, h);
                            bloom.resize(&gl, s.width as i32, s.height as i32);
                            cam.set_aspect(s.width as f32 / s.height as f32);
                        }
                    }

                    WindowEvent::MouseInput { state, button, .. } => {
                        if button == MouseButton::Left {
                            dragging = state == ElementState::Pressed;
                        }
                    }

                    WindowEvent::CursorMoved { position, .. } => {
                        let p = (position.x as f32, position.y as f32);
                        if dragging {
                            cam.rotate(p.0 - last_cursor.0, p.1 - last_cursor.1);
                        }
                        last_cursor = p;
                    }

                    WindowEvent::MouseWheel { delta, .. } => {
                        let amount = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y,
                            MouseScrollDelta::PixelDelta(d) => d.y as f32 / 50.0,
                        };
                        cam.zoom(amount);
                    }

                    WindowEvent::KeyboardInput { event, .. } => {
                        if event.state == ElementState::Pressed {
                            let mut rebuild = false;
                            match event.logical_key {
                                Key::Named(NamedKey::Space) => paused = !paused,
                                Key::Named(NamedKey::ArrowUp) => substeps = (substeps + 1).min(8),
                                Key::Named(NamedKey::ArrowDown) => {
                                    substeps = substeps.saturating_sub(1).max(1)
                                }
                                Key::Named(NamedKey::Escape) => elwt.exit(),
                                Key::Character(ref c) => match c.as_str() {
                                    "b" | "B" => bloom_on = !bloom_on,
                                    "r" | "R" => rebuild = true,
                                    "m" | "M" => {
                                        scene = if scene == Scene::Disk {
                                            Scene::Merger
                                        } else {
                                            Scene::Disk
                                        };
                                        rebuild = true;
                                    }
                                    "[" => bloom.intensity = (bloom.intensity - 0.1).max(0.0),
                                    "]" => bloom.intensity = (bloom.intensity + 0.1).min(3.0),
                                    "-" => renderer.brightness = (renderer.brightness - 0.05).max(0.02),
                                    "=" | "+" => renderer.brightness = (renderer.brightness + 0.05).min(2.0),
                                    _ => {}
                                },
                                _ => {}
                            }
                            if rebuild {
                                sim = build_sim(scene);
                                renderer.speed_scale = 1.0 / ref_speed(&sim);
                                interleaved = vec![0.0f32; sim.len() * 4];
                                cam.distance = scene_distance(scene);
                            }
                        }
                    }

                    WindowEvent::RedrawRequested => {
                        if !paused {
                            for _ in 0..substeps {
                                sim.step();
                            }
                        }

                        for i in 0..sim.len() {
                            let b = i * 4;
                            let p = sim.pos[i];
                            interleaved[b] = p.x;
                            interleaved[b + 1] = p.y;
                            interleaved[b + 2] = p.z;
                            interleaved[b + 3] = sim.vel[i].length();
                        }

                        let vp = cam.view_proj();
                        bloom.begin_scene(&gl);
                        renderer.draw(&gl, &vp, &interleaved, sim.len());
                        bloom.composite(&gl, bloom_on);
                        gl_surface.swap_buffers(&gl_context).expect("swap");
                    }

                    _ => {}
                },

                Event::AboutToWait => window.request_redraw(),
                _ => {}
            }
        })
        .expect("event loop run");
}
