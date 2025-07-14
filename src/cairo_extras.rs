use crate::config;

pub trait CairoExtras {
    fn set_source_color(&self, color: &config::Color);
}

impl CairoExtras for cairo::Context {
    fn set_source_color(&self, color: &config::Color) {
        self.set_source_rgba(color.red, color.green, color.blue, color.alpha);
    }
}
