pub struct ColorSet {
    pub input: u32,
    pub cleared: u32,
    pub caps_lock: u32,
    pub verifying: u32,
    pub wrong: u32,
}

pub const INSIDE: ColorSet = ColorSet {
    input: 0x000000C0,
    cleared: 0xE5A445C0,
    caps_lock: 0x000000C0,
    verifying: 0x0072FFC0,
    wrong: 0xFA0000C0,
};
pub const LINE: ColorSet = ColorSet {
    input: 0x000000FF,
    cleared: 0x000000FF,
    caps_lock: 0x000000FF,
    verifying: 0x000000FF,
    wrong: 0x000000FF,
};
pub const RING: ColorSet = ColorSet {
    input: 0x337D00FF,
    cleared: 0xE5A445FF,
    caps_lock: 0xE5A445FF,
    verifying: 0x3300FFFF,
    wrong: 0x7D3300FF,
};
#[allow(dead_code)]
pub const TEXT: ColorSet = ColorSet {
    input: 0xE5A445FF,
    cleared: 0x000000FF,
    caps_lock: 0xE5A445FF,
    verifying: 0x000000FF,
    wrong: 0x000000FF,
};

pub const BS_HIGHLIGHT: u32 = 0xDB3300FF;
pub const KEY_HIGHLIGHT: u32 = 0x33DB00FF;
pub const CAPS_LOCK_BS_HIGHLIGHT: u32 = 0xDB3300FF;
pub const CAPS_LOCK_KEY_HIGHLIGHT: u32 = 0x33DB00FF;
pub const CLOCK_FILL_COLOR: u32 = 0xFFFFFFFF;
pub const CLOCK_OUTLINE_COLOR: u32 = 0x1A1A1AC0;

pub fn map_to_rgba(color: u32) -> (f64, f64, f64, f64) {
    let bytes: [u8; 4] = color.to_be_bytes();
    (
        (bytes[0] as f64 / 256.0),
        (bytes[1] as f64 / 256.0),
        (bytes[2] as f64 / 256.0),
        (bytes[3] as f64 / 256.0),
    )
}
