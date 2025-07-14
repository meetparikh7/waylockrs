use std::collections::HashMap;

use smithay_client_toolkit::seat::keyboard;
use wayland_client::protocol::wl_keyboard;

pub struct KeyboardState {
    _keyboard: Option<wl_keyboard::WlKeyboard>,
    layouts: HashMap<u32, String>,
    active_layout: u32,
    pub is_caps_lock: bool,
    pub is_control: bool,
}

impl KeyboardState {
    pub fn new(keyboard: Option<wl_keyboard::WlKeyboard>) -> Self {
        Self {
            _keyboard: keyboard,
            layouts: HashMap::new(),
            active_layout: 0,
            is_caps_lock: false,
            is_control: false,
        }
    }

    pub fn parse_keymap_layouts(&mut self, keymap: keyboard::Keymap<'_>) {
        use xkbcommon::xkb;
        let ctx = xkb::Context::new(0);
        let keymap =
            xkb::Keymap::new_from_string(&ctx, keymap.as_string(), xkb::KEYMAP_FORMAT_TEXT_V1, 0)
                .unwrap();
        self.layouts = HashMap::new();
        for (idx, layout) in keymap.layouts().enumerate() {
            self.layouts.insert(idx as u32, layout.to_string());
        }
    }

    pub fn set_active_layout(&mut self, layout: u32) {
        self.active_layout = layout;
    }

    pub fn get_active_layout(&self) -> &str {
        &self.layouts[&self.active_layout]
    }

    pub fn get_num_layouts(&self) -> usize {
        self.layouts.len()
    }
}
