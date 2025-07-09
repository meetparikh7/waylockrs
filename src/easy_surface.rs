//! A double-buffered surface that attempts to be easy to use

use smithay_client_toolkit::{
    globals::ProvidesBoundGlobal,
    shm::slot::{Buffer, Slot, SlotPool},
};
use wayland_client::{
    QueueHandle,
    protocol::{wl_callback, wl_shm, wl_surface::WlSurface},
};

struct EasySurfaceInner {
    pool: SlotPool,
    width: i32,
    height: i32,
    slots: (Slot, Slot),
    buffers: (Buffer, Buffer),
}

pub struct EasySurface {
    surface: WlSurface,
    format: wl_shm::Format,
    inner: Option<EasySurfaceInner>,
}

impl EasySurface {
    pub fn new(surface: WlSurface, format: wl_shm::Format) -> Self {
        Self {
            surface,
            format,
            inner: None,
        }
    }

    pub fn configure(
        &mut self,
        shm: &impl ProvidesBoundGlobal<wl_shm::WlShm, 1>,
        width: i32,
        height: i32,
    ) {
        let stride = width * 4;
        let size = (stride as usize) * (height as usize);
        let mut pool = SlotPool::new(size, shm).expect("Failed to create pool");
        let slots = (
            pool.new_slot(size).expect("Failed to create slot"),
            pool.new_slot(size).expect("Failed to create slot"),
        );
        let buffers = (
            pool.create_buffer_in(&slots.0, width, height, stride, self.format)
                .expect("Failed to create Buffer"),
            pool.create_buffer_in(&slots.1, width, height, stride, self.format)
                .expect("Failed to create Buffer"),
        );
        self.inner = Some(EasySurfaceInner {
            pool,
            width,
            height,
            slots,
            buffers,
        });
    }

    #[allow(dead_code)]
    pub fn wl_surface(&self) -> &WlSurface {
        &self.surface
    }

    pub fn get_size(&self) -> (i32, i32) {
        let inner = self.inner.as_ref().expect("Not configured");
        (inner.width, inner.height)
    }

    fn get_active(&mut self) -> Option<(&mut Buffer, &mut [u8])> {
        let inner = self.inner.as_mut().expect("Get inner");
        let buffer = if inner.slots.0.has_active_buffers() {
            &mut inner.buffers.1
        } else {
            &mut inner.buffers.0
        };
        if buffer.slot().has_active_buffers() {
            return None;
        }

        let canvas = inner.pool.canvas(buffer).expect("Unable to get canvas");
        return Some((buffer, canvas));
    }

    pub fn render<F, D>(&mut self, qh: &QueueHandle<D>, render: F)
    where
        F: FnOnce(&mut Buffer, &mut [u8], i32, i32) -> (),
        D: wayland_client::Dispatch<wl_callback::WlCallback, WlSurface> + 'static,
    {
        if self.inner.is_none() {
            // Not configured
            return;
        }

        let surface_copy = self.surface.clone();
        let (width, height) = self.get_size();

        // Render and commit if buffers are available, otherwise do nothing as the
        // other invoker would trigger a next frame
        if let Some((buffer, data)) = self.get_active() {
            render(buffer, data, width, height);
            buffer.attach_to(&surface_copy).expect("buffer attach");
            self.surface.damage_buffer(0, 0, width, height);
            self.surface.commit();
            self.surface.frame(qh, surface_copy);
        }
    }
}
