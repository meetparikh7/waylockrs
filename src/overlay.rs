use std::time::Instant;

use crate::colors;

// Indicator state: status of authentication attempt
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum AuthState {
    Idle,       // nothing happening
    Validating, // currently validating password
    Invalid,    // displaying message: password was wrong
}

// Indicator state: status of password buffer / typing letters
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum InputState {
    Idle,      // nothing happening; other states decay to this after time
    Clear,     // displaying message: password buffer was cleared
    Letter,    // pressed a key that input a letter
    Backspace, // pressed backspace and removed a letter
    Neutral,   // pressed a key (like Ctrl) that did nothing
}

pub struct Indicator {
    pub radius: f64,
    pub arc_thickness: f64,
    pub input_state: InputState,
    pub auth_state: AuthState,
    pub is_caps_lock: bool,
    pub show_caps_lock_indictor: bool,
    pub show_caps_lock_text: bool,
    pub last_update: Instant,
    pub highlight_start: u32,
}

const PI: f64 = std::f64::consts::PI;
const TYPE_INDICATOR_RANGE: f64 = PI / 3.0;

fn configure_font_drawing(context: &cairo::Context, font_size: f64) {
    let mut font_options = context.font_options().unwrap();
    font_options.set_hint_style(cairo::HintStyle::Full);
    context.set_font_options(&font_options);
    context.select_font_face(
        "sans-serif",
        cairo::FontSlant::Normal,
        cairo::FontWeight::Normal,
    );
    context.set_font_size(font_size);
}

impl Indicator {
    fn set_color_for_state(&self, context: &cairo::Context, colorset: &colors::ColorSet) {
        let (r, g, b, a) = if self.input_state == InputState::Clear {
            colors::map_to_rgba(colorset.cleared)
        } else if self.auth_state == AuthState::Validating {
            colors::map_to_rgba(colorset.verifying)
        } else if self.auth_state == AuthState::Invalid {
            colors::map_to_rgba(colorset.wrong)
        } else {
            if self.is_caps_lock && self.show_caps_lock_indictor {
                colors::map_to_rgba(colorset.caps_lock)
            } else {
                colors::map_to_rgba(colorset.input)
            }
        };
        context.set_source_rgba(r, g, b, a);
    }

    fn text_for_state(&self) -> Option<&'static str> {
        if self.input_state == InputState::Clear {
            Some("Cleared")
        } else if self.auth_state == AuthState::Validating {
            Some("Verifying")
        } else if self.auth_state == AuthState::Invalid {
            Some("Wrong")
        } else if self.is_caps_lock && self.show_caps_lock_text {
            Some("Caps Lock")
        } else {
            None
        }
    }

    pub fn draw(&self, context: &cairo::Context, xc: f64, yc: f64, scale: f64) {
        let arc_thickness = self.arc_thickness * scale;
        let arc_radius = self.radius * scale;

        // fill inner circle
        context.set_line_width(0.0);
        context.arc(xc, yc, arc_radius, 0.0, 2.0 * PI);
        self.set_color_for_state(&context, &colors::INSIDE);
        context.fill_preserve().unwrap();
        context.stroke().unwrap();

        // Draw ring
        context.set_line_width(arc_thickness);
        context.arc(xc, yc, arc_radius, 0.0, 2.0 * PI);
        self.set_color_for_state(&context, &colors::RING);
        context.stroke().unwrap();

        if let Some(text) = self.text_for_state() {
            configure_font_drawing(context, arc_radius / 3.0);
            self.set_color_for_state(context, &colors::TEXT);
            let extents = context.text_extents(text).unwrap();
            let font_extents = context.font_extents().unwrap();
            let x = extents.width() / 2.0 + extents.x_bearing();
            let y = font_extents.height() / 2.0 - font_extents.descent();
            context.move_to(xc - x, yc + y);
            context.show_text(text).unwrap();
            context.close_path();
            context.new_sub_path();
        }

        if self.input_state == InputState::Letter || self.input_state == InputState::Backspace {
            let highlight_start = self.highlight_start as f64 * (PI / 1024.0);
            let highlight_end = highlight_start + TYPE_INDICATOR_RANGE;
            context.arc(xc, yc, arc_radius, highlight_start, highlight_end);
            let highlight = if self.input_state == InputState::Letter {
                if self.is_caps_lock && self.show_caps_lock_indictor {
                    colors::CAPS_LOCK_KEY_HIGHLIGHT
                } else {
                    colors::KEY_HIGHLIGHT
                }
            } else {
                if self.is_caps_lock && self.show_caps_lock_indictor {
                    colors::CAPS_LOCK_BS_HIGHLIGHT
                } else {
                    colors::BS_HIGHLIGHT
                }
            };
            let (r, g, b, a) = colors::map_to_rgba(highlight);
            context.set_source_rgba(r, g, b, a);
            context.stroke().unwrap();
        }

        // Draw inner + outer border of the circle
        self.set_color_for_state(&context, &colors::LINE);
        context.set_line_width(2.0 * scale);
        context.arc(xc, yc, arc_radius - arc_thickness / 2.0, 0.0, 2.0 * PI);
        context.stroke().unwrap();
        context.arc(xc, yc, arc_radius + arc_thickness / 2.0, 0.0, 2.0 * PI);
        context.stroke().unwrap();
    }
}

pub fn draw_clock(context: &cairo::Context, width: i32, height: i32, scale: f64) {
    use time::OffsetDateTime;
    use time::format_description;

    let xc = (width as f64) * scale / 2.0;
    let yc = (height as f64) * scale / 2.0;

    let format = format_description::parse("[hour]:[minute]:[second]").unwrap();
    let text = match OffsetDateTime::now_local() {
        Ok(dt) => dt.format(&format).unwrap(),
        _ => "Unknown time".to_string(),
    };

    configure_font_drawing(context, 70.0);
    let (r, g, b, a) = colors::map_to_rgba(colors::CLOCK_COLOR);
    context.set_source_rgba(r, g, b, a);

    let extents = context.text_extents(&text).unwrap();
    let font_extents = context.font_extents().unwrap();
    let x = extents.width() / 2.0 + extents.x_bearing();
    let y = font_extents.height() / 2.0 - font_extents.descent();
    context.move_to(xc - x, yc + y);
    context.show_text(&text).unwrap();
    context.close_path();
    context.new_sub_path();
}
