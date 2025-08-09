use rumqttc::{AsyncClient, MqttOptions, Event, Packet, QoS};
use tokio::sync::mpsc;
use tracing::{info, warn, error, debug};
use anyhow::Result;
use std::time::Duration;

use crate::config::MqttConfig;
use super::handler::MqttMessage;

// Connection status events that can be sent to UI
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    Connected,
    Disconnected,
    Error(String),
}

#[derive(Clone)]
pub struct MqttClient {
    client: AsyncClient,
    message_sender: mpsc::UnboundedSender<MqttMessage>,
}

impl MqttClient {
    pub fn new(
        config: &MqttConfig,
        message_sender: mpsc::UnboundedSender<MqttMessage>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<rumqttc::Event>)> {
        let mut mqtt_options = MqttOptions::new(&config.client_id, &config.host, config.port);
        
        // Set authentication if provided
        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            mqtt_options.set_credentials(username, password);
        }
        
        // Configure connection options
        mqtt_options.set_keep_alive(Duration::from_secs(30));
        mqtt_options.set_clean_session(true);
        mqtt_options.set_max_packet_size(1024 * 1024, 1024 * 1024); // 1MB
        
        let (client, mut eventloop) = AsyncClient::new(mqtt_options, 100);
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        
        // Spawn eventloop task
        tokio::spawn(async move {
            loop {
                match eventloop.poll().await {
                    Ok(event) => {
                        if let Err(_) = event_sender.send(event.clone()) {
                            error!("Failed to send MQTT event to handler");
                            break;
                        }
                    }
                    Err(e) => {
                        error!("MQTT eventloop error: {}", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
        
        let mqtt_client = Self {
            client,
            message_sender,
        };
        
        Ok((mqtt_client, event_receiver))
    }
    
    pub async fn connect_and_subscribe(&self) -> Result<()> {
        info!("Connecting to MQTT broker and subscribing to all topics...");
        
        // Subscribe to all topics using wildcard
        self.client.subscribe("#", QoS::AtMostOnce).await?;
        
        info!("Successfully subscribed to all topics (#)");
        Ok(())
    }
    
    pub async fn handle_events(&self, mut event_receiver: mpsc::UnboundedReceiver<rumqttc::Event>) {
        info!("Starting MQTT event handler...");
        
        while let Some(event) = event_receiver.recv().await {
            if let Err(e) = self.process_event(event).await {
                error!("Error processing MQTT event: {}", e);
            }
        }
    }
    
    pub async fn handle_events_with_status(
        &self, 
        mut event_receiver: mpsc::UnboundedReceiver<rumqttc::Event>,
        connection_status: std::sync::Arc<std::sync::Mutex<bool>>
    ) {
        info!("Starting MQTT event handler with status monitoring...");
        
        while let Some(event) = event_receiver.recv().await {
            // Update connection status based on event
            match &event {
                Event::Incoming(Packet::ConnAck(connack)) if connack.code == rumqttc::ConnectReturnCode::Success => {
                    if let Ok(mut status) = connection_status.lock() {
                        *status = true;
                        info!("Connection status updated to: connected");
                    }
                }
                Event::Incoming(Packet::Disconnect) | Event::Incoming(Packet::PingResp) => {
                    if matches!(event, Event::Incoming(Packet::Disconnect)) {
                        if let Ok(mut status) = connection_status.lock() {
                            *status = false;
                            info!("Connection status updated to: disconnected");
                        }
                    }
                }
                _ => {}
            }
            
            if let Err(e) = self.process_event(event).await {
                error!("Error processing MQTT event: {}", e);
            }
        }
    }
    
    async fn process_event(&self, event: Event) -> Result<()> {
        match event {
            Event::Incoming(Packet::Connect(_)) => {
                info!("MQTT Connected");
            }
            Event::Incoming(Packet::ConnAck(connack)) => {
                info!("MQTT Connection acknowledged: {:?}", connack.code);
                if connack.code == rumqttc::ConnectReturnCode::Success {
                    // Subscribe after successful connection
                    if let Err(e) = self.connect_and_subscribe().await {
                        error!("Failed to subscribe after connection: {}", e);
                    }
                }
            }
            Event::Incoming(Packet::SubAck(suback)) => {
                info!("Subscription acknowledged for packet ID: {}", suback.pkid);
            }
            Event::Incoming(Packet::Publish(publish)) => {
                debug!("Received message on topic: {}", publish.topic);
                
                let message = MqttMessage {
                    topic: publish.topic.clone(),
                    payload: String::from_utf8_lossy(&publish.payload).to_string(),
                    qos: publish.qos as i32,
                    retain: publish.retain,
                };
                
                if let Err(e) = self.message_sender.send(message) {
                    error!("Failed to send message to handler: {}", e);
                }
            }
            Event::Incoming(Packet::Disconnect) => {
                warn!("MQTT Disconnected");
            }
            Event::Outgoing(packet) => {
                debug!("Outgoing MQTT packet: {:?}", packet);
            }
            _ => {
                debug!("Other MQTT event: {:?}", event);
            }
        }
        
        Ok(())
    }
    
    pub async fn disconnect(&self) -> Result<()> {
        info!("Disconnecting from MQTT broker...");
        self.client.disconnect().await?;
        Ok(())
    }
    
    pub fn get_client(&self) -> &AsyncClient {
        &self.client
    }
}