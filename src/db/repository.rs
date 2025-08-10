use super::models::{Message, TopicStat, FilterCriteria};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rbatis::RBatis;
use rbdc_sqlite::driver::SqliteDriver;
use std::path::Path;
use tracing::{info, warn, error};
use rayon::prelude::*;
use regex::Regex;

#[derive(Clone)]
pub struct MessageRepository {
    rb: RBatis,
}

impl MessageRepository {
    pub async fn new(db_path: &str) -> Result<Self> {
        let rb = RBatis::new();
        
        // Create parent directory if it doesn't exist
        if let Some(parent) = Path::new(db_path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        
        let url = format!("sqlite:{}", db_path);
        rb.link(SqliteDriver {}, &url).await?;
        
        let repo = Self { rb };
        repo.initialize_schema().await?;
        
        info!("Database initialized at: {}", db_path);
        Ok(repo)
    }
    
    async fn initialize_schema(&self) -> Result<()> {
        let schema_sql = include_str!("../../migrations/init.sql");
        
        // Split by semicolon and execute each statement
        for statement in schema_sql.split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                self.rb.exec(statement, vec![]).await?;
            }
        }
        
        info!("Database schema initialized");
        Ok(())
    }
    
    pub async fn insert_message(&self, message: &Message) -> Result<i64> {
        let sql = r#"
            INSERT INTO messages (topic, payload, timestamp, qos, retain) 
            VALUES (?, ?, ?, ?, ?)
        "#;
        
        tracing::debug!("Inserting message: topic={}, payload_len={}, timestamp={}", 
                       message.topic, message.payload.len(), message.timestamp);
        
        let result = self.rb.exec(sql, vec![
            rbs::to_value(&message.topic)?,
            rbs::to_value(&message.payload)?,
            rbs::to_value(&message.timestamp.to_rfc3339())?, // Convert to string
            rbs::to_value(&message.qos)?,
            rbs::to_value(&message.retain)?,
        ]).await?;
        
        let insert_id = result.last_insert_id.as_i64().unwrap_or(0);
        tracing::debug!("Message inserted with ID: {}", insert_id);
        Ok(insert_id)
    }
    
    pub async fn get_messages_by_topic(
        &self,
        topic: &str,
        criteria: &FilterCriteria,
    ) -> Result<Vec<Message>> {
        // First, get all messages for the topic (with time filters only)
        let mut sql = "SELECT id, topic, payload, timestamp, qos, retain, created_at FROM messages WHERE topic = ?".to_string();
        let mut args = vec![rbs::to_value(topic)?];
        
        // Add time range filters (these stay in SQL for efficiency)
        if let Some(start_time) = &criteria.start_time {
            sql.push_str(" AND timestamp >= ?");
            args.push(rbs::to_value(start_time)?);
        }
        
        if let Some(end_time) = &criteria.end_time {
            sql.push_str(" AND timestamp <= ?");
            args.push(rbs::to_value(end_time)?);
        }
        
        sql.push_str(" ORDER BY timestamp DESC");
        
        tracing::debug!("Executing SQL: {} with args: {:?}", sql, args);
        
        let result = self.rb.query(&sql, args).await?;
        let mut messages = Vec::new();
        
        if let rbs::Value::Array(rows) = result {
            for (idx, row_value) in rows.into_iter().enumerate() {
                if let rbs::Value::Map(row) = row_value {
                    // Debug: print all keys in the map
                    if idx == 0 {
                        tracing::debug!("First row keys: {:?}", row.0.keys().collect::<Vec<_>>());
                    }
                    
                    let id_key = rbs::Value::String("id".to_string());
                    let topic_key = rbs::Value::String("topic".to_string());
                    let payload_key = rbs::Value::String("payload".to_string());
                    let timestamp_key = rbs::Value::String("timestamp".to_string());
                    let qos_key = rbs::Value::String("qos".to_string());
                    let retain_key = rbs::Value::String("retain".to_string());
                    
                    // Debug: check what value we get for payload
                    let payload_value = row.get(&payload_key);
                    if idx == 0 {
                        tracing::debug!("First row payload value type: {:?}", payload_value);
                    }
                    
                    let id = row.get(&id_key).as_i64();
                    let topic = row.get(&topic_key).as_str().unwrap_or("").to_string();
                    
                    // Handle payload - it might be a Map (JSON object) or a String
                    let payload = match payload_value {
                        rbs::Value::String(s) => s.clone(),
                        rbs::Value::Map(_) | rbs::Value::Array(_) => {
                            // Convert Map/Array back to JSON string
                            serde_json::to_string(payload_value).unwrap_or_else(|_| "{}".to_string())
                        }
                        _ => payload_value.as_str().unwrap_or("").to_string()
                    };
                    let timestamp_str = row.get(&timestamp_key).as_str().unwrap_or("");
                    let qos = row.get(&qos_key).as_i64().unwrap_or(0) as i32;
                    let retain = row.get(&retain_key).as_bool().unwrap_or(false);
                    
                    // Debug logging
                    if idx < 3 {
                        tracing::debug!("Row {} data - topic: {}, payload: {}, timestamp: {}", idx, topic, payload, timestamp_str);
                    }
                    
                    let timestamp = DateTime::parse_from_rfc3339(timestamp_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());
                    
                    messages.push(Message {
                        id,
                        topic,
                        payload,
                        timestamp,
                        qos,
                        retain,
                        created_at: None,
                    });
                }
            }
        } else {
            tracing::warn!("Query result is not an array: {:?}", result);
        }
        
        tracing::debug!("Retrieved {} messages from database", messages.len());
        
        // Now apply payload regex filter using rayon parallel processing
        if let Some(payload_regex) = &criteria.payload_regex {
            match Regex::new(payload_regex) {
                Ok(regex) => {
                    tracing::info!("Applying regex filter: {}", payload_regex);
                    
                    messages = messages
                        .into_par_iter()
                        .filter(|msg| regex.is_match(&msg.payload))
                        .collect();
                    
                    tracing::info!("After regex filter: {} messages", messages.len());
                }
                Err(e) => {
                    tracing::warn!("Invalid regex pattern '{}': {}", payload_regex, e);
                    // Don't filter if regex is invalid, return all messages
                }
            }
        }
        
        // Apply limit and offset after filtering
        if let Some(offset) = criteria.offset {
            messages = messages.into_iter().skip(offset as usize).collect();
        }
        
        if let Some(limit) = criteria.limit {
            messages.truncate(limit as usize);
        }
        
        tracing::debug!("Final result: {} messages", messages.len());
        Ok(messages)
    }
    
    pub async fn get_topic_stats(&self, criteria: &FilterCriteria) -> Result<Vec<TopicStat>> {
        // For topic stats, we need to get all messages first, then filter and aggregate
        let mut sql = "SELECT topic, payload, timestamp FROM messages".to_string();
        
        let mut args = vec![];
        let mut where_clauses = vec![];
        
        // Add time range filters (these stay in SQL for efficiency)
        if let Some(start_time) = &criteria.start_time {
            where_clauses.push("timestamp >= ?".to_string());
            args.push(rbs::to_value(start_time)?);
        }
        
        if let Some(end_time) = &criteria.end_time {
            where_clauses.push("timestamp <= ?".to_string());
            args.push(rbs::to_value(end_time)?);
        }
        
        if !where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clauses.join(" AND "));
        }
        
        sql.push_str(" ORDER BY timestamp DESC");
        
        tracing::debug!("Executing topic stats query: {}", sql);
        
        // Get all messages
        let result = self.rb.query(&sql, args).await?;
        let mut all_messages = Vec::new();
        
        if let rbs::Value::Array(rows) = result {
            for row_value in rows {
                if let rbs::Value::Map(row) = row_value {
                    let topic_key = rbs::Value::String("topic".to_string());
                    let payload_key = rbs::Value::String("payload".to_string());
                    let timestamp_key = rbs::Value::String("timestamp".to_string());
                    
                    let topic = row.get(&topic_key).as_str().unwrap_or("").to_string();
                    let payload = match row.get(&payload_key) {
                        rbs::Value::String(s) => s.clone(),
                        rbs::Value::Map(_) | rbs::Value::Array(_) => {
                            serde_json::to_string(row.get(&payload_key)).unwrap_or_else(|_| "{}".to_string())
                        }
                        _ => row.get(&payload_key).as_str().unwrap_or("").to_string()
                    };
                    let timestamp_str = row.get(&timestamp_key).as_str().unwrap_or("");
                    let timestamp = DateTime::parse_from_rfc3339(timestamp_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());
                    
                    all_messages.push((topic, payload, timestamp));
                }
            }
        }
        
        tracing::debug!("Retrieved {} messages for stats", all_messages.len());
        
        // Apply regex filters using rayon
        if let Some(topic_regex) = &criteria.topic_regex {
            match Regex::new(topic_regex) {
                Ok(regex) => {
                    all_messages = all_messages
                        .into_par_iter()
                        .filter(|(topic, _, _)| regex.is_match(topic))
                        .collect();
                    tracing::debug!("After topic regex filter: {} messages", all_messages.len());
                }
                Err(e) => {
                    tracing::warn!("Invalid topic regex pattern '{}': {}", topic_regex, e);
                }
            }
        }
        
        if let Some(payload_regex) = &criteria.payload_regex {
            match Regex::new(payload_regex) {
                Ok(regex) => {
                    all_messages = all_messages
                        .into_par_iter()
                        .filter(|(_, payload, _)| regex.is_match(payload))
                        .collect();
                    tracing::debug!("After payload regex filter: {} messages", all_messages.len());
                }
                Err(e) => {
                    tracing::warn!("Invalid payload regex pattern '{}': {}", payload_regex, e);
                }
            }
        }
        
        // Group by topic and calculate stats
        use std::collections::HashMap;
        let mut topic_map: HashMap<String, Vec<(String, DateTime<Utc>)>> = HashMap::new();
        
        for (topic, payload, timestamp) in all_messages {
            topic_map.entry(topic.clone())
                .or_insert_with(Vec::new)
                .push((payload, timestamp));
        }
        
        let mut topic_stats: Vec<TopicStat> = topic_map
            .into_par_iter()
            .map(|(topic, messages)| {
                let message_count = messages.len() as i64;
                let last_message_time = messages.iter().map(|(_, t)| *t).max().unwrap_or_else(Utc::now);
                let first_message_time = messages.iter().map(|(_, t)| *t).min().unwrap_or_else(Utc::now);
                let latest_payload = messages.iter()
                    .max_by_key(|(_, t)| *t)
                    .map(|(p, _)| p.clone());
                
                TopicStat {
                    topic,
                    message_count,
                    last_message_time,
                    first_message_time,
                    latest_payload,
                }
            })
            .collect();
        
        // Sort by last message time descending
        topic_stats.sort_by(|a, b| b.last_message_time.cmp(&a.last_message_time));
        
        // Apply limit if specified
        if let Some(limit) = criteria.limit {
            topic_stats.truncate(limit as usize);
        }
        
        tracing::debug!("Found {} topic stats", topic_stats.len());
        Ok(topic_stats)
    }
    
    pub async fn cleanup_old_messages(&self, days: u32) -> Result<u64> {
        let sql = "DELETE FROM messages WHERE created_at < datetime('now', '-{} days')";
        let sql = sql.replace("{}", &days.to_string());
        
        let result = self.rb.exec(&sql, vec![]).await?;
        let deleted_count = result.rows_affected;
        
        if deleted_count > 0 {
            info!("Cleaned up {} old messages", deleted_count);
        }
        
        Ok(deleted_count)
    }
    
    pub async fn get_total_message_count(&self) -> Result<i64> {
        let sql = "SELECT COUNT(*) as count FROM messages";
        // For now, return 0 - will implement proper counting later
        Ok(0)
    }
    
    pub async fn get_database_size(&self) -> Result<i64> {
        // This is SQLite specific - get page count and page size
        let sql = "PRAGMA page_count";
        // For now, return a default size - will implement proper size calculation later
        Ok(0)
    }
}