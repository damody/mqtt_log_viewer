use crossterm::{
    cursor,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
    ExecutableCommand, QueueableCommand,
};
use std::io::{stdout, Write};
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct StatusBarState {
    pub connection_status: ConnectionStatus,
    pub total_topics: usize,
    pub total_messages: i64,
    pub last_update: Option<chrono::DateTime<chrono::Utc>>,
    pub current_view: ViewType,
    pub help_text: String,
    pub quick_filter_states: Vec<(String, String, bool)>, // (name, color, enabled)
}

#[derive(Debug, Clone)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected(String), // broker address
    Error(String),
}

#[derive(Debug, Clone)]
pub enum ViewType {
    TopicList,
    MessageList(String), // topic name
    PayloadDetail(String, chrono::DateTime<chrono::Utc>), // topic and timestamp
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self {
            connection_status: ConnectionStatus::Disconnected,
            total_topics: 0,
            total_messages: 0,
            last_update: None,
            current_view: ViewType::TopicList,
            help_text: "[/]filter [Enter]select [↑↓]navigate [F5]refresh [F1]help".to_string(),
            quick_filter_states: Vec::new(),
        }
    }
}

pub struct StatusBar;

impl StatusBar {
    pub fn render(state: &StatusBarState, row: u16, _terminal_width: u16) -> Result<()> {
        Self::render_with_comparison(state, None, row)
    }
    
    pub fn render_incremental(
        state: &StatusBarState, 
        prev_state: Option<&StatusBarState>, 
        row: u16, 
        _terminal_width: u16
    ) -> Result<()> {
        Self::render_with_comparison(state, prev_state, row)
    }
    
    fn render_with_comparison(
        state: &StatusBarState, 
        prev_state: Option<&StatusBarState>, 
        row: u16
    ) -> Result<()> {
        let mut stdout = stdout();
        let force_redraw = prev_state.is_none();
        
        // Check if status line changed
        let status_line_changed = prev_state.map_or(true, |prev| {
            // Check connection status
            let connection_changed = match (&prev.connection_status, &state.connection_status) {
                (ConnectionStatus::Disconnected, ConnectionStatus::Disconnected) => false,
                (ConnectionStatus::Connecting, ConnectionStatus::Connecting) => false,
                (ConnectionStatus::Connected(a), ConnectionStatus::Connected(b)) => a != b,
                _ => true,
            };
            
            connection_changed ||
            prev.total_topics != state.total_topics ||
            prev.total_messages != state.total_messages ||
            prev.last_update != state.last_update
        });
        
        let help_line_changed = prev_state.map_or(true, |prev| {
            prev.help_text != state.help_text || prev.quick_filter_states != state.quick_filter_states
        });
        
        // Update connection status in the filter bar (row 1)
        if force_redraw || status_line_changed {
            stdout.queue(cursor::MoveTo(14, 1))?; // Position after "│ Connection: "
            Self::render_connection_status(&mut stdout, &state.connection_status)?;
            
            // Render stats line (Status: X topics | Y messages | Last: timestamp)
            stdout.queue(cursor::MoveTo(0, row))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print("Status: "))?;
            let stats_text = format!("{} topics | {} messages", 
                                   state.total_topics, state.total_messages);
            stdout.queue(Print(&stats_text))?;
            
            if let Some(last_update) = &state.last_update {
                let time_str = last_update.format("%Y-%m-%d %H:%M:%S").to_string();
                stdout.queue(Print(&format!(" | Last: {}", time_str)))?;
            }
        }
        
