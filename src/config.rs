use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub database: DatabaseConfig,
    pub ui: UiConfig,
    pub performance: PerformanceConfig,
    pub quick_filters: QuickFiltersConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub client_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub path: String,
    pub max_messages: u64,
    pub auto_cleanup: bool,
    pub cleanup_days: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UiConfig {
    pub refresh_interval_ms: u64,
    pub max_payload_preview: usize,
    pub theme: String,
    pub enable_json_highlight: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PerformanceConfig {
    pub max_memory_mb: u64,
    pub cache_size: usize,
    pub batch_size: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuickFiltersConfig {
    pub enabled: bool,
    pub filters: Vec<QuickFilter>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuickFilter {
    pub name: String,
    pub pattern: String,
    pub color: String,
    pub hotkey: String,
    pub enabled: bool,
    pub case_sensitive: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mqtt: MqttConfig {
                host: "127.0.0.1".to_string(),
                port: 1883,
                username: None,
                password: None,
                client_id: "mqtt_log_viewer".to_string(),
            },
            database: DatabaseConfig {
                path: "./mqtt_logs.db".to_string(),
                max_messages: 100_000,
                auto_cleanup: true,
                cleanup_days: 30,
            },
            ui: UiConfig {
                refresh_interval_ms: 250,
                max_payload_preview: 50,
                theme: "dark".to_string(),
                enable_json_highlight: true,
            },
            performance: PerformanceConfig {
                max_memory_mb: 100,
                cache_size: 1000,
                batch_size: 100,
            },
            quick_filters: QuickFiltersConfig {
                enabled: true,
                filters: vec![
                    QuickFilter {
                        name: "INFO".to_string(),
                        pattern: "INFO".to_string(),
                        color: "light_green".to_string(),
                        hotkey: "F1".to_string(),
                        enabled: true,
                        case_sensitive: false,
                    },
                    QuickFilter {
                        name: "WARN".to_string(),
                        pattern: "WARN".to_string(),
                        color: "yellow".to_string(),
                        hotkey: "F2".to_string(),
                        enabled: true,
                        case_sensitive: false,
                    },
                    QuickFilter {
                        name: "ERROR".to_string(),
                        pattern: "ERROR".to_string(),
                        color: "red".to_string(),
                        hotkey: "F3".to_string(),
                        enabled: true,
                        case_sensitive: false,
                    },
                    QuickFilter {
                        name: "TRACE".to_string(),
                        pattern: "TRACE".to_string(),
                        color: "dark_grey".to_string(),
                        hotkey: "F4".to_string(),
                        enabled: true,
                        case_sensitive: false,
                    },
                    QuickFilter {
                        name: "DEBUG".to_string(),
                        pattern: "DEBUG".to_string(),
                        color: "cyan".to_string(),
                        hotkey: "F5".to_string(),
                        enabled: true,
                        case_sensitive: false,
                    },
                ],
            },
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = std::env::var("MQTT_LOG_VIEWER_CONFIG")
            .unwrap_or_else(|_| "./config.toml".to_string());
        
        if std::path::Path::new(&config_path).exists() {
            let content = fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            let config = Config::default();
            // Save default config to file
            let toml_content = toml::to_string_pretty(&config)?;
            fs::write(&config_path, toml_content)?;
            Ok(config)
        }
    }
}