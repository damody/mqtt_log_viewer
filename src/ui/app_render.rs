use std::io::{stdout, Write};
use crossterm::{
    terminal::{size, Clear, ClearType},
    cursor::{MoveTo, Hide, Show},
    style::{Print, SetForegroundColor, ResetColor},
    ExecutableCommand, QueueableCommand,
};
use anyhow::Result;
use tracing::{info, error};

use crate::ui::widgets::{FilterBar, StatusBar};
use crate::ui::views::{TopicListView};
use crate::ui::app::{App, AppState};

// Rendering implementation for App
impl App {
    pub fn render(&mut self) -> Result<()> {
        info!("render() called - current state: {:?}", self.get_state());
        // Update terminal size if needed
        let (width, height) = size()?;
        let (current_width, current_height) = self.get_terminal_size();
        if width != current_width || height != current_height {
            self.set_terminal_size(width, height);
            self.update_visible_rows();
            self.set_needs_full_redraw(true);
        }
        
        // Only clear screen if full redraw is needed
        if self.needs_full_redraw() {
            info!("Full redraw needed - clearing screen");
            let mut stdout = stdout();
            stdout.execute(Clear(ClearType::All))?;
            stdout.execute(MoveTo(0, 0))?;
        }
        
        match self.get_state() {
            AppState::TopicList => {
                info!("Rendering TopicList");
                self.render_topic_list_incremental()?;
            },
            AppState::MessageList => {
                info!("Rendering MessageList");
                self.render_message_list()?;
            },
            AppState::PayloadDetail => {
                info!("Rendering PayloadDetail");
                self.render_payload_detail()?;
            },
            _ => {
                panic!("Unhandled state in render");
            }
        }
        self.set_needs_full_redraw(false);
        Ok(())
    }
    
    fn render_topic_list_incremental(&mut self) -> Result<()> {
        let filter_rows = 5;  // Filter takes 5 rows (title + connection + topic + payload + time)
        let status_rows = 2;  // Status takes 2 rows
        let (_, terminal_height) = self.get_terminal_size();
        let available_height = terminal_height.saturating_sub(filter_rows + status_rows + 1);
        
        let force_redraw = self.needs_full_redraw();
        
        // Check if filter state changed
        let filter_changed = self.has_filter_state_changed();
        if force_redraw || filter_changed {
            let (terminal_width, _) = self.get_terminal_size();
            if force_redraw {
                FilterBar::render(self.get_filter_state(), 0, terminal_width)?;
            } else {
                FilterBar::render_incremental(
                    self.get_filter_state(), 
                    self.get_prev_filter_state(), 
                    0, 
                    terminal_width
                )?;
            }
        }
        
        // Check if topic list changed
        let topics_changed = self.has_topic_list_state_changed();
        
        // 如果資料有變化就強制重繪主題列表
        let force_topic_redraw = topics_changed;
        
        if force_redraw || topics_changed || force_topic_redraw {
            let list_start_row = filter_rows;
            let list_end_row = list_start_row + available_height;
            let (terminal_width, _) = self.get_terminal_size();
            
            if force_redraw {
                // Full redraw
                TopicListView::render(
                    self.get_topic_list_state(),
                    list_start_row,
                    list_end_row,
                    terminal_width
                )?;
            } else {
                // Incremental update
                TopicListView::render_incremental(
                    self.get_topic_list_state(),
                    self.get_prev_topic_list_state(),
                    list_start_row,
                    list_end_row,
                    terminal_width
                )?;
            }
        }
        
        // Check if status bar changed
        let status_changed = self.has_status_bar_state_changed();
        
        if force_redraw || status_changed {
            let (terminal_width, terminal_height) = self.get_terminal_size();
            let status_start_row = terminal_height.saturating_sub(status_rows);
            if force_redraw {
                StatusBar::render(self.get_status_bar_state(), status_start_row, terminal_width)?;
            } else {
                StatusBar::render_incremental(
                    self.get_status_bar_state(), 
                    self.get_prev_status_bar_state(), 
                    status_start_row, 
                    terminal_width
                )?;
            }
        }
        
        // Position cursor for filter editing
        if let Some((x, y)) = FilterBar::get_cursor_position(self.get_filter_state(), 0) {
            let mut stdout = stdout();
            stdout.execute(MoveTo(x, y))?;
            stdout.execute(Show)?;
        } else {
            let mut stdout = stdout();
            stdout.execute(Hide)?;
        }
        
        // Update previous states for next comparison
        self.update_prev_states();
        
        Ok(())
    }
    
