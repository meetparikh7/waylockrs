use std::time::Instant;

use crate::CairoExtras;
use crate::config;
use crate::keyboard_state::KeyboardState;

/// Indicator state: status of authentication attempt
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum AuthState {
    /// nothing happening
    Idle,
    /// currently validating password
    Validating,
    /// displaying message: password was wrong
    Invalid,
}

/// Indicator state: status of password buffer / typing letters
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum InputState {
    /// nothing happening; other states decay to this after time
    Idle,
    /// displaying message: password buffer was cleared
    Clear,
    /// pressed a key that input a letter
    Letter,
    /// pressed backspace and removed a letter
    Backspace,
    /// pressed a key (like Ctrl) that did nothing
    Neutral,
}

pub struct AttemptsCounter {
    value: u32,
    value_str: String,
}

impl AttemptsCounter {
    pub fn new() -> Self {
        Self {
            value: 0,
            value_str: "".to_string(),
        }
    }

    pub fn value(&self) -> u32 {
        self.value
    }

    pub fn inc(&mut self) {
        if self.value < 1000 {
            self.value += 1;
            self.value_str = if self.value > 999 {
                "999+".to_string()
            } else {
                format!("{}", self.value)
            };
        }
    }

    pub fn format(&self) -> &str {
        &self.value_str
    }
}

pub struct Indicator {
    pub config: config::Indicator,
    pub input_state: InputState,
    pub auth_state: AuthState,
    pub is_caps_lock: bool,
    pub last_update: Instant,
    pub highlight_start: u32,
    pub failed_attempts: AttemptsCounter,
}

