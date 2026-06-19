// Copyright 2026 Ahmed Wafdy <ahmedadelwafdy782@gmail.com>
//
// This file is part of xdg-desktop-portal-agl.
//
// xdg-desktop-portal-agl is free software: you can redistribute it and/or
// modify it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 2 of the License, or
// (at your option) any later version.
//
// xdg-desktop-portal-agl is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General
// Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// xdg-desktop-portal-agl. If not, see <https://www.gnu.org/licenses/>.

use std::collections::HashMap;
use std::sync::RwLock;
use zbus::{
    interface,
    object_server::SignalEmitter,
    zvariant::{OwnedValue, Value},
};

/*
This struct represents the appearance configs for the system settings
it can be updated depending on the toolkit being used.
 - color-scheme :Indicates the system's preferred color scheme.
 - accent-color : Indicates the system's preferred accent color as a tuple of RGB values
  in the sRGB color space
 - contrast : Indicates the system's preferred contrast level.
  Supported values are
 - reduced-motion : Indicates the system's preferred reduced motion setting.
*/
#[derive(Debug, Clone, Default)]
pub struct AppearanceConfig {
    pub color_scheme: u32,
    pub contrast: u32,
    pub reduced_motion: u32,
    pub accent_color: Option<(f64, f64, f64)>,
}

pub struct SettingsPortal {
    config: RwLock<AppearanceConfig>,
}

impl SettingsPortal {
    pub fn new() -> Self {
        Self {
            config: RwLock::new(load_config()),
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.Settings")]
impl SettingsPortal {
    #[zbus(property)]
    async fn version(&self) -> u32 {
        2
    }

    async fn read_all(
        &self,
        namespaces: Vec<String>,
    ) -> zbus::fdo::Result<HashMap<String, HashMap<String, OwnedValue>>> {
        let mut out = HashMap::new();

        if namespace_matches("org.freedesktop.appearance", &namespaces) {
            out.insert(
                "org.freedesktop.appearance".to_string(),
                self.build_appearance_map(),
            );
        }

        Ok(out)
    }

    async fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        if namespace != "org.freedesktop.appearance" {
            return Err(zbus::fdo::Error::Failed(format!(
                "unknown namespace {}",
                namespace
            )));
        }

        let config = self.config.read().unwrap();

        let value: OwnedValue = match key {
            "color-scheme" => Value::from(config.color_scheme).try_into().unwrap(),
            "accent-color" => {
                let (r, g, b) = config.accent_color.unwrap_or((10.0, 10.0, 10.0));
                Value::from((r, g, b)).try_into().unwrap()
            }
            "contrast" => Value::from(config.contrast).try_into().unwrap(),
            "reduced-motion" => Value::from(config.reduced_motion).try_into().unwrap(),
            _ => {
                return Err(zbus::fdo::Error::Failed(format!(
                    "unknown key {}/{}",
                    namespace, key
                )));
            }
        };

        Ok(value)
    }

    /**
     * This never fires for now, no file watcher.
     */
    #[zbus(signal)]
    pub async fn setting_changed(
        emitter: &SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: OwnedValue,
    ) -> zbus::Result<()>;
}

impl SettingsPortal {
    fn build_appearance_map(&self) -> HashMap<String, OwnedValue> {
        let config = self.config.read().unwrap();
        let mut hash_map = HashMap::new();

        hash_map.insert(
            "color-scheme".into(),
            Value::from(config.color_scheme).try_into().unwrap(),
        );
        let (r, g, b) = config.accent_color.unwrap_or((10.0, 10.0, 10.0));
        hash_map.insert(
            "accent-color".into(),
            Value::from((r, g, b)).try_into().unwrap(),
        );
        hash_map.insert(
            "contrast".into(),
            Value::from(config.contrast).try_into().unwrap(),
        );
        hash_map.insert(
            "reduced-motion".into(),
            Value::from(config.reduced_motion).try_into().unwrap(),
        );

        hash_map
    }
}

fn namespace_matches(namespace: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() || patterns.iter().any(|p| p.is_empty()) {
        return true;
    }

    patterns.iter().any(|p| {
        p == namespace
            || p.strip_suffix(".*")
                .map(|prefix| namespace.starts_with(prefix))
                .unwrap_or(false)
    })
}

/**
 File format simple KEY=VALUE, one per line:
 color-scheme=1
 contrast=0
 reduced-motion=0
 accent-color=0.2,0.5,0.9

 note: other keys can be updated depending on toolkit.
*/
fn load_config() -> AppearanceConfig {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));

    let candidates = [
        format!("{}/agl-portal/settings.conf", config_home),
        "/etc/agl-portal/settings.conf".to_string(),
    ];

    for path in &candidates {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                tracing::info!("settings loaded from {}", path);
                return parse_config(&content);
            }
            Err(_) => continue,
        }
    }

    tracing::info!("no default settings found, using default");
    AppearanceConfig::default()
}

fn parse_config(path: &str) -> AppearanceConfig {
    let mut config = AppearanceConfig::default();

    for line in path.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("#") {
            continue;
        }

        let Some((key, val)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        let val = val.trim();

        match key {
            "color-scheme" => {
                config.color_scheme = val.parse().unwrap_or(0);
            }

            "accent-color" => {
                let rgb: Vec<f64> = val
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if rgb.len() == 3 && rgb.iter().all(|&v| (0.0..=1.0).contains(&v)) {
                    config.accent_color = Some((rgb[0], rgb[1], rgb[2]));
                }
            }

            "contrast" => {
                config.contrast = val.parse().unwrap_or(0);
            }

            "reduced-motion" => {
                config.reduced_motion = val.parse().unwrap_or(0);
            }

            _ => {
                tracing::error!("failed to parse key : {}", key);
            }
        }
    }

    config
}
