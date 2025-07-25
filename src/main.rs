mod auth;
mod background_image;
mod cairo_extras;
mod config;
mod easy_surface;
mod keyboard_state;
mod overlay;
mod swaylock_config;

use crate::{
    auth::{PasswordBuffer, create_and_run_auth_loop},
    cairo_extras::CairoExtras,
    keyboard_state::KeyboardState,
};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

use log::error;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_output, delegate_registry, delegate_seat,
    delegate_session_lock, delegate_shm, delegate_subcompositor,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, LoopHandle, LoopSignal, channel},
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

    let xdg_dirs = xdg::BaseDirectories::new();
    let config_path = Path::new("waylockrs/config.toml");
    let config_str = match xdg_dirs.get_config_file(config_path) {
        Some(file) => {
            if file.exists() {
                std::fs::read_to_string(file).unwrap()
            } else {
                swaylock_config::try_mapping_swalock_config(&xdg_dirs, &config_path)
            }
        }
        None => {
            error!("Unable to retrieve XDG config directory. Using empty config.");
            "".to_string()
        }
    };

    let config = Config::parse(&config_str);
    if config.show_help {
        println!("Usage: waylockrs --background-image path/to/image");
        println!("Please refer to the default config for all options");
        println!("");
        println!("Note: config can be specified in $XDG_CONFIG_DIR/waylockrs/config.toml");
        println!("Note: or via CLI, e.g. --clock.font-size=100.0");
        return;
    }

    if config.daemonize {
        daemon(false, true).unwrap();
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

    let background_image = if config.background_mode != config::BackgroundMode::SolidColor {
        match &config.background_image {
            Some(path) => Some(load_image(&path)),
            None => None,
        }
    } else {
        None
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
        keyboard: KeyboardState::new(None),
        password: PasswordBuffer::new(),
        lifecycle: LifeCycle::Initing,
        end_signal: event_loop.get_signal(),
        auth_req_send: None,
        indicator: Indicator {
            config: config.indicator.clone(),
            input_state: overlay::InputState::Idle,
            auth_state: overlay::AuthState::Idle,
            failed_attempts: overlay::AttemptsCounter::new(),
            is_caps_lock: false,
            last_update: Instant::now(),
            highlight_start: 0,
        },
        clock: Clock {
            config: config.clock.clone(),
        },
        sigusr_received: Arc::new(AtomicBool::new(false)),
    };

    // Early dispatch to fastly create lock surfaces
    event_loop.dispatch(None, &mut state).unwrap();
    let lock = state.session_lock_state.lock(&qh).expect("Could not lock");
    for output in state.output_state.outputs() {
        state.create_lock_surface(&qh, &lock, output);
    }
    state.draw(&conn, &qh);

    state.create_auth_channel(&mut event_loop);
    state.create_sigusr_interrupt_handler();

    event_loop
        .run(None, &mut state, |state| {
            state.lifecycle = match state.lifecycle {
                LifeCycle::Initing => {
                    if state.lock.is_some() {
                        state.notify_ready_fd();
                        LifeCycle::Locked
                    } else {
                        LifeCycle::Initing
                    }
                }
                LifeCycle::Locked => {
                    if state
                        .sigusr_received
                        .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        if let Some(lock) = state.lock.take() {
                            lock.unlock();
                        }
                        state.lock_surfaces.clear();
                        LifeCycle::Authenticated
                    } else {
                        LifeCycle::Locked
                    }
                }
                LifeCycle::Authenticated => LifeCycle::Ended,
                LifeCycle::Ended => {
                    state.end_signal.stop();
                    LifeCycle::Ended
                }
            };
        })
        .unwrap();
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum LifeCycle {
    Initing,
    Locked,
    Authenticated,
    Ended,
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
    keyboard: KeyboardState,
    lock: Option<SessionLock>,
    password: PasswordBuffer,
    lifecycle: LifeCycle,
    end_signal: LoopSignal,
    auth_req_send: Option<channel::Sender<PasswordBuffer>>,
    indicator: Indicator,
    clock: Clock,
    sigusr_received: Arc<AtomicBool>,
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
        _time: u32,
    ) {
        self.draw(conn, qh);
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
        if let Some(lock) = self.lock.take() {
            self.create_lock_surface(qh, &lock, output);
            self.lock = Some(lock);
        }
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
            let keyboard = self
                .seat_state
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    self.loop_handle.clone(),
                    Box::new(|state, _wl_kbd, event| {
                        state.handle_key_press_or_repeat(event);
                    }),
                )
                .expect("Failed to get keyboard");
            self.keyboard = KeyboardState::new(Some(keyboard));
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
        layout: u32,
    ) {
        self.keyboard.is_caps_lock = modifiers.caps_lock;
        self.keyboard.is_control = modifiers.ctrl;
        self.keyboard.set_active_layout(layout);
    }

    fn update_keymap(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        keymap: keyboard::Keymap<'_>,
    ) {
        self.keyboard.parse_keymap_layouts(keymap);
    }
}

