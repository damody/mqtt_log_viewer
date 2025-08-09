mod config;
mod db;
mod mqtt;
mod ui;
mod utils;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{info, error, Level};
use tracing_subscriber;

use config::Config;
use db::MessageRepository;
use mqtt::{MqttClient, MessageHandler, MqttMessage, ConnectionEvent};
use std::sync::{Arc, Mutex};
use ui::App;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging to file - clear existing log on startup
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)  // 清空現有日誌檔案
        .open("mqtt_log_viewer.log")?;
    
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_writer(log_file)
        .with_ansi(false) // 移除顏色控制碼
        .init();

    info!("Starting MQTT Log Viewer");

    // Load configuration
    let config = Config::load()?;
    info!("Configuration loaded from: {:?}", config);

    // Initialize database
    let repository = MessageRepository::new(&config.database.path).await?;
    info!("Database initialized");

    // Create message channel
    let (message_sender, message_receiver) = mpsc::unbounded_channel::<MqttMessage>();

    // Initialize MQTT client with connection status callback
    let (mqtt_client, event_receiver) = MqttClient::new(&config.mqtt, message_sender)?;
    info!("MQTT client initialized");

    // Initialize message handler
    let mut message_handler = MessageHandler::new(repository.clone(), message_receiver);

    // Create shared connection status
    let connection_status = Arc::new(Mutex::new(false));
    
    // Initialize UI application
    let mut app = App::new(config.clone()).await?;
    app.update_connection_status_from_mqtt(false); // Start as disconnected

    // Spawn background tasks
    let mqtt_handle = {
        let client = mqtt_client.clone();
        let status = connection_status.clone();
        tokio::spawn(async move {
            client.handle_events_with_status(event_receiver, status).await;
        })
    };

    let handler_handle = tokio::spawn(async move {
        if let Err(e) = message_handler.start().await {
            error!("Message handler error: {}", e);
        }
    });

    // Don't spawn connection task separately - let the event handler manage it
    // Just trigger the initial connection
    tokio::spawn({
        let client = mqtt_client.clone();
        async move {
            if let Err(e) = client.connect_and_subscribe().await {
                error!("MQTT connection error: {}", e);
            }
        }
    });

    // Run the UI application with connection status monitoring
    let app_result = app.run_with_connection_status(connection_status).await;

    // Cleanup
    info!("Shutting down...");
    
    // Disconnect MQTT client
    if let Err(e) = mqtt_client.disconnect().await {
        error!("Error disconnecting MQTT client: {}", e);
    }

    // Cancel background tasks
    mqtt_handle.abort();
    handler_handle.abort();

    match app_result {
        Ok(_) => {
            info!("Application shutdown completed successfully");
            Ok(())
        }
        Err(e) => {
            error!("Application error: {}", e);
            Err(e)
        }
    }
}
