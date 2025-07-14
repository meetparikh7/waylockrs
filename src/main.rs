mod auth;
mod background_image;
mod cairo_extras;
mod config;
mod constants;
mod easy_surface;
mod overlay;

use crate::{
    auth::{PasswordBuffer, create_and_run_auth_loop},
    cairo_extras::CairoExtras,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_registry, delegate_seat,
    delegate_session_lock, delegate_shm, delegate_subcompositor,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, LoopHandle, channel},
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        self, SeatHandler, SeatState,
        keyboard::{self, KeyboardHandler},
    },
    session_lock::{
        SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
        SessionLockSurfaceConfigure,
    },
    shm::{Shm, ShmHandler},
    subcompositor::SubcompositorState,
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    backend::ObjectId,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
};

use crate::{
    background_image::{load_image, render_background_image},
    config::Config,
    easy_surface::EasySurface,
    overlay::{Clock, Indicator},
};

fn main() {
    env_logger::init();

    let config_str = std::fs::read_to_string("config.toml").unwrap();
    let config = Config::parse(&config_str);
    if config.show_help {
        println!("Usage: funlock --background-image path/to/image");
        println!("Please refer to the default config for all options");
        println!("");
        println!("Note: config can be specified in $XDG_CONFIG_DIR/funlock/config.toml");
        println!("Note: or via CLI, e.g. --clock.font-size=100.0");
        return;
    }

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
                if status && state.lock.is_some() {
                    let lock = state.lock.take();
                    lock.unwrap().unlock();
                    state.lock_surfaces.clear();
                } else {
                    state.indicator.auth_state = overlay::AuthState::Invalid;
                }
            }
            channel::Event::Closed => {
                if !state.authenticated {
                    panic!("Auth loop closed early!")
                }
            }
        })
        .unwrap();

    let background_image = match &config.background_image {
        Some(path) => Some(load_image(&path)),
        None => None,
    };

    let mut state = State {
        loop_handle: event_loop.handle(),
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state,
        subcompositor_state,
        seat_state: SeatState::new(&globals, &qh),
        shm_state: Shm::bind(&globals, &qh).expect("wl_shm not available"),
        session_lock_state: SessionLockState::new(&globals, &qh),

        config: config.clone(),
        background_image,
        lock: None,
        lock_surfaces: HashMap::new(),
        output_to_lock_surfaces: HashMap::new(),
        keyboard: None,
        password: PasswordBuffer::new(),
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

    state.lock = Some(state.session_lock_state.lock(&qh).expect("Could not lock"));

    while !state.authenticated {
        if state.lock.is_none() {
            state.authenticated = true;
        }
        event_loop
            .dispatch(None, &mut state)
            .expect("Failed to run");
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
    session_lock_state: SessionLockState,

    config: Config,
    background_image: Option<cairo::ImageSurface>,
    lock_surfaces: HashMap<ObjectId, LockSurface>,
    output_to_lock_surfaces: HashMap<ObjectId, ObjectId>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    lock: Option<SessionLock>,
    password: PasswordBuffer,
    authenticated: bool,
    auth_req_send: channel::Sender<PasswordBuffer>,
    indicator: Indicator,
    clock: Clock,
}

struct LockSurface {
    _lock_surface: SessionLockSurface,
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
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.create_lock_surface(qh, output);
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
        output: wl_output::WlOutput,
    ) {
        if let Some(surface_id) = self.output_to_lock_surfaces.remove(&output.id()) {
            self.lock_surfaces.remove(&surface_id);
        }
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

impl SessionLockHandler for State {
    fn locked(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _session_lock: SessionLock) {
        for output in self.output_state.outputs() {
            self.create_lock_surface(qh, output);
        }
    }

    fn finished(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _session_lock: SessionLock,
    ) {
        panic!("Failed to lock session. Is another lock screen running?");
    }

    fn configure(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        let surface_id = surface.wl_surface().id();
        self.lock_surfaces.entry(surface_id).and_modify(|e| {
            let (width, height) = configure.new_size;
            let (width, height) = (width as i32, height as i32);
            e.base_surface.configure(&self.shm_state, width, height);
            e.indicator_surface
                .configure(&self.shm_state, width, height);
        });
        self.draw(conn, qh, 0);
    }
}

impl State {
    pub fn create_lock_surface(&mut self, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let lock = match self.lock.as_ref() {
            Some(lock) => lock,
            None => return,
        };

        if self.output_to_lock_surfaces.contains_key(&output.id()) {
            return;
        }

        let surface = self.compositor_state.create_surface(&qh);
        let lock_surface = lock.create_lock_surface(surface.clone(), &output, &qh);
        let surface_id = lock_surface.wl_surface().id();
        let (indicator_subsurface, indicator_surface) = self
            .subcompositor_state
            .create_subsurface(lock_surface.wl_surface().clone(), &qh);

        indicator_subsurface.set_sync();
        indicator_subsurface.set_position(0, 0);

        self.lock_surfaces.insert(
            surface_id.clone(),
            LockSurface {
                _lock_surface: lock_surface,
                base_surface: EasySurface::new(surface, wl_shm::Format::Argb8888),
                indicator_surface: EasySurface::new(indicator_surface, wl_shm::Format::Argb8888),
            },
        );
        self.output_to_lock_surfaces.insert(output.id(), surface_id);
    }

    pub fn handle_key_press_or_repeat(&mut self, event: keyboard::KeyEvent) {
        if event.keysym == keyboard::Keysym::Return {
            self.auth_req_send.send(self.password.take()).unwrap();
            self.indicator.auth_state = overlay::AuthState::Validating;
            self.indicator.input_state = overlay::InputState::Idle;
        } else if event.keysym == keyboard::Keysym::BackSpace {
            self.password.backspace();
            self.indicator.input_state = if self.password.unsecure().len() == 0 {
                overlay::InputState::Clear
            } else {
                overlay::InputState::Backspace
            };
        } else if let Some(input) = event.utf8 {
            self.password.append(input);
            self.indicator.input_state = overlay::InputState::Letter;
        } else {
            self.indicator.input_state = overlay::InputState::Neutral;
        }
        self.indicator.highlight_start = rand::random::<u32>() % 2048;
        self.indicator.last_update = Instant::now();
    }

    pub fn draw(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _time: u32) {
        if Instant::now() - self.indicator.last_update >= Duration::from_secs(3) {
            self.indicator.input_state = overlay::InputState::Idle;
        }
        for lock_surface in &mut self.lock_surfaces.values_mut() {
            lock_surface.indicator_surface.render(
                qh,
                |_buffer, canvas, width, height, _resized| {
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
                },
            );

            lock_surface
                .base_surface
                .render(qh, |_buffer, canvas, width, height, resized| {
                    if resized {
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
                        context.set_antialias(cairo::Antialias::Best);
                        context.save().unwrap();

                        context.set_operator(cairo::Operator::Source);
                        context.set_source_color(&self.config.background_color);
                        context.paint().unwrap();
                        context.save().unwrap();

                        context.set_operator(cairo::Operator::Over);
                        if let Some(image) = self.background_image.as_ref() {
                            render_background_image(
                                &context,
                                &image,
                                self.config.background_mode,
                                width,
                                height,
                            );
                        }
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
delegate_shm!(State);
delegate_session_lock!(State);

delegate_seat!(State);
delegate_keyboard!(State);

delegate_registry!(State);

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers!(OutputState);
}