impl SessionLockHandler for State {
    fn locked(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, session_lock: SessionLock) {
        for output in self.output_state.outputs() {
            self.create_lock_surface(qh, &session_lock, output);
        }
        self.lock = Some(session_lock);
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
        self.draw(conn, qh);
    }
}

pub fn daemon(nochdir: bool, noclose: bool) -> Result<(), i32> {
    use libc::c_int;
    let res = unsafe { libc::daemon(nochdir as c_int, noclose as c_int) };
    if res == 0 {
        Ok(())
    } else {
        error!("Failed to daemonize with rc {res}");
        Err(res)
    }
}

impl State {
    pub fn create_auth_channel(&mut self, event_loop: &mut EventLoop<Self>) {
        let (auth_req_send, auth_res_recv) = create_and_run_auth_loop();
        self.auth_req_send = Some(auth_req_send);
        event_loop
            .handle()
            .insert_source(auth_res_recv, |evt, _metadata, state| match evt {
                channel::Event::Msg(status) => {
                    if status {
                        if let Some(lock) = state.lock.take() {
                            lock.unlock();
                        }
                        state.lock_surfaces.clear();
                        state.lifecycle = LifeCycle::Authenticated;
                    } else {
                        state.indicator.auth_state = overlay::AuthState::Invalid;
                        state.indicator.failed_attempts.inc();
                        state.indicator.last_update = Instant::now();
                    }
                }
                channel::Event::Closed => {
                    if state.lifecycle == LifeCycle::Locked {
                        panic!("Auth loop closed early!")
                    }
                }
            })
            .unwrap();
    }

    pub fn create_sigusr_interrupt_handler(&self) {
        const SIGUSR1: i32 = 10;
        match signal_hook::flag::register(SIGUSR1, self.sigusr_received.clone()) {
            Ok(_) => {}
            Err(err) => error!("Failed to register SIGUSR1 handling with {err}"),
        };
    }

    pub fn notify_ready_fd(&mut self) {
        use std::io::Write;
        use std::os::fd::FromRawFd;

        if self.config.ready_fd >= 0 {
            let mut f = unsafe { std::fs::File::from_raw_fd(self.config.ready_fd) };
            match write!(&mut f, "\n") {
                Ok(_) => {}
                Err(err) => {
                    error!("Failed to send readiness notification with error {err}")
                }
            };
            self.config.ready_fd = -1;
        }
    }

    pub fn create_lock_surface(
        &mut self,
        qh: &QueueHandle<Self>,
        lock: &SessionLock,
        output: wl_output::WlOutput,
    ) {
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
            if self.config.ignore_empty_password && self.password.unsecure().len() == 0 {
                // pass
            } else if self.indicator.auth_state == overlay::AuthState::Validating {
                // pass
            } else {
                let password = self.password.take();
                self.auth_req_send.as_ref().unwrap().send(password).unwrap();
                self.indicator.auth_state = overlay::AuthState::Validating;
                self.indicator.input_state = overlay::InputState::Idle;
            }
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

    pub fn draw(&mut self, _conn: &Connection, qh: &QueueHandle<Self>) {
        if Instant::now() - self.indicator.last_update >= Duration::from_secs(3) {
            self.indicator.input_state = overlay::InputState::Idle;
            self.indicator.auth_state = overlay::AuthState::Idle;
        }
        let mut requested_reframe = false;
        for lock_surface in &mut self.lock_surfaces.values_mut() {
            let rendered = lock_surface.indicator_surface.render(
                qh,
                !requested_reframe,
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
                        self.indicator
                            .draw(&context, width, height, 1.0, &self.keyboard);
                    }
                    if self.config.show_clock {
                        self.clock.draw(&context, width, height, 1.0);
                    }
                },
            );
            requested_reframe = requested_reframe || rendered;

            let rendered = lock_surface.base_surface.render(
                qh,
                !requested_reframe,
                |_buffer, canvas, width, height, resized| {
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
                },
            );
            requested_reframe = requested_reframe || rendered;
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
