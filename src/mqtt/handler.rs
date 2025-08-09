use tokio::sync::mpsc;
use tracing::{info, error, debug};
use anyhow::Result;
use chrono::Utc;

use crate::db::{MessageRepository, Message};

#[derive(Debug, Clone)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: String,
    pub qos: i32,
    pub retain: bool,
}

pub struct MessageHandler {
    repository: MessageRepository,
    message_receiver: mpsc::UnboundedReceiver<MqttMessage>,
}

impl MessageHandler {
    pub fn new(
        repository: MessageRepository,
        message_receiver: mpsc::UnboundedReceiver<MqttMessage>,
    ) -> Self {
        Self {
            repository,
            message_receiver,
        }
    }
    
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting MQTT message handler...");
        
        let mut batch = Vec::new();
        let batch_size = 100;
        let mut batch_timeout = tokio::time::interval(std::time::Duration::from_millis(1000));
        
        loop {
            tokio::select! {
                // Process incoming messages
                message = self.message_receiver.recv() => {
                    match message {
                        Some(mqtt_msg) => {
                            debug!("Received message on topic: {}", mqtt_msg.topic);
                            
                            let db_message = Message::new(
                                mqtt_msg.topic,
                                mqtt_msg.payload,
                                mqtt_msg.qos,
                                mqtt_msg.retain,
                            );
                            
                            batch.push(db_message);
                            
                            // Process batch if it reaches the size limit
                            if batch.len() >= batch_size {
                                if let Err(e) = self.process_batch(&mut batch).await {
                                    error!("Failed to process message batch: {}", e);
                                }
                            }
                        }
                        None => {
                            info!("Message channel closed, shutting down handler");
                            break;
                        }
                    }
                }
                
                // Process batch on timeout (even if not full)
                _ = batch_timeout.tick() => {
                    if !batch.is_empty() {
                        if let Err(e) = self.process_batch(&mut batch).await {
                            error!("Failed to process timed batch: {}", e);
                        }
                    }
                }
            }
        }
        
        // Process any remaining messages in batch before shutting down
        if !batch.is_empty() {
            if let Err(e) = self.process_batch(&mut batch).await {
                error!("Failed to process final batch: {}", e);
            }
        }
        
        info!("MQTT message handler stopped");
        Ok(())
    }
    
    async fn process_batch(&self, batch: &mut Vec<Message>) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        
        debug!("Processing batch of {} messages", batch.len());
        
        for message in batch.drain(..) {
            match self.repository.insert_message(&message).await {
                Ok(id) => {
                    debug!("Successfully inserted message with ID {}: topic={}, payload_len={}", 
                          id, message.topic, message.payload.len());
                }
                Err(e) => {
                    error!("Failed to insert message for topic '{}': {}", message.topic, e);
                    // Continue processing other messages even if one fails
                }
            }
        }
        
        Ok(())
    }
    
    pub async fn cleanup_old_messages(&self, days: u32) -> Result<u64> {
        info!("Starting cleanup of messages older than {} days", days);
        
        match self.repository.cleanup_old_messages(days).await {
            Ok(deleted_count) => {
                if deleted_count > 0 {
                    info!("Successfully deleted {} old messages", deleted_count);
                } else {
                    info!("No old messages to delete");
                }
                Ok(deleted_count)
            }
            Err(e) => {
                error!("Failed to cleanup old messages: {}", e);
                Err(e)
            }
        }
    }
    
    pub async fn get_stats(&self) -> Result<(i64, i64)> {
        let total_messages = self.repository.get_total_message_count().await?;
        let db_size = self.repository.get_database_size().await?;
        
        Ok((total_messages, db_size))
    }
}