use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub mqtt: MqttConfig,
    pub database: DatabaseConfig,
    pub ui: UiConfig,
    pub performance: PerformanceConfig,
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