    fn render_message_list(&mut self) -> Result<()> {
        info!("render_message_list() called - starting MessageList UI rendering");
        let mut stdout = stdout();
        // Always clear screen when entering message list (second layer)
        stdout.execute(Clear(crossterm::terminal::ClearType::All))?;
        stdout.execute(MoveTo(0, 0))?;
        info!("Screen cleared and cursor moved to 0,0");
        
        let (terminal_width, terminal_height) = self.get_terminal_size();
        let terminal_width: usize = terminal_width as usize;
        
        // Render title bar - Topic: xxx
        stdout.queue(MoveTo(0, 0))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        if let Some(selected_topic) = self.get_selected_topic() {
            let title = format!("┌─ Topic: {} ", selected_topic.topic);
            let padding = terminal_width.saturating_sub(title.len() + 1);
            stdout.queue(Print(&title))?;
            stdout.queue(Print(&"─".repeat(padding)))?;
            stdout.queue(Print("┐"))?;
        } else {
            error!("No topic selected");
        }
        
        // Render payload filter line with focus indicator and content
        self.render_message_list_payload_filter(&mut stdout, terminal_width)?;
        
        // Render time filter line with focus indicators and content
        self.render_message_list_time_filter(&mut stdout, terminal_width)?;
        
        // Render separator line
        stdout.queue(MoveTo(0, 3))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let separator = format!("├{:─<width$}┤", "─", width = terminal_width.saturating_sub(2));
        stdout.queue(Print(&separator))?;
        
        // Render header
        stdout.queue(MoveTo(0, 4))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("│ "))?;
        let header = format!("{:<12} │ {:<50}", "Time", "Payload");
        let padded_header = format!("{:<width$}", header, width = terminal_width.saturating_sub(3));
        stdout.queue(Print(&padded_header))?;
        stdout.queue(Print("│"))?;
        
        // Calculate content area
        let content_start_row = 5;
        let status_rows = 2;
        let available_height = terminal_height.saturating_sub(content_start_row + status_rows + 1);
        
        // Render message list content
        self.render_message_list_content(&mut stdout, terminal_width, content_start_row, available_height)?;
        
        // Render bottom border
        let bottom_row = terminal_height.saturating_sub(3);
        stdout.queue(MoveTo(0, bottom_row))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let bottom_border = format!("└{:─<width$}┘", "─", width = terminal_width.saturating_sub(2));
        stdout.queue(Print(&bottom_border))?;
        
        // Render status line
        let status_start_row = terminal_height.saturating_sub(2);
        self.render_message_list_status(&mut stdout, status_start_row)?;
        
        // Render help line
        stdout.queue(MoveTo(0, status_start_row + 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("[←][ESC]back [Tab]focus [Enter]view [↑↓]navigate [PgUp/PgDn]page [F1]help"))?;
        
        // Position cursor for input if editing
        if let Some((col, row)) = self.get_message_list_cursor_position() {
            stdout.queue(MoveTo(col, row))?;
        }
        
        stdout.flush()?;
        info!("render_message_list() completed - MessageList UI should now be visible");
        self.set_needs_full_redraw(false); // Reset the redraw flag after successful render
        Ok(())
    }
    
