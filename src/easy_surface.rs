//! A double-buffered surface that attempts to be easy to use

use smithay_client_toolkit::{
    globals::ProvidesBoundGlobal,
    shm::slot::{Buffer, Slot, SlotPool},
};
use wayland_client::{
    QueueHandle,
    protocol::{wl_callback, wl_shm, wl_surface::WlSurface},
};

struct EasySlotBuffer {
    slot: Slot,
    buffer: Buffer,
    resized: bool,
}

struct EasySurfaceInner {
    pool: SlotPool,
    slot_1: EasySlotBuffer,
    slot_2: EasySlotBuffer,
    width: i32,
    height: i32,
}

pub struct EasySurface {
    surface: WlSurface,
    format: wl_shm::Format,
    inner: Option<EasySurfaceInner>,
}

impl EasySurfaceInner {
    fn get_active(&mut self) -> Option<(&mut EasySlotBuffer, &mut [u8])> {
        let buffer = if self.slot_1.slot.has_active_buffers() {
            &mut self.slot_2
        } else {
            &mut self.slot_1
        };
        if buffer.slot.has_active_buffers() {
            return None;
        }
        let canvas = buffer.slot.canvas(&mut self.pool).unwrap();
        Some((buffer, canvas))
    }
}

impl EasySurface {
    pub fn new(surface: WlSurface, format: wl_shm::Format) -> Self {
        Self {
            surface,
            format,
            inner: None,
        }
    }

    pub fn get_size(&self) -> Option<(i32, i32)> {
        match self.inner.as_ref() {
            Some(inner) => Some((inner.width, inner.height)),
            None => None,
        }
    }

    pub fn configure(
        &mut self,
        shm: &impl ProvidesBoundGlobal<wl_shm::WlShm, 1>,
        width: i32,
        height: i32,
    ) {
        let old_size = self.get_size();
        if let Some((old_width, old_height)) = old_size
            && old_width == width
            && old_height == height
        {
            return;
        }

        let stride = width * 4;
        let size = (stride as usize) * (height as usize);
        let mut pool = SlotPool::new(size, shm).expect("Failed to create pool");
        let create = |pool: &mut SlotPool| {
            let slot = pool.new_slot(size).expect("Failed to create slot");
            let buffer = pool
                .create_buffer_in(&slot, width, height, stride, self.format)
                .expect("Failed to create Buffer");
            return EasySlotBuffer {
                slot,
                buffer,
                resized: true,
            };
        };
        let slots = (create(&mut pool), create(&mut pool));
        self.inner = Some(EasySurfaceInner {
            pool,
            slot_1: slots.0,
            slot_2: slots.1,
            width,
            height,
        });
    }

    #[allow(dead_code)]
    pub fn wl_surface(&self) -> &WlSurface {
        &self.surface
    }

    pub fn render<F, D>(&mut self, qh: &QueueHandle<D>, render: F)
    where
        F: FnOnce(&mut Buffer, &mut [u8], i32, i32, bool) -> (),
        D: wayland_client::Dispatch<wl_callback::WlCallback, WlSurface> + 'static,
    {
        let mut inner = match self.inner.take() {
            Some(inner) => inner,
            None => {
                // Not configured
                return;
            }
        };

        let (width, height) = (inner.width, inner.height);

        // Render and commit if buffers are available, otherwise do nothing as the
        // other invoker would trigger a next frame
        if let Some((slot_buffer, canvas)) = inner.get_active() {
            let buffer = &mut slot_buffer.buffer;
            render(buffer, canvas, width, height, slot_buffer.resized);
            buffer.attach_to(&self.surface).unwrap();
            self.surface.damage_buffer(0, 0, width, height);
            self.surface.commit();
            self.surface.frame(qh, self.surface.clone());
            slot_buffer.resized = false;
        }
        self.inner = Some(inner);
    }
}
