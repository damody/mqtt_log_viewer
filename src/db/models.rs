use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Option<i64>,
    pub topic: String,
    pub payload: String,
    pub timestamp: DateTime<Utc>,
    pub qos: i32,
    pub retain: bool,
    pub created_at: Option<DateTime<Utc>>,
}

impl Message {
    pub fn new(topic: String, payload: String, qos: i32, retain: bool) -> Self {
        Self {
            id: None,
            topic,
            payload,
            timestamp: Utc::now(),
            qos,
            retain,
            created_at: Some(Utc::now()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TopicStat {
    pub topic: String,
    pub message_count: i64,
    pub last_message_time: DateTime<Utc>,
    pub first_message_time: DateTime<Utc>,
    pub latest_payload: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FilterCriteria {
    pub topic_regex: Option<String>,
    pub payload_regex: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl Default for FilterCriteria {
    fn default() -> Self {
        Self {
            topic_regex: None,
            payload_regex: None,
            start_time: None,
            end_time: None,
            limit: Some(1000),
            offset: Some(0),
        }
    }
}