    fn render_payload_detail(&mut self) -> Result<()> {
        info!("render_payload_detail() called - starting PayloadDetail UI rendering");
        let mut stdout = stdout();
        
        // Always clear screen when entering payload detail (third layer)
        stdout.execute(Clear(crossterm::terminal::ClearType::All))?;
        stdout.execute(MoveTo(0, 0))?;
        info!("Screen cleared and cursor moved to 0,0");
        
        let (terminal_width, terminal_height) = self.get_terminal_size();
        let terminal_width: usize = terminal_width as usize;
        
        // Get selected message (clone to avoid borrowing issues)
        let selected_message = match self.get_selected_message() {
            Some(msg) => msg.clone(),
            None => {
                error!("No message selected for payload detail");
                return Ok(());
            }
        };
        
        // Render title bar - Topic: xxx | Message Detail
        stdout.queue(MoveTo(0, 0))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let title = format!("┌─ Message Detail: {} ", selected_message.topic);
        let padding = terminal_width.saturating_sub(title.len() + 1);
        stdout.queue(Print(&title))?;
        stdout.queue(Print(&"─".repeat(padding)))?;
        stdout.queue(Print("┐"))?;
        
        // Render metadata lines
        self.render_payload_detail_metadata(&mut stdout, terminal_width, &selected_message)?;
        
        // Render separator line
        stdout.queue(MoveTo(0, 3))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let separator = format!("├{:─<width$}┤", "─ Payload ", width = terminal_width.saturating_sub(2));
        stdout.queue(Print(&separator))?;
        
        // Calculate content area
        let content_start_row = 4;
        let status_rows = 2;
        let available_height = terminal_height.saturating_sub(content_start_row + status_rows + 1);
        
        // Render payload content
        let payload_lines = self.format_payload_content(&selected_message.payload);
        self.render_payload_detail_content(&mut stdout, terminal_width, content_start_row, available_height, &payload_lines)?;
        
        // Render bottom border
        let bottom_row = content_start_row + available_height as u16;
        stdout.queue(MoveTo(0, bottom_row))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let bottom_border = format!("└{:─<width$}┘", "", width = terminal_width.saturating_sub(2));
        stdout.queue(Print(&bottom_border))?;
        
        // Render status lines
        let status_start_row = terminal_height.saturating_sub(status_rows);
        self.render_payload_detail_status(&mut stdout, status_start_row, &selected_message, &payload_lines, available_height)?;
        
        // Render help line
        stdout.queue(MoveTo(0, status_start_row + 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("[←][ESC]back [F2]json-depth [c]opy [↑↓]scroll [PgUp/PgDn]page [F1]help"))?;
        
        stdout.flush()?;
        info!("render_payload_detail() completed - PayloadDetail UI should now be visible");
        self.set_needs_full_redraw(false); // Reset the redraw flag after successful render
        Ok(())
    }
    
    pub fn get_payload_detail_page_size(&self) -> usize {
        let content_start_row = 4;
        let status_rows = 2;
        let (_, terminal_height) = self.get_terminal_size();
        let available_height = terminal_height.saturating_sub(content_start_row + status_rows + 1);
        available_height as usize
    }
    
    // Helper methods for rendering message list components
    pub fn render_message_list_payload_filter(&self, stdout: &mut std::io::Stdout, terminal_width: usize) -> Result<()> {
        let message_state = self.get_message_list_state();
        
        stdout.queue(MoveTo(0, 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let payload_filter_text = if !message_state.payload_filter_input.is_empty() {
            &message_state.payload_filter_input
        } else if matches!(message_state.get_focus(), crate::ui::views::message_list::FocusTarget::PayloadFilter) {
            if message_state.is_editing {
                "<<<EDITING>>>"
            } else {
                "<<<FOCUSED>>>"
            }
        } else {
            "___________"
        };
        
        if matches!(message_state.get_focus(), crate::ui::views::message_list::FocusTarget::PayloadFilter) {
            stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
        }
        stdout.queue(Print(&format!("│ Payload Filter: [{}] [Apply] [Clear]", payload_filter_text)))?;
        if matches!(message_state.get_focus(), crate::ui::views::message_list::FocusTarget::PayloadFilter) {
            stdout.queue(ResetColor)?;
        }
        let line_len = 23 + payload_filter_text.len() + 17; // "│ Payload Filter: [" + content + "] [Apply] [Clear]"
        let padding = terminal_width.saturating_sub(line_len + 1);
        stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
        stdout.queue(Print("│"))?;
        Ok(())
    }
    
    pub fn render_message_list_time_filter(&self, stdout: &mut std::io::Stdout, terminal_width: usize) -> Result<()> {
        let message_state = self.get_message_list_state();
        
        stdout.queue(MoveTo(0, 2))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let focus = message_state.get_focus();
        
        let from_text = if !message_state.time_from_input.is_empty() {
            &message_state.time_from_input
        } else if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterFrom) {
            if message_state.is_editing {
                "<<<EDITING>>>"
            } else {
                "<<<FOCUS>>>"
            }
        } else {
            "__________"
        };
        
        let to_text = if !message_state.time_to_input.is_empty() {
            &message_state.time_to_input
        } else if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterTo) {
            if message_state.is_editing {
                "<<<EDITING>>>"
            } else {
                "<<<FOCUS>>>"
            }
        } else {
            "__________"
        };
        
        stdout.queue(Print("│ Time: From ["))?;
        if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterFrom) {
            stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
        }
        stdout.queue(Print(from_text))?;
        if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterFrom) {
            stdout.queue(ResetColor)?;
        }
        stdout.queue(Print("] To ["))?;
        if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterTo) {
            stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
        }
        stdout.queue(Print(to_text))?;
        if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterTo) {
            stdout.queue(ResetColor)?;
        }
        stdout.queue(Print("] [Apply]"))?;
        
        let line_len = 16 + from_text.len() + 6 + to_text.len() + 9;
        let padding = terminal_width.saturating_sub(line_len + 1);
        stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
        stdout.queue(Print("│"))?;
        Ok(())
    }
    
    pub fn render_message_list_content(&self, stdout: &mut std::io::Stdout, terminal_width: usize, 
                                  content_start_row: u16, available_height: u16) -> Result<()> {
        let message_state = self.get_message_list_state();
        let messages = &message_state.messages;
        let selected_index = message_state.selected_index;
        
        for i in 0..available_height {
            let row = content_start_row + i as u16;
            stdout.queue(MoveTo(0, row))?;
            stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
            
            stdout.queue(Print("│ "))?;
            
            if let Some(msg) = messages.get(i as usize) {
                // Highlight selected row with focus indication
                if i as usize == selected_index {
                    let is_message_list_focused = matches!(message_state.get_focus(), 
                        crate::ui::views::message_list::FocusTarget::MessageList);
                    if is_message_list_focused {
                        stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
                        stdout.queue(Print(">>"))?;
                    } else {
                        stdout.queue(SetForegroundColor(crossterm::style::Color::DarkCyan))?;
                        stdout.queue(Print("> "))?;
                    }
                } else {
                    stdout.queue(Print("  "))?;
                }
                
                // Format timestamp (HH:MM:SS)
                let time_str = msg.timestamp.format("%H:%M:%S").to_string();
                stdout.queue(Print(&format!("{:<10}", time_str)))?;
                stdout.queue(Print(" │ "))?;
                
                // Truncate payload if too long
                let max_payload_width = terminal_width.saturating_sub(20);
                let payload_display = if msg.payload.len() > max_payload_width {
                    format!("{}...", &msg.payload[..max_payload_width.saturating_sub(3)])
                } else {
                    msg.payload.clone()
                };
                
                stdout.queue(Print(&format!("{:<width$}", payload_display, width = max_payload_width)))?;
                
                if i as usize == selected_index {
                    stdout.queue(ResetColor)?;
                }
            } else if messages.is_empty() && i == 0 {
                stdout.queue(SetForegroundColor(crossterm::style::Color::DarkGrey))?;
                stdout.queue(Print("  No messages found for this topic"))?;
                stdout.queue(ResetColor)?;
                let padding = terminal_width.saturating_sub(37);
                stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            } else {
                let padding = terminal_width.saturating_sub(3);
                stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            }
            
            stdout.queue(Print("│"))?;
        }
        Ok(())
    }
    
    pub fn render_message_list_status(&self, stdout: &mut std::io::Stdout, status_start_row: u16) -> Result<()> {
        let message_state = self.get_message_list_state();
        
        stdout.queue(MoveTo(0, status_start_row))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("Status: "))?;
        
        let message_count = message_state.messages.len();
        let current_page = message_state.page;
        let total_pages = (message_state.total_count + message_state.per_page - 1) / message_state.per_page;
        
        if let Some(topic) = &message_state.current_topic {
            stdout.queue(Print(format!("Page {}/{} | {} messages | Topic: {}", 
                                     current_page, total_pages.max(1), message_count, topic)))?;
        }
        Ok(())
    }
    
    pub fn render_payload_detail_metadata(&self, stdout: &mut std::io::Stdout, terminal_width: usize, 
                                     selected_message: &crate::db::Message) -> Result<()> {
        // Render metadata line 1 - UTC and Local time on same line
        stdout.queue(MoveTo(0, 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let utc_str = selected_message.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string();
        let local_time = selected_message.timestamp.with_timezone(&chrono::Local);
        let local_str = local_time.format("%Y-%m-%d %H:%M:%S %Z").to_string();
        let time_display = format!("│ UTC: {} | Local: {}", utc_str, local_str);
        stdout.queue(Print(&time_display))?;
        let padding = terminal_width.saturating_sub(time_display.len() + 1);
        stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
        stdout.queue(Print("│"))?;
        
        // Render metadata line 2 - QoS and Retain
        stdout.queue(MoveTo(0, 2))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print(&format!("│ QoS: {} | Retain: {}", selected_message.qos, selected_message.retain)))?;
        let qos_retain_text = format!(" QoS: {} | Retain: {}", selected_message.qos, selected_message.retain);
        let line_len = 1 + qos_retain_text.len(); // "│" + text
        let padding = terminal_width.saturating_sub(line_len + 1);
        stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
        stdout.queue(Print("│"))?;
        Ok(())
    }
    
    pub fn render_payload_detail_content(&mut self, stdout: &mut std::io::Stdout, terminal_width: usize,
                                   content_start_row: u16, available_height: u16, payload_lines: &[String]) -> Result<()> {
        let payload_scroll_offset = self.get_payload_detail_scroll_offset();
        
        // Ensure scroll offset doesn't exceed content
        let max_scroll = payload_lines.len().saturating_sub(available_height as usize);
        if payload_scroll_offset > max_scroll {
            self.set_payload_detail_scroll_offset(max_scroll);
        }
        
        let current_scroll_offset = self.get_payload_detail_scroll_offset();
        
        // Render payload content with scroll offset
        for i in 0..available_height {
            let row = content_start_row + i as u16;
            stdout.queue(MoveTo(0, row))?;
            stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
            
            stdout.queue(Print("│ "))?;
            
            let line_index = (i as usize) + current_scroll_offset;
            if let Some(line) = payload_lines.get(line_index) {
                // Truncate line if too long for terminal
                let max_content_width = terminal_width.saturating_sub(4); // "│ " + " │"
                if line.len() > max_content_width {
                    let truncated = format!("{}...", &line[..max_content_width.saturating_sub(3)]);
                    stdout.queue(Print(&truncated))?;
                    let padding = max_content_width.saturating_sub(truncated.len());
                    stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
                } else {
                    stdout.queue(Print(line))?;
                    let padding = max_content_width.saturating_sub(line.len());
                    stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
                }
            } else {
                // Empty line
                let padding = terminal_width.saturating_sub(3); // "│ " + "│"
                stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
            }
            
            stdout.queue(Print("│"))?;
        }
        Ok(())
    }
    
    pub fn render_payload_detail_status(&self, stdout: &mut std::io::Stdout, status_start_row: u16,
                                   selected_message: &crate::db::Message, payload_lines: &[String],
                                   available_height: u16) -> Result<()> {
        stdout.queue(MoveTo(0, status_start_row))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let payload_size = selected_message.payload.len();
        let line_count = payload_lines.len();
        let scroll_offset = self.get_payload_detail_scroll_offset();
        let scroll_info = if line_count > available_height as usize {
            format!(" | Lines: {}-{}/{}", 
                    scroll_offset + 1,
                    std::cmp::min(scroll_offset + available_height as usize, line_count),
                    line_count)
        } else {
            format!(" | Lines: {}", line_count)
        };
        stdout.queue(Print(format!("Payload: {} bytes{} | Topic: {}", 
                                 payload_size, scroll_info, selected_message.topic)))?;
        Ok(())
    }
}