mod auth;
mod constants;
mod easy_surface;

use std::env;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_xdg_shell,
    delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};

use crate::easy_surface::EasySurface;

pub fn old_main() {
    match rpassword::prompt_password("Enter password: ") {
        Err(_) => eprintln!("Failed to get password"),
        Ok(password) => println!("{:?}", auth::verify(password.as_str())),
    };
}

fn main() {
    env_logger::init();

    let conn = Connection::connect_to_env().unwrap();

    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor_state: CompositorState::bind(&globals, &qh)
            .expect("wl_compositor not available"),
        shm_state: Shm::bind(&globals, &qh).expect("wl_shm not available"),
        xdg_shell_state: XdgShell::bind(&globals, &qh).expect("xdg shell not available"),

        pool: None,
        windows: Vec::new(),
    };

    let mut pool_size = 0;

    for path in env::args_os().skip(1) {
        let image = match image::open(&path) {
            Ok(i) => i,
            Err(e) => {
                println!("Failed to open image {}.", path.to_string_lossy());
                println!("Error was: {e:?}");
                return;
            }
        };

        // We'll need the image in RGBA for drawing it
        let image = image.to_rgba8();

        let surface = state.compositor_state.create_surface(&qh);

        pool_size += image.width() * image.height() * 4;

        let window =
            state
                .xdg_shell_state
                .create_window(surface, WindowDecorations::ServerDefault, &qh);
        window.set_title("A wayland window");
        // GitHub does not let projects use the `org.github` domain but the `io.github` domain is fine.
        window.set_app_id("io.github.smithay.client-toolkit.ImageViewer");

        // In order for the window to be mapped, we need to perform an initial commit with no attached buffer.
        // For more info, see WaylandSurface::commit
        //
        // The compositor will respond with an initial configure that we can then use to present to the window with
        // the correct options.
        window.commit();

        let surface = window.wl_surface().clone();
        state.windows.push(ImageViewer {
            window,
            image,
            buffer: EasySurface::new(surface, wl_shm::Format::Argb8888),
        });
    }

    let pool = SlotPool::new(pool_size as usize, &state.shm_state).expect("Failed to create pool");
    state.pool = Some(pool);

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
    shm_state: Shm,
    xdg_shell_state: XdgShell,

    pool: Option<SlotPool>,
    windows: Vec<ImageViewer>,
}

struct ImageViewer {
    window: Window,
    image: image::RgbaImage,
    buffer: EasySurface,
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
                .buffer
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

impl State {
    pub fn draw(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, time: u32) {
        for viewer in &mut self.windows {
            viewer.buffer.render(qh, |_buffer, canvas, width, height| {
                let image = image::imageops::resize(
                    &viewer.image,
                    width as u32,
                    height as u32,
                    image::imageops::FilterType::Nearest,
                );

                let image = image::imageops::huerotate(&image, (time % 360) as i32);

                for (pixel, argb) in image.pixels().zip(canvas.chunks_exact_mut(4)) {
                    // We do this in an horribly inefficient manner, for the sake of simplicity.
                    // We'll send pixels to the server in ARGB8888 format (this is one of the only
                    // formats that are guaranteed to be supported), but image provides it in
                    // big-endian RGBA8888, so we need to do the conversion.
                    argb[3] = pixel.0[3];
                    argb[2] = pixel.0[0];
                    argb[1] = pixel.0[1];
                    argb[0] = pixel.0[2];
                }
            });
        }
    }
}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);

delegate_xdg_shell!(State);
delegate_xdg_window!(State);

delegate_registry!(State);

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers!(OutputState);
}
