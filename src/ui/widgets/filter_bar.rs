use crossterm::{
    cursor,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
    ExecutableCommand, QueueableCommand,
};
use std::io::{stdout, Write};
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct FilterState {
    pub topic_filter: String,
    pub payload_filter: String,
    pub start_time: String,
    pub end_time: String,
    pub active_field: FilterField,
    pub is_editing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterField {
    Topic,
    Payload,
    StartTime,
    EndTime,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            topic_filter: String::new(),
            payload_filter: String::new(),
            start_time: String::new(),
            end_time: String::new(),
            active_field: FilterField::Topic,
            is_editing: false,
        }
    }
}

impl FilterState {
    pub fn get_active_field_value(&self) -> &str {
        match self.active_field {
            FilterField::Topic => &self.topic_filter,
            FilterField::Payload => &self.payload_filter,
            FilterField::StartTime => &self.start_time,
            FilterField::EndTime => &self.end_time,
        }
    }
    
    pub fn get_active_field_value_mut(&mut self) -> &mut String {
        match self.active_field {
            FilterField::Topic => &mut self.topic_filter,
            FilterField::Payload => &mut self.payload_filter,
            FilterField::StartTime => &mut self.start_time,
            FilterField::EndTime => &mut self.end_time,
        }
    }
    
    pub fn next_field(&mut self) {
        self.active_field = match self.active_field {
            FilterField::Topic => FilterField::Payload,
            FilterField::Payload => FilterField::StartTime,
            FilterField::StartTime => FilterField::EndTime,
            FilterField::EndTime => FilterField::Topic,
        };
    }
    
    pub fn previous_field(&mut self) {
        self.active_field = match self.active_field {
            FilterField::Topic => FilterField::EndTime,
            FilterField::Payload => FilterField::Topic,
            FilterField::StartTime => FilterField::Payload,
            FilterField::EndTime => FilterField::StartTime,
        };
    }
    
    pub fn clear_all(&mut self) {
        self.topic_filter.clear();
        self.payload_filter.clear();
        self.start_time.clear();
        self.end_time.clear();
    }
    
    pub fn has_filters(&self) -> bool {
        !self.topic_filter.is_empty() 
            || !self.payload_filter.is_empty()
            || !self.start_time.is_empty()
            || !self.end_time.is_empty()
    }
}

pub struct FilterBar;

impl FilterBar {
    pub fn render(state: &FilterState, row: u16, _terminal_width: u16) -> Result<()> {
        Self::render_with_comparison(state, None, row)
    }
    
    pub fn render_incremental(
        state: &FilterState, 
        prev_state: Option<&FilterState>, 
        row: u16, 
        _terminal_width: u16
    ) -> Result<()> {
        Self::render_with_comparison(state, prev_state, row)
    }
    
    fn render_with_comparison(
        state: &FilterState, 
        prev_state: Option<&FilterState>, 
        row: u16
    ) -> Result<()> {
        let mut stdout = stdout();
        let force_redraw = prev_state.is_none();
        
        // Check what needs to be redrawn
        let topic_changed = prev_state.map_or(true, |prev| 
            prev.topic_filter != state.topic_filter || 
            prev.active_field != state.active_field ||
            prev.is_editing != state.is_editing
        );
        
        let payload_changed = prev_state.map_or(true, |prev| 
            prev.payload_filter != state.payload_filter || 
            prev.active_field != state.active_field ||
            prev.is_editing != state.is_editing
        );
        
        let time_changed = prev_state.map_or(true, |prev| 
            prev.start_time != state.start_time || 
            prev.end_time != state.end_time ||
            prev.active_field != state.active_field ||
            prev.is_editing != state.is_editing
        );
        
        // Always render the complete filter block
        if force_redraw || topic_changed || payload_changed || time_changed {
            let terminal_width: usize = 80; // Use fixed width for consistency
            // Title bar line
            stdout.queue(cursor::MoveTo(0, row))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print("┌─ MQTT Log Viewer "))?;
            let title_padding = terminal_width.saturating_sub(20); // "┌─ MQTT Log Viewer ┐" 
            stdout.queue(Print(&"─".repeat(title_padding)))?;
            stdout.queue(Print("┐"))?;
            
            // Connection status line
            stdout.queue(cursor::MoveTo(0, row + 1))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print("│ Connection: "))?;
            // Status will be filled by StatusBar
            let padding = terminal_width.saturating_sub(15); // "│ Connection: │"
            stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            stdout.queue(Print("│"))?;
            
            // Topic filter line
            stdout.queue(cursor::MoveTo(0, row + 2))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print("│ Topic Filter: "))?;
            Self::render_field(&mut stdout, &state.topic_filter, state.active_field == FilterField::Topic && state.is_editing)?;
            stdout.queue(Print(" [Apply] [Clear]"))?;
            let padding = terminal_width.saturating_sub(48); // Estimate
            stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            stdout.queue(Print("│"))?;
        
            // Payload filter line
            stdout.queue(cursor::MoveTo(0, row + 3))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print("│ Payload Filter: "))?;
            Self::render_field(&mut stdout, &state.payload_filter, state.active_field == FilterField::Payload && state.is_editing)?;
            stdout.queue(Print(" [Apply] [Clear]"))?;
            let padding = terminal_width.saturating_sub(50); // Estimate
            stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            stdout.queue(Print("│"))?;
        
            // Time filter line
            stdout.queue(cursor::MoveTo(0, row + 4))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print("│ Time: From "))?;
            Self::render_field(&mut stdout, &state.start_time, state.active_field == FilterField::StartTime && state.is_editing)?;
            stdout.queue(Print(" To "))?;
            Self::render_field(&mut stdout, &state.end_time, state.active_field == FilterField::EndTime && state.is_editing)?;
            stdout.queue(Print(" [Apply]"))?;
            let padding = terminal_width.saturating_sub(55); // Estimate
            stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            stdout.queue(Print("│"))?;
        }
        
        stdout.flush()?;
        Ok(())
    }
    
    fn render_field<W: Write>(writer: &mut W, value: &str, is_active: bool) -> Result<()> {
        if is_active {
            writer.queue(SetForegroundColor(Color::Yellow))?;
        }
        
        writer.queue(Print("["))?;
        
        // Show field content or placeholder
        if value.is_empty() {
            if is_active {
                writer.queue(Print("_"))?;
            } else {
                writer.queue(Print("___________"))?;
            }
        } else {
            let display_value = if value.len() > 11 {
                format!("{}...", &value[..8])
            } else {
                format!("{:11}", value)
            };
            writer.queue(Print(&display_value))?;
        }
        
        writer.queue(Print("]"))?;
        
        if is_active {
            writer.queue(ResetColor)?;
        }
        
        Ok(())
    }
    
    pub fn get_cursor_position(state: &FilterState, row: u16) -> Option<(u16, u16)> {
        if !state.is_editing {
            return None;
        }
        
        let (field_row, field_col) = match state.active_field {
            FilterField::Topic => (row+2, 17), // "Topic Filter: [" = 17 chars
            FilterField::Payload => (row + 3, 19), // "Payload Filter: [" = 19 chars
            FilterField::StartTime => (row + 4, 14), // "Time: From [" = 11 chars
            FilterField::EndTime => (row + 4, 17 + 14 + state.start_time.len().min(11) as u16), // After "From [xxx] To ["
        };
        
        let cursor_offset = state.get_active_field_value().len().min(11) as u16;
        Some((field_col + cursor_offset, field_row))
    }
}