-- MQTT Log Viewer Database Schema
-- Initialize the messages table for storing MQTT messages

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    topic TEXT NOT NULL,
    payload TEXT NOT NULL,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    qos INTEGER DEFAULT 0,
    retain BOOLEAN DEFAULT 0,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Create indexes for better query performance
CREATE INDEX IF NOT EXISTS idx_messages_topic ON messages(topic);
CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_topic_timestamp ON messages(topic, timestamp);

-- Create a view for topic statistics
CREATE VIEW IF NOT EXISTS topic_stats AS
SELECT 
    topic,
    COUNT(*) as message_count,
    MAX(timestamp) as last_message_time,
    MIN(timestamp) as first_message_time
FROM messages
GROUP BY topic;