        // Render help line if changed
        if force_redraw || help_line_changed {
            stdout.queue(cursor::MoveTo(0, row + 1))?;
            stdout.queue(Clear(ClearType::CurrentLine))?;
            stdout.queue(Print(&state.help_text))?;
            
            // 在幫助文字後面顯示快速過濾器狀態
            if !state.quick_filter_states.is_empty() {
                // 獲取終端寬度
                let (terminal_width, _) = crossterm::terminal::size()?;
                
                // 計算所有過濾器狀態文字的總長度
                let mut filter_status_parts = Vec::new();
                for (index, (name, color, enabled)) in state.quick_filter_states.iter().enumerate() {
                    if index < 5 { // 只顯示F1-F5
                        let status_symbol = if *enabled { "✓" } else { "✗" };
                        let status_text = format!("[F{}:{} {}]", index + 1, name, status_symbol);
                        filter_status_parts.push((status_text, color.clone(), *enabled));
                    }
                }
                
                let total_filter_len: usize = filter_status_parts.iter()
                    .map(|(text, _, _)| text.len() + 1) // +1 for space
                    .sum();
                
                if total_filter_len > 0 {
                    // 計算右對齊位置
                    let help_len = state.help_text.len();
                    let available_space = terminal_width.saturating_sub(help_len as u16 + total_filter_len as u16 + 3); // +3 for " | "
                    
                    if available_space > 0 {
                        stdout.queue(Print(&format!("{:<width$}", "", width = available_space as usize)))?;
                        stdout.queue(Print(" | "))?;
                        
                        // 顯示每個過濾器狀態，使用對應顏色
                        for (i, (status_text, color_name, is_enabled)) in filter_status_parts.iter().enumerate() {
                            if i > 0 {
                                stdout.queue(Print(" "))?;
                            }
                            
                            // 根據顏色名稱設定顏色
                            let color = match color_name.as_str() {
                                "Green" => Color::Green,
                                "Yellow" => Color::Yellow,
                                "Red" => Color::Red,
                                "Blue" => Color::Blue,
                                "Cyan" => Color::Cyan,
                                "Magenta" => Color::Magenta,
                                _ => Color::White,
                            };
                            
                            if *is_enabled {
                                stdout.queue(SetForegroundColor(color))?;
                            } else {
                                stdout.queue(SetForegroundColor(Color::DarkGrey))?;
                            }
                            
                            stdout.queue(Print(status_text))?;
                            stdout.queue(ResetColor)?;
                        }
                    }
                }
            }
        }
        
        stdout.flush()?;
        Ok(())
    }
    
    fn render_connection_status<W: Write>(writer: &mut W, status: &ConnectionStatus) -> Result<()> {
        match status {
            ConnectionStatus::Disconnected => {
                writer.queue(SetForegroundColor(Color::Red))?;
                writer.queue(Print("●Disconnected"))?;
                writer.queue(ResetColor)?;
            }
            ConnectionStatus::Connecting => {
                writer.queue(SetForegroundColor(Color::Yellow))?;
                writer.queue(Print("●Connecting..."))?;
                writer.queue(ResetColor)?;
            }
            ConnectionStatus::Connected(addr) => {
                writer.queue(SetForegroundColor(Color::Green))?;
                writer.queue(Print(&format!("●Connected ({})", addr)))?;
                writer.queue(ResetColor)?;
            }
            ConnectionStatus::Error(err) => {
                writer.queue(SetForegroundColor(Color::Red))?;
                writer.queue(Print(&format!("●Error: {}", err)))?;
                writer.queue(ResetColor)?;
            }
        }
        Ok(())
    }
    
    pub fn set_help_text_for_view(state: &mut StatusBarState, view: &ViewType) {
        state.current_view = view.clone();
        
        state.help_text = match view {
            ViewType::TopicList => {
                "[/]filter [Enter]select [↑↓]navigate [Home/End]first/last [←]back [F5]refresh [F1]help".to_string()
            }
            ViewType::MessageList(_) => {
                "[←]back [/]filter [Enter]view [↑↓]navigate [Home/End]first/last [F2]json [F1]help".to_string()
            }
            ViewType::PayloadDetail(_, _) => {
                "[←]back [F2]json-depth [c]opy [↑↓]scroll [PgUp/PgDn]page [F1]help".to_string()
            }
        };
    }
}