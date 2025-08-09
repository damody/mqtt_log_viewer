use super::models::{Message, TopicStat, FilterCriteria};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rbatis::RBatis;
use rbdc_sqlite::driver::SqliteDriver;
use std::path::Path;
use tracing::{info, warn, error};

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
        let mut sql = "SELECT id, topic, payload, timestamp, qos, retain, created_at FROM messages WHERE topic = ?".to_string();
        let mut args = vec![rbs::to_value(topic)?];
        
        // Add payload filter if specified
        if let Some(payload_regex) = &criteria.payload_regex {
            sql.push_str(" AND payload REGEXP ?");
            args.push(rbs::to_value(payload_regex)?);
        }
        
        // Add time range filters
        if let Some(start_time) = &criteria.start_time {
            sql.push_str(" AND timestamp >= ?");
            args.push(rbs::to_value(start_time)?);
        }
        
        if let Some(end_time) = &criteria.end_time {
            sql.push_str(" AND timestamp <= ?");
            args.push(rbs::to_value(end_time)?);
        }
        
        sql.push_str(" ORDER BY timestamp DESC");
        
        if let Some(limit) = criteria.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        
        if let Some(offset) = criteria.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }
        
        // For now, return empty vector - will implement proper querying later
        Ok(Vec::new())
    }
    
    pub async fn get_topic_stats(&self, criteria: &FilterCriteria) -> Result<Vec<TopicStat>> {
        let mut sql = r#"
            SELECT 
                topic,
                COUNT(*) as message_count,
                MAX(timestamp) as last_message_time,
                MIN(timestamp) as first_message_time,
                (SELECT payload FROM messages m2 WHERE m2.topic = m1.topic ORDER BY timestamp DESC LIMIT 1) as latest_payload
            FROM messages m1
        "#.to_string();
        
        let mut args = vec![];
        let mut where_clauses = vec![];
        
        // Add topic filter if specified
        if let Some(topic_regex) = &criteria.topic_regex {
            where_clauses.push("topic REGEXP ?".to_string());
            args.push(rbs::to_value(topic_regex)?);
        }
        
        // Add payload filter if specified
        if let Some(payload_regex) = &criteria.payload_regex {
            where_clauses.push("payload REGEXP ?".to_string());
            args.push(rbs::to_value(payload_regex)?);
        }
        
        // Add time range filters
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
        
        sql.push_str(" GROUP BY topic ORDER BY last_message_time DESC");
        
        if let Some(limit) = criteria.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        
        tracing::debug!("Executing topic stats query");
        
        // Check if we have any messages at all
        let count_sql = "SELECT COUNT(*) as total FROM messages";
        let total_count: Result<i64, _> = self.rb.query_decode(&count_sql, vec![]).await;
        match total_count {
            Ok(count) => {
                tracing::debug!("Total messages in database: {}", count);
                if count == 0 {
                    // No messages yet, return empty result
                    return Ok(Vec::new());
                }
            }
            Err(e) => {
                tracing::warn!("Failed to count messages: {}", e);
                return Ok(Vec::new());
            }
        }

        // 執行實際的統計查詢
        tracing::debug!("Executing full topic stats query: {}", sql);
        
        let result: Result<Vec<rbs::Value>, _> = self.rb.query_decode(&sql, args).await;
        
        match result {
            Ok(rows) => {
                let mut topic_stats = Vec::new();
                
                for row in rows {
                    if let Ok(row_map) = rbs::from_value::<std::collections::HashMap<String, rbs::Value>>(row) {
                        let topic = row_map.get("topic")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                            
                        let message_count = row_map.get("message_count")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                            
                        let last_time_str = row_map.get("last_message_time")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                            
                        let first_time_str = row_map.get("first_message_time")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                            
                        let latest_payload = row_map.get("latest_payload")
                            .and_then(|v| v.as_str().map(|s| s.to_string()));
                        
                        // 解析時間戳
                        let last_message_time = chrono::DateTime::parse_from_rfc3339(last_time_str)
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(last_time_str, "%Y-%m-%d %H:%M:%S")
                                    .map(|dt| dt.and_utc().into())
                            })
                            .unwrap_or_else(|_| chrono::Utc::now().into());
                        
                        let first_message_time = chrono::DateTime::parse_from_rfc3339(first_time_str)
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(first_time_str, "%Y-%m-%d %H:%M:%S")
                                    .map(|dt| dt.and_utc().into())
                            })
                            .unwrap_or_else(|_| chrono::Utc::now().into());
                        
                        topic_stats.push(TopicStat {
                            topic,
                            message_count,
                            last_message_time: last_message_time.with_timezone(&chrono::Utc),
                            first_message_time: first_message_time.with_timezone(&chrono::Utc),
                            latest_payload,
                        });
                    }
                }
                
                tracing::debug!("Found {} topic stats", topic_stats.len());
                Ok(topic_stats)
            }
            Err(e) => {
                tracing::error!("Failed to execute topic stats query: {}", e);
                Ok(Vec::new())
            }
        }
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