mod auth;
mod background_image;
mod cairo_extras;
mod config;
mod constants;
mod easy_surface;
mod overlay;

use crate::{auth::create_and_run_auth_loop, cairo_extras::CairoExtras};
use std::time::{Duration, Instant};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_registry, delegate_seat,
    delegate_shm, delegate_subcompositor, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::channel,
        calloop::{EventLoop, LoopHandle},
        calloop_wayland_source::WaylandSource,
    },
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
    config::Config,
    easy_surface::EasySurface,
    overlay::{Clock, Indicator},
};

fn main() {
    env_logger::init();

    let config_str = std::fs::read_to_string("config.toml").unwrap();
    let config = Config::parse(&config_str);
    println!("{:?}", config);

    let conn = Connection::connect_to_env().unwrap();

    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let compositor_state =
        CompositorState::bind(&globals, &qh).expect("wl_compositor not available");
    let subcompositor_state =
        SubcompositorState::bind(compositor_state.wl_compositor().clone(), &globals, &qh)
            .expect("wl_subcompositor not available");

    let mut event_loop: EventLoop<State> =
        EventLoop::try_new().expect("failed to initialize the event loop");
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle)
        .expect("Failed to insert loop_handle");

    let (auth_req_send, auth_res_recv) = create_and_run_auth_loop();

    event_loop
        .handle()
        .insert_source(auth_res_recv, |evt, _metadata, state| match evt {
            channel::Event::Msg(status) => {
                state.authenticated = state.authenticated || status;
            }
            channel::Event::Closed => {
                if !state.authenticated {
                    panic!("Auth loop closed early!")
                }
            }
        })
        .unwrap();

    let mut state = State {
        loop_handle: event_loop.handle(),
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        subcompositor_state,
        seat_state: SeatState::new(&globals, &qh),
        shm_state: Shm::bind(&globals, &qh).expect("wl_shm not available"),
        xdg_shell_state: XdgShell::bind(&globals, &qh).expect("xdg shell not available"),

        config: config.clone(),
        windows: Vec::new(),
        keyboard: None,
        password: String::new(),
        authenticated: false,
        auth_req_send,
        indicator: Indicator {
            config: config.indicator.clone(),
            input_state: overlay::InputState::Idle,
            auth_state: overlay::AuthState::Idle,
            is_caps_lock: false,
            last_update: Instant::now(),
            highlight_start: 0,
        },
        clock: Clock {
            config: config.clock.clone(),
        },
    };

    {
        let path = state.config.background_image.clone();
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
            image: load_image(&path.unwrap()),
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
        event_loop
            .dispatch(None, &mut state)
            .expect("Failed to run");

        if state.authenticated {
            state.windows.clear();
        }
        if state.windows.is_empty() {
            println!("exiting example");
            break;
        }
    }
}

struct State {
    loop_handle: LoopHandle<'static, Self>,
    registry_state: RegistryState,
    output_state: OutputState,
    compositor_state: CompositorState,
    subcompositor_state: SubcompositorState,
    shm_state: Shm,
    seat_state: SeatState,
    xdg_shell_state: XdgShell,

    config: Config,
    windows: Vec<ImageViewer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    password: String,
    authenticated: bool,
    auth_req_send: channel::Sender<String>,
    indicator: Indicator,
    clock: Clock,
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
            self.keyboard = Some(
                self.seat_state
                    .get_keyboard_with_repeat(
                        qh,
                        &seat,
                        None,
                        self.loop_handle.clone(),
                        Box::new(|state, _wl_kbd, event| {
                            println!("repeat {:?} {:?}", event.utf8, event.keysym);
                            state.handle_key_press_or_repeat(event);
                        }),
                    )
                    .expect("Failed to get keyboard"),
            )
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
        println!("press {:?} {:?}", event.utf8, event.keysym);
        self.handle_key_press_or_repeat(event);
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
        modifiers: keyboard::Modifiers,
        _layout: u32,
    ) {
        self.indicator.is_caps_lock = modifiers.caps_lock;
    }
}

impl State {
    pub fn handle_key_press_or_repeat(&mut self, event: keyboard::KeyEvent) {
        if event.keysym == keyboard::Keysym::Return {
            let password = std::mem::take(&mut self.password);
            self.auth_req_send.send(password).unwrap();
        } else if event.keysym == keyboard::Keysym::BackSpace {
            if self.password.len() != 0 {
                self.password = self.password[0..self.password.len() - 1].to_string();
            }
            self.indicator.input_state = if self.password.len() == 0 {
                overlay::InputState::Clear
            } else {
                overlay::InputState::Backspace
            };
            self.indicator.last_update = Instant::now();
        } else if let Some(input) = &event.utf8 {
            self.password.push_str(&input);
            self.indicator.input_state = overlay::InputState::Letter;
            self.indicator.last_update = Instant::now();
        } else {
            self.indicator.input_state = overlay::InputState::Neutral;
            self.indicator.last_update = Instant::now();
        }
        self.indicator.highlight_start = rand::random::<u32>() % 2048;
    }

    pub fn draw(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _time: u32) {
        if Instant::now() - self.indicator.last_update >= Duration::from_secs(3) {
            self.indicator.input_state = overlay::InputState::Idle;
            self.indicator.auth_state = overlay::AuthState::Idle;
        }
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

                    // Clear
                    context.save().unwrap();
                    context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
                    context.set_operator(cairo::Operator::Source);
                    context.paint().unwrap();
                    context.restore().unwrap();

                    if self.config.show_indicator {
                        self.indicator.draw(&context, width, height, 1.0);
                    }
                    if self.config.show_clock {
                        self.clock.draw(&context, width, height, 1.0);
                    }
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
                        context.set_source_color(&self.config.background_color);
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
