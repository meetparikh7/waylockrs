use serde::Deserialize;

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

#[derive(Clone, Debug, Deserialize)]
pub struct ColorSet {
    pub input: Color,
    pub cleared: Color,
    pub caps_lock: Color,
    pub verifying: Color,
    pub wrong: Color,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Clock {
    pub show_seconds: bool,
    pub text_color: Color,
    pub outline_color: Color,
}

#[derive(Clone, Debug, Deserialize)]
pub struct IndicatorColors {
    pub inside: ColorSet,
    pub line: ColorSet,
    pub ring: ColorSet,
    pub text: ColorSet,
}

#[derive(Clone, Debug, Deserialize)]
pub struct IndicatorHighlights {
    pub backspace: Color,
    pub key: Color,
    pub caps_lock_backspace: Color,
    pub caps_lock_key: Color,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Indicator {
    pub colors: IndicatorColors,
    pub highlights: IndicatorHighlights,
    pub radius: f64,
    pub thickness: f64,
    pub show_caps_lock_indicator: bool,
    pub show_caps_lock_text: bool,
    pub show_text: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub background_color: Color,
    pub background_image: Option<String>,
    pub clock: Clock,
    pub indicator: Indicator,
    pub show_clock: bool,
    pub show_indicator: bool,
}

impl Config {
    fn merge_config_with_defaults(config_str: &str) -> toml::Table {
        const DEFAULT_CONFIG_STR: &'static str = include_str!("../defaults.toml");
        let default_config = DEFAULT_CONFIG_STR.parse::<toml::Table>().unwrap();
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

        merge_table(&default_config, &user_config)
    }
    pub fn parse(config_str: &str) -> Self {
        let merged_config = Self::merge_config_with_defaults(config_str);
        let config: Self = Config::deserialize(merged_config).unwrap();
        config
    }
}
