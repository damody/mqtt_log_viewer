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
            prev.help_text != state.help_text
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
                "[/]filter [Enter]select [↑↓]navigate [←]back [F5]refresh [F1]help".to_string()
            }
            ViewType::MessageList(_) => {
                "[←]back [/]filter [Enter]view [↑↓]navigate [F2]json [F1]help".to_string()
            }
            ViewType::PayloadDetail(_, _) => {
                "[←]back [F2]json-depth [c]opy [↑↓]scroll [PgUp/PgDn]page [F1]help".to_string()
            }
        };
    }
}