fn configure_font_drawing(context: &cairo::Context, font: &str, font_size: f64) {
    let mut font_options = context.font_options().unwrap();
    font_options.set_hint_style(cairo::HintStyle::Full);
    context.set_font_options(&font_options);
    context.select_font_face(font, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    context.set_font_size(font_size);
}

impl Indicator {
    fn set_color_for_state(&self, context: &cairo::Context, colorset: &config::ColorSet) {
        if self.input_state == InputState::Clear {
            context.set_source_color(&colorset.cleared)
        } else if self.auth_state == AuthState::Validating {
            context.set_source_color(&colorset.verifying)
        } else if self.auth_state == AuthState::Invalid {
            context.set_source_color(&colorset.wrong)
        } else {
            if self.is_caps_lock && self.config.show_caps_lock_indicator {
                context.set_source_color(&colorset.caps_lock)
            } else {
                context.set_source_color(&colorset.input)
            }
        };
    }

    fn text_for_state(&self) -> Option<&str> {
        if self.input_state == InputState::Clear {
            Some("Cleared")
        } else if self.auth_state == AuthState::Validating {
            Some("Verifying")
        } else if self.auth_state == AuthState::Invalid {
            Some("Wrong")
        } else if self.is_caps_lock && self.config.show_caps_lock_text {
            Some("Caps Lock")
        } else {
            if self.config.show_failed_attempts && self.failed_attempts.value() > 0 {
                Some(self.failed_attempts.format())
            } else {
                None
            }
        }
    }

    pub fn draw(
        &mut self,
        context: &cairo::Context,
        width: i32,
        height: i32,
        scale: f64,
        keyboard: &KeyboardState,
    ) {
        if !self.config.show_even_if_idle
            && self.auth_state == AuthState::Idle
            && self.input_state == InputState::Idle
        {
            return;
        }

        self.is_caps_lock = keyboard.is_caps_lock;

        let show_layout = if !self.config.hide_keyboard_layout && keyboard.get_num_layouts() > 1 {
            true
        } else {
            false
        };

        const PI: f64 = std::f64::consts::PI;
        const TYPE_INDICATOR_RANGE: f64 = PI / 3.0;

        let arc_thickness = self.config.thickness * scale;
        let arc_radius = self.config.radius * scale;
        let xc = (width as f64) * scale / 2.0;
        let yc = (height as f64) * scale * 0.5 + arc_radius * 3.0;

        if self.config.font_size <= 0.0 {
            self.config.font_size = arc_radius / 3.0;
        }

        // fill inner circle
        context.set_line_width(0.0);
        context.arc(xc, yc, arc_radius, 0.0, 2.0 * PI);
        self.set_color_for_state(&context, &self.config.colors.inside);
        context.fill_preserve().unwrap();
        context.stroke().unwrap();

        // Draw ring
        context.set_line_width(arc_thickness);
        context.arc(xc, yc, arc_radius, 0.0, 2.0 * PI);
        self.set_color_for_state(&context, &self.config.colors.ring);
        context.stroke().unwrap();

        if self.config.show_text
            && let Some(text) = self.text_for_state()
        {
            configure_font_drawing(context, &self.config.font, self.config.font_size);
            self.set_color_for_state(context, &self.config.colors.text);
            let extents = context.text_extents(text).unwrap();
            let font_extents = context.font_extents().unwrap();
            let x = extents.width() / 2.0 + extents.x_bearing();
            let y = font_extents.height() / 2.0 - font_extents.descent();
            context.move_to(xc - x, yc + y);
            context.show_text(text).unwrap();
            context.close_path();
            context.new_sub_path();
        }

        if show_layout {
            configure_font_drawing(context, &self.config.font, self.config.font_size);
            let text = keyboard.get_active_layout();
            let extents = context.text_extents(text).unwrap();
            let font_extents = context.font_extents().unwrap();
            let box_padding = font_extents.height() * 0.2 * scale;
            let yc = yc + arc_radius + arc_thickness + box_padding;
            let (x_off, y_off) = (extents.x_advance() / 2.0, font_extents.height() / 2.0);
            self.set_color_for_state(context, &self.config.colors.inside);
            context.rectangle(
                xc - x_off - box_padding,
                yc,
                x_off * 2.0 + box_padding * 2.0,
                font_extents.height() + font_extents.descent(),
            );
            context.fill_preserve().unwrap();
            context.set_line_width(2.0 * scale);
            self.set_color_for_state(&context, &self.config.colors.line);
            context.stroke().unwrap();
            self.set_color_for_state(context, &self.config.colors.text);
            context.move_to(xc - x_off, yc + box_padding * 2.0 + y_off);
            context.show_text(text).unwrap();
            context.close_path();
            context.new_sub_path();
        }

        if self.input_state == InputState::Letter || self.input_state == InputState::Backspace {
            let highlight_start = self.highlight_start as f64 * (PI / 1024.0);
            let highlight_end = highlight_start + TYPE_INDICATOR_RANGE;
            context.arc(xc, yc, arc_radius, highlight_start, highlight_end);
            let highlight = if self.input_state == InputState::Letter {
                if self.is_caps_lock && self.config.show_caps_lock_indicator {
                    &self.config.highlights.caps_lock_key
                } else {
                    &self.config.highlights.key
                }
            } else {
                if self.is_caps_lock && self.config.show_caps_lock_indicator {
                    &self.config.highlights.caps_lock_backspace
                } else {
                    &self.config.highlights.backspace
                }
            };
            context.set_source_color(highlight);
            context.stroke().unwrap();
        }

        // Draw inner + outer border of the circle
        self.set_color_for_state(&context, &self.config.colors.line);
        context.set_line_width(2.0 * scale);
        context.arc(xc, yc, arc_radius - arc_thickness / 2.0, 0.0, 2.0 * PI);
        context.stroke().unwrap();
        context.arc(xc, yc, arc_radius + arc_thickness / 2.0, 0.0, 2.0 * PI);
        context.stroke().unwrap();
    }
}

pub struct Clock {
    pub config: config::Clock,
}

impl Clock {
    pub fn draw(&self, context: &cairo::Context, width: i32, height: i32, scale: f64) {
        use time::OffsetDateTime;
        use time::format_description;

        let xc = (width as f64) * scale / 2.0;
        let yc = (height as f64) * scale / 2.0;

        let format = if self.config.show_seconds {
            format_description::parse_borrowed::<2>("[hour]:[minute]:[second]")
        } else {
            format_description::parse_borrowed::<2>("[hour]:[minute]")
        }
        .unwrap();
        let text = match OffsetDateTime::now_local() {
            Ok(dt) => dt.format(&format).unwrap(),
            _ => "Unknown time".to_string(),
        };

        configure_font_drawing(context, &self.config.font, self.config.font_size);

        let extents = context.text_extents(&text).unwrap();
        let font_extents = context.font_extents().unwrap();
        let x = extents.x_advance() / 2.0;
        let y = font_extents.height() / 2.0 - font_extents.descent();
        context.move_to(xc - x, yc + y);
        context.text_path(&text);

        context.set_source_color(&self.config.text_color);
        context.fill_preserve().unwrap();

        context.set_source_color(&self.config.outline_color);
        context.set_line_width(self.config.outline_width);
        context.stroke().unwrap();

        context.close_path();
        context.new_sub_path();
    }
}
