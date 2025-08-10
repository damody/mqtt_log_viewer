use crossterm::{
    cursor,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
    ExecutableCommand, QueueableCommand,
};
use std::io::{stdout, Write};
use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::db::TopicStat;
use crate::utils::json_formatter::JsonFormatter;

#[derive(Debug, Clone)]
pub struct TopicListState {
    pub topics: Vec<TopicStat>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub visible_rows: usize,
}

impl Default for TopicListState {
    fn default() -> Self {
        Self {
            topics: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            visible_rows: 10,
        }
    }
}

impl TopicListState {
    pub fn update_topics(&mut self, topics: Vec<TopicStat>) {
        let was_empty = self.topics.is_empty();
        self.topics = topics;
        
        // Reset selection if topics list was empty
        if was_empty && !self.topics.is_empty() {
            self.selected_index = 0;
            self.scroll_offset = 0;
        } else if self.selected_index >= self.topics.len() && !self.topics.is_empty() {
            self.selected_index = self.topics.len() - 1;
        }
        
        self.adjust_scroll();
    }
    
    pub fn move_up(&mut self) {
        tracing::debug!("move_up called - topics.len()={}, selected_index={}", 
                       self.topics.len(), self.selected_index);
        if self.topics.is_empty() {
            tracing::debug!("No topics available to navigate - showing user feedback");
            // Even if no topics, we can still indicate that the key was received
            return;
        }
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.adjust_scroll();
            tracing::debug!("Moved up to index: {}", self.selected_index);
        } else {
            tracing::debug!("Already at top of list");
        }
    }
    
    pub fn move_down(&mut self) {
        tracing::debug!("move_down called - topics.len()={}, selected_index={}", 
                       self.topics.len(), self.selected_index);
        if self.topics.is_empty() {
            tracing::debug!("No topics available to navigate - showing user feedback");
            // Even if no topics, we can still indicate that the key was received
            return;
        }
        if self.selected_index + 1 < self.topics.len() {
            self.selected_index += 1;
            self.adjust_scroll();
            tracing::debug!("Moved down to index: {}", self.selected_index);
        } else {
            tracing::debug!("Already at bottom of list");
        }
    }
    
    pub fn page_up(&mut self) {
        let page_size = self.visible_rows.saturating_sub(1);
        if self.selected_index >= page_size {
            self.selected_index -= page_size;
        } else {
            self.selected_index = 0;
        }
        self.adjust_scroll();
    }
    
    pub fn page_down(&mut self) {
        let page_size = self.visible_rows.saturating_sub(1);
        let max_index = self.topics.len().saturating_sub(1);
        if self.selected_index + page_size < self.topics.len() {
            self.selected_index += page_size;
        } else {
            self.selected_index = max_index;
        }
        self.adjust_scroll();
    }
    
    pub fn move_to_top(&mut self) {
        tracing::debug!("move_to_top called - topics.len()={}", self.topics.len());
        if !self.topics.is_empty() {
            self.selected_index = 0;
            self.adjust_scroll();
            tracing::debug!("Moved to top - selected_index: {}", self.selected_index);
        }
    }
    
    pub fn move_to_bottom(&mut self) {
        tracing::debug!("move_to_bottom called - topics.len()={}", self.topics.len());
        if !self.topics.is_empty() {
            self.selected_index = self.topics.len() - 1;
            self.adjust_scroll();
            tracing::debug!("Moved to bottom - selected_index: {}", self.selected_index);
        }
    }
    
    fn adjust_scroll(&mut self) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index >= self.scroll_offset + self.visible_rows {
            self.scroll_offset = self.selected_index.saturating_sub(self.visible_rows - 1);
        }
    }
    
    pub fn get_selected_topic(&self) -> Option<&TopicStat> {
        self.topics.get(self.selected_index)
    }
    
    pub fn set_visible_rows(&mut self, rows: usize) {
        self.visible_rows = rows;
        self.adjust_scroll();
    }
}

pub struct TopicListView;

impl TopicListView {
    pub fn render(
        state: &TopicListState, 
        start_row: u16, 
        end_row: u16, 
        terminal_width: u16
    ) -> Result<()> {
        Self::render_with_clear(state, start_row, end_row, terminal_width, true)
    }
    
    pub fn render_incremental(
        state: &TopicListState,
        prev_state: Option<&TopicListState>,
        start_row: u16, 
        end_row: u16, 
        terminal_width: u16
    ) -> Result<()> {
        if let Some(prev) = prev_state {
            // Only redraw changed parts
            Self::render_changed_parts(state, prev, start_row, end_row, terminal_width)
        } else {
            // Full redraw if no previous state
            Self::render_with_clear(state, start_row, end_row, terminal_width, true)
        }
    }
    
