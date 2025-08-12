use crossterm::{
    cursor,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
    ExecutableCommand, QueueableCommand,
};
use std::io::{stdout, Write};
use anyhow::Result;
use chrono::{DateTime, Local, Datelike, Timelike, NaiveDateTime};

#[derive(Debug, Clone)]
pub struct FilterState {
    pub topic_filter: String,
    pub payload_filter: String,
    pub start_time: String,
    pub end_time: String,
    pub active_field: FilterField,
    pub is_editing: bool,
    pub time_edit_mode: bool,  // 是否處於時間編輯模式
    pub time_edit_position: TimeEditPosition,  // 當前編輯的時間部分
    pub temp_datetime: Option<DateTime<Local>>,  // 暫存的時間值
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeEditPosition {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
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
        use chrono::{Duration, Local};
        let now = Local::now();
        let yesterday = now - Duration::days(1);
        let tomorrow = now + Duration::days(1);
        
        Self {
            topic_filter: String::new(),
            payload_filter: String::new(),
            start_time: yesterday.format("%Y-%m-%d %H:%M:%S").to_string(),
            end_time: tomorrow.format("%Y-%m-%d %H:%M:%S").to_string(),
            active_field: FilterField::Topic,
            is_editing: false,
            time_edit_mode: false,
            time_edit_position: TimeEditPosition::Year,
            temp_datetime: None,
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
    
    // 時間編輯模式相關方法
    pub fn toggle_time_edit_mode(&mut self) {
        if matches!(self.active_field, FilterField::StartTime | FilterField::EndTime) {
            self.time_edit_mode = !self.time_edit_mode;
            
            if self.time_edit_mode {
                // 進入編輯模式時，如果欄位是空的，填入當前時間
                let current_value = match self.active_field {
                    FilterField::StartTime => &self.start_time,
                    FilterField::EndTime => &self.end_time,
                    _ => "",
                };
                
                if current_value.is_empty() {
                    let now = Local::now();
                    self.temp_datetime = Some(now);
                    let formatted = now.format("%Y-%m-%d %H:%M:%S").to_string();
                    
                    match self.active_field {
                        FilterField::StartTime => self.start_time = formatted,
                        FilterField::EndTime => self.end_time = formatted,
                        _ => {}
                    }
                } else {
                    // 解析現有時間
                    if let Ok(naive) = NaiveDateTime::parse_from_str(current_value, "%Y-%m-%d %H:%M:%S") {
                        // 將NaiveDateTime轉換為Local時間
                        use chrono::TimeZone;
                        self.temp_datetime = Some(Local.from_local_datetime(&naive).unwrap());
                    }
                }
                
                self.time_edit_position = TimeEditPosition::Day; // 預設光標放在日期上
            } else {
                // 離開編輯模式
                self.temp_datetime = None;
            }
        }
    }
    
    pub fn enter_time_edit_mode(&mut self) {
        if matches!(self.active_field, FilterField::StartTime | FilterField::EndTime) {
            self.time_edit_mode = true;
            
            // 進入編輯模式時，如果欄位是空的，填入當前時間
            let current_value = match self.active_field {
                FilterField::StartTime => &self.start_time,
                FilterField::EndTime => &self.end_time,
                _ => "",
            };
            
            if current_value.is_empty() {
                let now = Local::now();
                self.temp_datetime = Some(now);
                let formatted = now.format("%Y-%m-%d %H:%M:%S").to_string();
                
                match self.active_field {
                    FilterField::StartTime => self.start_time = formatted,
                    FilterField::EndTime => self.end_time = formatted,
                    _ => {}
                }
            } else {
                // 解析現有時間
                if let Ok(naive) = NaiveDateTime::parse_from_str(current_value, "%Y-%m-%d %H:%M:%S") {
                    // 將NaiveDateTime轉換為Local時間
                    use chrono::TimeZone;
                    self.temp_datetime = Some(Local.from_local_datetime(&naive).unwrap());
                }
            }
            
            self.time_edit_position = TimeEditPosition::Day; // 預設光標放在日期上
        }
    }
    
    pub fn next_time_position(&mut self) {
        if self.time_edit_mode {
            self.time_edit_position = match self.time_edit_position {
                TimeEditPosition::Year => TimeEditPosition::Month,
                TimeEditPosition::Month => TimeEditPosition::Day,
                TimeEditPosition::Day => TimeEditPosition::Hour,
                TimeEditPosition::Hour => TimeEditPosition::Minute,
                TimeEditPosition::Minute => TimeEditPosition::Second,
                TimeEditPosition::Second => TimeEditPosition::Year,
            };
        }
    }
    
    pub fn prev_time_position(&mut self) {
        if self.time_edit_mode {
            self.time_edit_position = match self.time_edit_position {
                TimeEditPosition::Year => TimeEditPosition::Second,
                TimeEditPosition::Month => TimeEditPosition::Year,
                TimeEditPosition::Day => TimeEditPosition::Month,
                TimeEditPosition::Hour => TimeEditPosition::Day,
                TimeEditPosition::Minute => TimeEditPosition::Hour,
                TimeEditPosition::Second => TimeEditPosition::Minute,
            };
        }
    }
    
    pub fn adjust_time_value(&mut self, delta: i32) {
        if !self.time_edit_mode || self.temp_datetime.is_none() {
            return;
        }
        
        if let Some(mut dt) = self.temp_datetime {
            match self.time_edit_position {
                TimeEditPosition::Year => {
                    let new_year = dt.year() + delta;
                    if new_year >= 1970 && new_year <= 9999 {
                        dt = dt.with_year(new_year).unwrap_or(dt);
                    }
                }
                TimeEditPosition::Month => {
                    let new_month = dt.month() as i32 + delta;
                    if new_month >= 1 && new_month <= 12 {
                        dt = dt.with_month(new_month as u32).unwrap_or(dt);
                    }
                }
                TimeEditPosition::Day => {
                    let new_day = dt.day() as i32 + delta;
                    if new_day >= 1 && new_day <= 31 {
                        dt = dt.with_day(new_day as u32).unwrap_or(dt);
                    }
                }
                TimeEditPosition::Hour => {
                    let new_hour = dt.hour() as i32 + delta;
                    if new_hour >= 0 && new_hour <= 23 {
                        dt = dt.with_hour(new_hour as u32).unwrap_or(dt);
                    }
                }
                TimeEditPosition::Minute => {
                    let new_minute = dt.minute() as i32 + delta;
                    if new_minute >= 0 && new_minute <= 59 {
                        dt = dt.with_minute(new_minute as u32).unwrap_or(dt);
                    }
                }
                TimeEditPosition::Second => {
                    let new_second = dt.second() as i32 + delta;
                    if new_second >= 0 && new_second <= 59 {
                        dt = dt.with_second(new_second as u32).unwrap_or(dt);
                    }
                }
            }
            
            self.temp_datetime = Some(dt);
            let formatted = dt.format("%Y-%m-%d %H:%M:%S").to_string();
            
            match self.active_field {
                FilterField::StartTime => self.start_time = formatted,
                FilterField::EndTime => self.end_time = formatted,
                _ => {}
            }
        }
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
            let terminal_width: usize = 100; // Use wider width for larger fields
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
            stdout.queue(Print("]"))?;
            let padding = terminal_width.saturating_sub(38); // 15 + 19 + 4 = 38 chars for topic filter line
            stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            stdout.queue(Print("│"))?;
        
            // Payload filter line
            stdout.queue(cursor::MoveTo(0, row + 3))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print("│ Payload Filter: "))?;
            Self::render_field(&mut stdout, &state.payload_filter, state.active_field == FilterField::Payload && state.is_editing)?;
            stdout.queue(Print("]"))?;
            let padding = terminal_width.saturating_sub(40); // 17 + 19 + 4 = 40 chars for payload filter line
            stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            stdout.queue(Print("│"))?;
        
            // Time filter line
            stdout.queue(cursor::MoveTo(0, row + 4))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print("│ Time: From "))?;
            
            // 渲染Start Time，如果在時間編輯模式則特殊顯示
            if state.active_field == FilterField::StartTime && state.time_edit_mode {
                Self::render_time_edit_field(&mut stdout, &state.start_time, &state.time_edit_position)?;
            } else {
                Self::render_field(&mut stdout, &state.start_time, state.active_field == FilterField::StartTime && state.is_editing)?;
            }
            
            stdout.queue(Print(" To "))?;
            
            // 渲染End Time，如果在時間編輯模式則特殊顯示
            if state.active_field == FilterField::EndTime && state.time_edit_mode {
                Self::render_time_edit_field(&mut stdout, &state.end_time, &state.time_edit_position)?;
            } else {
                Self::render_field(&mut stdout, &state.end_time, state.active_field == FilterField::EndTime && state.is_editing)?;
            }
            
            stdout.queue(Print("]"))?;
            
            // 如果在時間編輯模式，顯示提示
            if state.time_edit_mode {
                stdout.queue(Print(" "))?;
                stdout.queue(SetForegroundColor(Color::Cyan))?;
                stdout.queue(Print("[←→:切換 ↑↓:±1 PgUp/Dn:±10]"))?;
                stdout.queue(ResetColor)?;
                let padding = terminal_width.saturating_sub(75);
            } else {
                let padding = terminal_width.saturating_sub(70); // Time line is much longer now
            }
            stdout.queue(Print(&format!("{:<width$}", "", width = terminal_width.saturating_sub(70))))?;
            stdout.queue(Print("│"))?;
        }
        
