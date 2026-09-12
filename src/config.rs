//! `~/.config/lily58-assistant/config.toml`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::hostlayout::HostLayout;
use crate::layers::TriLayer;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// OS keyboard layout used to show characters: "gb" or "us".
    pub host_layout: HostLayout,
    /// [lower, upper, adjust] for tri-layer emulation; [] turns it off.
    pub tri_layer: Vec<u8>,
}

impl Default for Config {
    fn default() -> Self {
        Self { host_layout: HostLayout::Gb, tri_layer: vec![1, 2, 3] }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        if !(config.tri_layer.is_empty() || config.tri_layer.len() == 3) {
            return Err("tri_layer must be [] or [lower, upper, adjust]".into());
        }
        Ok(config)
    }

    /// A missing file means defaults; an unreadable or invalid one is an error.
    pub fn load_from(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Tri-layer layers and whether to emulate it for plain MO keys.
    pub fn tri(&self) -> (TriLayer, bool) {
        match self.tri_layer[..] {
            [lower, upper, adjust] => (TriLayer { lower, upper, adjust }, true),
            _ => (TriLayer::default(), false),
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("lily58-assistant/config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.host_layout, HostLayout::Gb);
        assert_eq!(c.tri(), (TriLayer::default(), true));
        assert_eq!(Config::parse("").unwrap(), c);
    }

    #[test]
    fn parses_values() {
        let c = Config::parse("host_layout = \"us\"\ntri_layer = [2, 3, 4]\n").unwrap();
        assert_eq!(c.host_layout, HostLayout::Us);
        assert_eq!(c.tri(), (TriLayer { lower: 2, upper: 3, adjust: 4 }, true));
        assert!(!Config::parse("tri_layer = []").unwrap().tri().1);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::parse("tri_layer = [1, 2]").is_err());
        assert!(Config::parse("host_layout = \"fr\"").is_err());
        assert!(Config::parse("colour = \"red\"").is_err());
    }

    #[test]
    fn missing_file_means_defaults() {
        assert_eq!(Config::load_from(Path::new("/nonexistent/lily58-assistant.toml")).unwrap(), Config::default());
    }
}