    fn render_with_clear(
        state: &TopicListState, 
        start_row: u16, 
        end_row: u16, 
        terminal_width: u16,
        clear_lines: bool
    ) -> Result<()> {
        let mut stdout = stdout();
        
        // Calculate available height
        let available_height = (end_row - start_row) as usize;
        
        // Render separator line (between filters and table) 
        stdout.queue(cursor::MoveTo(0, start_row))?;
        if clear_lines {
            stdout.queue(Clear(ClearType::CurrentLine))?;
        }
        let separator = format!("├{:─<width$}┤", "─", width = terminal_width.saturating_sub(2) as usize);
        stdout.queue(Print(&separator))?;
        
        // Render header
        stdout.queue(cursor::MoveTo(0, start_row + 1))?;
        if clear_lines {
            stdout.queue(Clear(ClearType::CurrentLine))?;
        }
        stdout.queue(Print("│ "))?;
        
        let header = format!(
            "{:<12} │ {:<18} │ {:<6} │ {:<25}",
            "Last Message", "Topic", "Count", "Latest Payload"
        );
        let padded_header = format!("{:<width$}", header, width = terminal_width.saturating_sub(3) as usize);
        stdout.queue(Print(&padded_header))?;
        stdout.queue(Print("│"))?;
        
        // Render topic entries
        let list_start_row = start_row + 2;
        let available_list_height = available_height.saturating_sub(3); // Header + bottom border + status
        
        if state.topics.is_empty() {
            // Show "No topics available" message with borders
            for i in 0..available_list_height {
                let row = list_start_row + i as u16;
                stdout.queue(cursor::MoveTo(0, row))?;
                if clear_lines {
                    stdout.queue(Clear(ClearType::CurrentLine))?;
                }
                stdout.queue(Print("│"))?;
                
                if i == available_list_height / 2 {
                    stdout.queue(SetForegroundColor(Color::DarkGrey))?;
                    let message = "No MQTT topics available. Waiting for messages...";
                    let centered = (terminal_width.saturating_sub(message.len() as u16 + 2)) / 2;
                    stdout.queue(Print(&format!("{:>width$}{}", "", message, width = centered as usize)))?;
                    let padding = terminal_width.saturating_sub(message.len() as u16 + centered + 2);
                    stdout.queue(Print(&format!("{:<width$}", "", width = padding as usize)))?;
                    stdout.queue(ResetColor)?;
                } else {
                    let padding = terminal_width.saturating_sub(2);
                    stdout.queue(Print(&format!("{:<width$}", "", width = padding as usize)))?;
                }
                
                stdout.queue(Print("│"))?;
            }
        } else {
            for (display_index, topic_index) in (state.scroll_offset..).enumerate() {
                if display_index >= available_list_height {
                    break;
                }
                
                let row = list_start_row + display_index as u16;
                stdout.queue(cursor::MoveTo(0, row))?;
                if clear_lines {
                    stdout.queue(Clear(ClearType::CurrentLine))?;
                }
                
                // Render row with borders
                if let Some(topic) = state.topics.get(topic_index) {
                    let is_selected = topic_index == state.selected_index;
                    
                    stdout.queue(Print("│"))?;
                    
                    if is_selected {
                        stdout.queue(SetBackgroundColor(Color::Blue))?;
                        stdout.queue(SetForegroundColor(Color::White))?;
                    }
                    
                    Self::render_topic_row_with_border(&mut stdout, topic, terminal_width)?;
                    
                    if is_selected {
                        stdout.queue(ResetColor)?;
                    }
                    
                    stdout.queue(Print("│"))?;
                } else {
                    // Empty row with borders
                    stdout.queue(Print("│"))?;
                    let padding = terminal_width.saturating_sub(2);
                    stdout.queue(Print(&format!("{:<width$}", "", width = padding as usize)))?;
                    stdout.queue(Print("│"))?;
                }
            }
            
            // Fill remaining rows with empty borders
            for i in (state.topics.len() - state.scroll_offset).min(available_list_height)..available_list_height {
                let row = list_start_row + i as u16;
                stdout.queue(cursor::MoveTo(0, row))?;
                if clear_lines {
                    stdout.queue(Clear(ClearType::CurrentLine))?;
                }
                stdout.queue(Print("│"))?;
                let padding = terminal_width.saturating_sub(2);
                stdout.queue(Print(&format!("{:<width$}", "", width = padding as usize)))?;
                stdout.queue(Print("│"))?;
            }
        }
        
        // Render bottom border
        let bottom_row = end_row.saturating_sub(1);
        stdout.queue(cursor::MoveTo(0, bottom_row))?;
        if clear_lines {
            stdout.queue(Clear(ClearType::CurrentLine))?;
        }
        let bottom_border = format!("└{:─<width$}┘", "─", width = terminal_width.saturating_sub(2) as usize);
        stdout.queue(Print(&bottom_border))?;
        
        // Clear remaining lines if requested
        if clear_lines {
            for i in (list_start_row + (state.topics.len() - state.scroll_offset).min(available_list_height) as u16)..end_row {
                stdout.queue(cursor::MoveTo(0, i))?;
                stdout.queue(Clear(ClearType::CurrentLine))?;
            }
        }
        
        stdout.flush()?;
        Ok(())
    }
    