        stdout.flush()?;
        Ok(())
    }
    
    fn render_time_edit_field<W: Write>(writer: &mut W, value: &str, position: &TimeEditPosition) -> Result<()> {
        writer.queue(Print("["))?;
        
        if value.len() >= 19 {  // "YYYY-MM-DD HH:MM:SS"
            // 根據當前編輯位置高亮顯示不同部分
            let parts = [
                (&value[0..4], TimeEditPosition::Year),     // YYYY
                (&value[5..7], TimeEditPosition::Month),    // MM
                (&value[8..10], TimeEditPosition::Day),     // DD
                (&value[11..13], TimeEditPosition::Hour),   // HH
                (&value[14..16], TimeEditPosition::Minute), // MM
                (&value[17..19], TimeEditPosition::Second), // SS
            ];
            
            for (i, (part, part_position)) in parts.iter().enumerate() {
                if part_position == position {
                    // 高亮當前編輯的部分
                    writer.queue(SetForegroundColor(Color::Green))?;
                    writer.queue(Print(part))?;
                    writer.queue(ResetColor)?;
                } else {
                    writer.queue(Print(part))?;
                }
                
                // 添加分隔符
                if i < 2 {
                    writer.queue(Print("-"))?;
                } else if i == 2 {
                    writer.queue(Print(" "))?;
                } else if i < 5 {
                    writer.queue(Print(":"))?;
                }
            }
        } else {
            writer.queue(Print(value))?;
        }
        
        writer.queue(ResetColor)?;
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
                writer.queue(Print("___________________"))?; // 19 characters for time format
            }
        } else {
            let display_value = if value.len() > 19 {
                format!("{}...", &value[..16])
            } else {
                format!("{:19}", value) // 19 characters for time format
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
            FilterField::StartTime => (row + 4, 13), // "│ Time: From " = 12 chars + "[" = 13
            FilterField::EndTime => (row + 4, 13 + 19 + 6), // After "From [19 chars] To [" = 13 + 19 + 6 = 38
        };
        
        // 在時間編輯模式下，光標應該定位到當前編輯位置
        if state.time_edit_mode && matches!(state.active_field, FilterField::StartTime | FilterField::EndTime) {
            let cursor_offset = match state.time_edit_position {
                TimeEditPosition::Year => 2,    // 位於年份的中間位置 (20|25)
                TimeEditPosition::Month => 6,   // 位於月份的中間位置 (2025-0|8)
                TimeEditPosition::Day => 9,     // 位於日期的中間位置 (2025-08-1|2)
                TimeEditPosition::Hour => 12,   // 位於小時的中間位置 (2025-08-12 0|0)
                TimeEditPosition::Minute => 15, // 位於分鐘的中間位置 (2025-08-12 00:3|0)
                TimeEditPosition::Second => 18, // 位於秒鐘的中間位置 (2025-08-12 00:30:0|0)
            };
            Some((field_col + cursor_offset, field_row))
        } else {
            let cursor_offset = state.get_active_field_value().len().min(19) as u16;
            Some((field_col + cursor_offset, field_row))
        }
    }
}