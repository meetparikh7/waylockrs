use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use log::{error, info};
use serde::Deserialize;

use crate::config::Config;

/// Returns a map of swaylock CLI arguments to rustlock TOML config keys.
pub fn swaylock_to_rustlock_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();

    // Global settings
    map.insert("color", "background_color");
    map.insert("image", "background_image");
    map.insert("scaling", "background_mode");
    map.insert("ignore-empty-password", "ignore_empty_password");
    map.insert("show-failed-attempts", "indicator.show_failed_attempts");
    map.insert("ready-fd", "ready_fd");
    map.insert("daemonize", "daemonize");
    map.insert("no-unlock-indicator", "show_indicator"); // inverted

    // Indicator
    map.insert("indicator-radius", "indicator.radius");
    map.insert("indicator-thickness", "indicator.thickness");
    map.insert("indicator-idle-visible", "indicator.show_even_if_idle");

    map.insert("disable-caps-lock-text", "indicator.show_caps_lock_text"); // inverted
    map.insert("indicator-caps-lock", "indicator.show_caps_lock_indicator");
    map.insert("hide-keyboard-layout", "indicator.hide_keyboard_layout");
    map.insert("show-keyboard-layout", "indicator.hide_keyboard_layout"); // inverted

    // Font fallback
    map.insert("font", "indicator.font");
    map.insert("font-size", "indicator.font_size");

    // Indicator colors - inside
    map.insert("inside-color", "indicator.colors.inside.input");
    map.insert("inside-clear-color", "indicator.colors.inside.cleared");
    map.insert(
        "inside-caps-lock-color",
        "indicator.colors.inside.caps_lock",
    );
    map.insert("inside-ver-color", "indicator.colors.inside.verifying");
    map.insert("inside-wrong-color", "indicator.colors.inside.wrong");

    // Indicator colors - line
    map.insert("line-color", "indicator.colors.line.input");
    map.insert("line-clear-color", "indicator.colors.line.cleared");
    map.insert("line-caps-lock-color", "indicator.colors.line.caps_lock");
    map.insert("line-ver-color", "indicator.colors.line.verifying");
    map.insert("line-wrong-color", "indicator.colors.line.wrong");

    // Indicator colors - ring
    map.insert("ring-color", "indicator.colors.ring.input");
    map.insert("ring-clear-color", "indicator.colors.ring.cleared");
    map.insert("ring-caps-lock-color", "indicator.colors.ring.caps_lock");
    map.insert("ring-ver-color", "indicator.colors.ring.verifying");
    map.insert("ring-wrong-color", "indicator.colors.ring.wrong");

    // Indicator colors - text
    map.insert("text-color", "indicator.colors.text.input");
    map.insert("text-clear-color", "indicator.colors.text.cleared");
    map.insert("text-caps-lock-color", "indicator.colors.text.caps_lock");
    map.insert("text-ver-color", "indicator.colors.text.verifying");
    map.insert("text-wrong-color", "indicator.colors.text.wrong");

    // Highlights
    map.insert("bs-hl-color", "indicator.highlights.backspace");
    map.insert(
        "caps-lock-bs-hl-color",
        "indicator.highlights.caps_lock_backspace",
    );
    map.insert(
        "caps-lock-key-hl-color",
        "indicator.highlights.caps_lock_key",
    );
    map.insert("key-hl-color", "indicator.highlights.key");

    map
}

fn apply_inversion(key: &str, value: bool) -> bool {
    match key {
        "no-unlock-indicator" | "disable-caps-lock-text" => !value,
        "show-keyboard-layout" => false, // because CLI enables it, config disables with `true`
        "hide-keyboard-layout" => true,
        _ => value,
    }
}

fn toml_table_insert_dotted(table: &mut toml::Table, key: &str, value: toml::Value) -> bool {
    let mut current = table;
    let key_parts = key.split(".").collect::<Vec<_>>();
    for key_part in key_parts[0..key_parts.len() - 1].iter() {
        if !current.contains_key(*key_part) {
            let new_table = toml::Value::Table(toml::Table::new());
            current.insert(key_part.to_string(), new_table);
        }
        if let Some(toml::Value::Table(next_config)) = current.get_mut(*key_part) {
            current = next_config;
        } else {
            return false;
        }
    }
    current.insert(key_parts[key_parts.len() - 1].to_string(), value);
    true
}

pub fn parse_swaylock_config(config: &str) -> Option<Config> {
    let mut result = toml::Table::new();
    let lookup_map = swaylock_to_rustlock_map();
    for line in config.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((key, value)) => (key, value),
            None => (line, "true"),
        };
        let key = key.trim_start_matches("--");
        let value = if value == "true" {
            toml::Value::Boolean(true)
        } else if value == "false" {
            toml::Value::Boolean(false)
        } else if key.contains("color") || ["font", "image", "scaling"].contains(&key) {
            toml::Value::String(value.to_string())
        } else {
            if let Ok(value) = f64::from_str(value) {
                toml::Value::Float(value)
            } else {
                error!("Skipping field '{key}' with '{value}'");
                continue;
            }
        };
        let value = if let toml::Value::Boolean(value) = value {
            toml::Value::Boolean(apply_inversion(key, value))
        } else {
            value
        };
        if let Some(mapped_key) = lookup_map.get(key) {
            if !toml_table_insert_dotted(&mut result, mapped_key, value.clone()) {
                error!("Could not insert {key} with {:?}", value);
            }
        } else {
            error!("Could not map {key} with {value}");
        }
    }
    let result = Config::merge_config_with_defaults(result);
    match Config::deserialize(result) {
        Ok(config) => Some(config),
        Err(err) => {
            error!("Failed to auto-convert swaylock config with {err}");
            None
        }
    }
}

pub fn try_mapping_swalock_config(xdg_dirs: &xdg::BaseDirectories, config_path: &Path) -> String {
    if let Some(sconfig_file) = xdg_dirs.get_config_file(Path::new("swaylock/config"))
        && let Ok(sconfig) = std::fs::read_to_string(sconfig_file)
        && let Some(mapped_config) = parse_swaylock_config(&sconfig)
        && let Ok(config_file) = xdg_dirs.place_config_file(config_path)
    {
        use std::io::Write;
        let exclusive_config = Config::exclusive_config(mapped_config);
        let serialized = toml::to_string_pretty(&exclusive_config).expect("Failed to serialize");
        let mut file = std::fs::File::create(config_file).expect("Failed to create");
        file.write_all(serialized.as_bytes())
            .expect("Failed to write");
        serialized
    } else {
        info!(
            "Config file '$XDG_CONFIG_HOME/{:?}' does not exist. Using defaults",
            config_path
        );
        "".to_string()
    }
}