    fn render_changed_parts(
        state: &TopicListState,
        prev_state: &TopicListState,
        start_row: u16, 
        end_row: u16, 
        terminal_width: u16
    ) -> Result<()> {
        // Use full redraw to avoid breaking the table format
        // The incremental rendering was corrupting the borders and headers
        Self::render_with_clear(state, start_row, end_row, terminal_width, true)
    }
    
    fn render_topic_row_with_border<W: Write>(
        writer: &mut W,
        topic: &TopicStat,
        terminal_width: u16
    ) -> Result<()> {
        // Format timestamp
        let time_str = topic.last_message_time.format("%H:%M:%S").to_string();
        
        // Format message count
        let count_str = if topic.message_count > 9999 {
            "9999+".to_string()
        } else {
            topic.message_count.to_string()
        };
        
        // Simplify payload for display according to PRD
        let payload_str = if let Some(payload) = &topic.latest_payload {
            // JSON format: show only keys like {"temperature","unit"}
            if payload.trim().starts_with('{') && payload.trim().ends_with('}') {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) {
                    if let Some(obj) = json.as_object() {
                        let keys: Vec<String> = obj.keys().map(|k| format!("\"{}\"", k)).collect();
                        format!("{{{}}}", keys.join(","))
                    } else {
                        payload.chars().take(25).collect()
                    }
                } else {
                    payload.chars().take(25).collect()
                }
            } else if payload.is_empty() {
                "(empty)".to_string()
            } else {
                // Non-JSON: show first 25 chars
                let trimmed: String = payload.chars().take(25).collect();
                if payload.len() > 25 {
                    format!("{}...", trimmed)
                } else {
                    trimmed
                }
            }
        } else {
            "(no data)".to_string()
        };
        
        // Truncate topic name if too long
        let topic_name = if topic.topic.len() > 18 {
            format!("{}...", &topic.topic[..15])
        } else {
            topic.topic.clone()
        };
        
        let line = format!(
            " {:<12} │ {:<18} │ {:<6} │ {:<25}",
            time_str,
            topic_name,
            count_str,
            payload_str
        );
        
        // Pad line to fit terminal width minus borders
        let max_width = terminal_width.saturating_sub(3) as usize;
        let padded_line = if line.len() > max_width {
            format!("{}", &line[..max_width])
        } else {
            format!("{:<width$}", line, width = max_width)
        };
        
        writer.queue(Print(&padded_line))?;
        
        Ok(())
    }

    fn render_topic_row<W: Write>(
        writer: &mut W,
        topic: &TopicStat,
        terminal_width: u16
    ) -> Result<()> {
        // Format timestamp
        let time_str = topic.last_message_time.format("%H:%M:%S").to_string();
        
        // Format message count
        let count_str = if topic.message_count > 9999 {
            "9999+".to_string()
        } else {
            topic.message_count.to_string()
        };
        
        // Simplify payload for display
        let payload_str = if let Some(payload) = &topic.latest_payload {
            JsonFormatter::simplify_payload(payload, 30)
        } else {
            "(no data)".to_string()
        };
        
        // Truncate topic name if too long
        let topic_name = if topic.topic.len() > 18 {
            format!("{}...", &topic.topic[..15])
        } else {
            topic.topic.clone()
        };
        
        let line = format!(
            "{:<12} │ {:<18} │ {:<6} │ {}",
            time_str,
            topic_name,
            count_str,
            payload_str
        );
        
        // Truncate line to fit terminal width
        let max_width = terminal_width.saturating_sub(1) as usize;
        if line.len() > max_width {
            writer.queue(Print(&line[..max_width]))?;
        } else {
            writer.queue(Print(&line))?;
        }
        
        Ok(())
    }
}