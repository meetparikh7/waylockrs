mod auth;
mod background_image;
mod constants;
mod easy_surface;

use std::env;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_registry, delegate_seat,
    delegate_shm, delegate_subcompositor, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        self, SeatHandler, SeatState,
        keyboard::{self, KeyboardHandler},
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler},
    subcompositor::SubcompositorState,
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
};

use crate::{
    background_image::{BackgroundMode, load_image, render_background_image},
    easy_surface::EasySurface,
};

fn main() {
    env_logger::init();

    let conn = Connection::connect_to_env().unwrap();

    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let compositor_state =
        CompositorState::bind(&globals, &qh).expect("wl_compositor not available");
    let subcompositor_state =
        SubcompositorState::bind(compositor_state.wl_compositor().clone(), &globals, &qh)
            .expect("wl_subcompositor not available");

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        subcompositor_state,
        seat_state: SeatState::new(&globals, &qh),
        shm_state: Shm::bind(&globals, &qh).expect("wl_shm not available"),
        xdg_shell_state: XdgShell::bind(&globals, &qh).expect("xdg shell not available"),

        windows: Vec::new(),
        password: String::new(),
    };

    for path in env::args_os().skip(1) {
        let surface = state.compositor_state.create_surface(&qh);
        let (indicator_subsurface, indicator_surface) = state
            .subcompositor_state
            .create_subsurface(surface.clone(), &qh);

        indicator_subsurface.set_sync();
        indicator_subsurface.set_position(0, 0);

        let window = state.xdg_shell_state.create_window(
            surface.clone(),
            WindowDecorations::ServerDefault,
            &qh,
        );
        window.set_title("A wayland window");
        // GitHub does not let projects use the `org.github` domain but the `io.github` domain is fine.
        window.set_app_id("io.github.smithay.client-toolkit.ImageViewer");

        // In order for the window to be mapped, we need to perform an initial commit with no attached buffer.
        // For more info, see WaylandSurface::commit
        //
        // The compositor will respond with an initial configure that we can then use to present to the window with
        // the correct options.
        window.commit();

        state.windows.push(ImageViewer {
            window,
            image: load_image(&path.to_string_lossy()),
            base_surface: EasySurface::new(surface, wl_shm::Format::Argb8888),
            indicator_surface: EasySurface::new(indicator_surface, wl_shm::Format::Argb8888),
        });
    }

    if state.windows.is_empty() {
        println!("USAGE: ./image_viewer <PATH> [<PATH>]...");
        return;
    }

    // We don't draw immediately, the configure will notify us when to first draw.

    loop {
        event_queue.blocking_dispatch(&mut state).unwrap();

        if state.windows.is_empty() {
            println!("exiting example");
            break;
        }
    }
}

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    subcompositor_state: SubcompositorState,
    shm_state: Shm,
    seat_state: SeatState,
    xdg_shell_state: XdgShell,

    windows: Vec<ImageViewer>,
    password: String,
}

struct ImageViewer {
    window: Window,
    image: cairo::ImageSurface,
    base_surface: EasySurface,
    indicator_surface: EasySurface,
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // Not needed for this example.
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Not needed for this example.
    }

    fn frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        time: u32,
    ) {
        self.draw(conn, qh, time);
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        self.windows.retain(|v| v.window != *window);
    }

    fn configure(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        for viewer in &mut self.windows {
            if viewer.window != *window {
                continue;
            }

            let width = configure.new_size.0.map(|v| v.get()).unwrap_or(256);
            let height = configure.new_size.1.map(|v| v.get()).unwrap_or(256);

            viewer
                .base_surface
                .configure(&self.shm_state, width as i32, height as i32);
            viewer
                .indicator_surface
                .configure(&self.shm_state, width as i32, height as i32);
        }
        self.draw(conn, qh, 0);
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: seat::Capability,
    ) {
        if capability == seat::Capability::Keyboard {
            let _keyboard = self.seat_state.get_keyboard(&qh, &seat, Option::None);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: seat::Capability,
    ) {
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[keyboard::Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: keyboard::KeyEvent,
    ) {
        if event.keysym == keyboard::Keysym::Return {
            let verification = auth::verify(&self.password);
            println!("Auth verify {:?} {:?}", verification, &self.password);
            self.password.clear();
        } else if let Some(input) = &event.utf8 {
            self.password.push_str(&input);
        }
        println!("press {:?} {:?}", event.utf8, event.keysym);
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: keyboard::KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: keyboard::Modifiers,
        _layout: u32,
    ) {
    }
}

impl State {
    pub fn draw(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, time: u32) {
        for viewer in &mut self.windows {
            viewer
                .indicator_surface
                .render(qh, |_buffer, canvas, width, height, _resized| {
                    let stride = width * 4;
                    let cairo_surface = unsafe {
                        cairo::ImageSurface::create_for_data_unsafe(
                            canvas.first_mut().unwrap(),
                            cairo::Format::ARgb32,
                            width,
                            height,
                            stride,
                        )
                        .unwrap()
                    };
                    let context = cairo::Context::new(&cairo_surface).unwrap();
                    context.set_source_rgba(1.0, 1.0, 1.0, 0.0);
                    context.set_operator(cairo::Operator::Source);
                    context.paint().unwrap();
                    context.set_source_rgb(1.0, 1.0, 1.0);
                    context.rectangle((time as i32 % width) as f64, 200.0, 200.0, 500.0);
                    context.stroke().unwrap();
                });

            viewer
                .base_surface
                .render(qh, |_buffer, canvas, width, height, resized| {
                    if resized {
                        let stride = width * 4;
                        println!("Resized {resized}");
                        let cairo_surface = unsafe {
                            cairo::ImageSurface::create_for_data_unsafe(
                                canvas.first_mut().unwrap(),
                                cairo::Format::ARgb32,
                                width,
                                height,
                                stride,
                            )
                            .unwrap()
                        };
                        let context = cairo::Context::new(&cairo_surface).unwrap();
                        context.set_antialias(cairo::Antialias::Best);
                        context.save().unwrap();

                        context.set_operator(cairo::Operator::Source);
                        context.set_source_rgb(1.0, 1.0, 1.0);
                        context.paint().unwrap();
                        context.save().unwrap();

                        context.set_operator(cairo::Operator::Over);
                        render_background_image(
                            &context,
                            &viewer.image,
                            BackgroundMode::Fit,
                            width,
                            height,
                        );
                        context.restore().unwrap();
                        context.identity_matrix();
                    }
                });
        }
    }
}

delegate_compositor!(State);
delegate_subcompositor!(State);
delegate_output!(State);
delegate_xdg_shell!(State);
delegate_xdg_window!(State);
delegate_shm!(State);

delegate_seat!(State);
delegate_keyboard!(State);

delegate_registry!(State);

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers!(OutputState);
}
