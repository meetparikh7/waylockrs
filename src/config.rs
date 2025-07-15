use std::ffi::OsString;

use lexopt::ValueExt;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundMode {
    Stretch,
    Fill,
    Fit,
    Center,
    Tile,
}

#[derive(Clone, Debug)]
pub struct Color {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> Result<Color, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let u32_val: u32 = Deserialize::deserialize(deserializer)?;
        let bytes: [u8; 4] = u32_val.to_be_bytes();
        Ok(Color {
            red: (bytes[0] as f64 / 256.0),
            green: (bytes[1] as f64 / 256.0),
            blue: (bytes[2] as f64 / 256.0),
            alpha: (bytes[3] as f64 / 256.0),
        })
    }
}

impl Serialize for Color {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let bytes: [u8; 4] = [
            (self.red * 256.0).round().clamp(0.0, 255.0) as u8,
            (self.green * 256.0).round().clamp(0.0, 255.0) as u8,
            (self.blue * 256.0).round().clamp(0.0, 255.0) as u8,
            (self.alpha * 256.0).round().clamp(0.0, 255.0) as u8,
        ];
        let u32_val: u32 = u32::from_be_bytes(bytes);
        serializer.serialize_u32(u32_val)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColorSet {
    pub input: Color,
    pub cleared: Color,
    pub caps_lock: Color,
    pub verifying: Color,
    pub wrong: Color,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Clock {
    pub show_seconds: bool,
    pub font_size: f64,
    pub text_color: Color,
    pub outline_color: Color,
    pub outline_width: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IndicatorColors {
    pub inside: ColorSet,
    pub line: ColorSet,
    pub ring: ColorSet,
    pub text: ColorSet,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IndicatorHighlights {
    pub backspace: Color,
    pub key: Color,
    pub caps_lock_backspace: Color,
    pub caps_lock_key: Color,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Indicator {
    pub colors: IndicatorColors,
    pub highlights: IndicatorHighlights,
    pub radius: f64,
    pub thickness: f64,
    pub show_caps_lock_indicator: bool,
    pub show_caps_lock_text: bool,
    pub hide_keyboard_layout: bool,
    pub show_text: bool,
    pub show_even_if_idle: bool,
    pub show_failed_attempts: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub background_color: Color,
    pub background_image: Option<String>,
    pub background_mode: BackgroundMode,
    pub clock: Clock,
    pub indicator: Indicator,
    pub ignore_empty_password: bool,
    pub show_clock: bool,
    pub show_indicator: bool,

    /// Workaround for CLI help as our Config loads the CLI flags
    #[serde(alias = "help")]
    pub show_help: bool,
}

/// Returns all long form arguments with their specified value or "true"
struct ConfigArgsIter {
    parser: lexopt::Parser,
}

impl Iterator for ConfigArgsIter {
    type Item = Result<(String, OsString), lexopt::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let key = match self.parser.next() {
            Ok(Some(arg)) => match arg {
                lexopt::Arg::Long(key) => key.to_string(),
                lexopt::Arg::Short(key) => {
                    // Support '-h' for user-convenience
                    if key == 'h' {
                        String::from("help")
                    } else {
                        return Some(Err(arg.unexpected()));
                    }
                }
                _ => return Some(Err(arg.unexpected())),
            },
            Ok(None) => return None,
            Err(err) => return Some(Err(err)),
        };

        let value = match self.parser.values() {
            Ok(values) => values.collect::<Vec<_>>(),
            Err(lexopt::Error::MissingValue { option: _ }) => Vec::new(),
            Err(e) => return Some(Err(e)),
        };
        let value = match value.len() {
            0 => OsString::from("true"),
            1 => value[0].clone(),
            _ => {
                return Some(Err(lexopt::Error::UnexpectedValue {
                    option: key,
                    value: value[1].clone(),
                }));
            }
        };

        Some(Ok((key, value)))
    }
}

impl Config {
    fn default_toml_overrides(config: &mut toml::Table) {
        // Hard-coded overrides for defaults.toml as:
        // - TOML lacks a None for option types
        // - Users might copy the default.toml and we want the 'help'
        //   CLI workaround to stay internal
        config.remove("background_image");
        config.insert("help".to_string(), toml::Value::Boolean(false));
    }

    fn merge_config_with_defaults(config_str: &str) -> toml::Table {
        const DEFAULT_CONFIG_STR: &'static str = include_str!("../defaults.toml");
        let mut default_config = DEFAULT_CONFIG_STR.parse::<toml::Table>().unwrap();
        let user_config = config_str.parse::<toml::Table>().unwrap();

        fn merge_table(orig: &toml::Table, provided: &toml::Table) -> toml::Table {
            let mut result = toml::Table::new();
            for key in orig.keys() {
                if let Some(toml::Value::Table(orig_table)) = orig.get(key)
                    && let Some(toml::Value::Table(provided_table)) = provided.get(key)
                {
                    let new_table = merge_table(orig_table, provided_table);
                    result.insert(key.clone(), toml::Value::Table(new_table));
                } else if let Some(provided_value) = provided.get(key) {
                    result.insert(key.clone(), provided_value.clone());
                } else {
                    result.insert(key.clone(), orig[key].clone());
                }
            }
            for key in provided.keys() {
                if !result.contains_key(key) {
                    result.insert(key.clone(), provided[key].clone());
                }
            }
            result
        }

        Self::default_toml_overrides(&mut default_config);
        merge_table(&default_config, &user_config)
    }

    pub fn merge_with_args(mut config: toml::Table) -> Result<toml::Table, lexopt::Error> {
        let parser = lexopt::Parser::from_env();
        let args_iter = ConfigArgsIter { parser };

        for arg in args_iter {
            let (key, value) = arg?;
            let key = key.replace("-", "_");
            let key_parts = key.split(".").collect::<Vec<_>>();
            let mut current_config = &mut config;
            for key_part in key_parts[0..key_parts.len() - 1].iter() {
                if !current_config.contains_key(*key_part) {
                    let new_table = toml::Value::Table(toml::Table::new());
                    current_config.insert(key_part.to_string(), new_table);
                    continue;
                } else if let Some(toml::Value::Table(next_config)) =
                    current_config.get_mut(*key_part)
                {
                    current_config = next_config;
                } else {
                    return Err(lexopt::Error::UnexpectedOption(key.to_string()));
                }
            }
            let default_value = &current_config.get(*key_parts.last().unwrap());
            let value = match default_value {
                Some(toml::Value::String(_)) | None => {
                    toml::Value::String(value.parse::<String>()?)
                }
                Some(toml::Value::Integer(_)) => toml::Value::Integer(value.parse::<i64>()?),
                Some(toml::Value::Float(_)) => toml::Value::Float(value.parse::<f64>()?),
                Some(toml::Value::Boolean(_)) => toml::Value::Boolean(value.parse::<bool>()?),
                _ => {
                    return Err(lexopt::Error::UnexpectedValue {
                        option: key.to_string(),
                        value: value.clone(),
                    });
                }
            };
            current_config.insert(key_parts[key_parts.len() - 1].to_string(), value);
        }
        Ok(config)
    }

    pub fn parse(config_str: &str) -> Self {
        let merged_config = Self::merge_config_with_defaults(config_str);
        let merged_with_args = Self::merge_with_args(merged_config).unwrap();
        let config: Self = Config::deserialize(merged_with_args).unwrap();
        config
    }
}
