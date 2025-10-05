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
        self.render_message_list_incremental()?;
        info!("render_message_list() completed - MessageList UI should now be visible");
        self.set_needs_full_redraw(false); // Reset the redraw flag after successful render
        Ok(())
    }
    
    fn render_message_list_incremental(&mut self) -> Result<()> {
        let mut stdout = stdout();
        
        // Only clear screen on full redraw (like when entering message list)
        if self.needs_full_redraw() {
            stdout.execute(Clear(crossterm::terminal::ClearType::All))?;
            stdout.execute(MoveTo(0, 0))?;
            info!("Screen cleared and cursor moved to 0,0");
        }
        
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
        let header = format!("  {:>5} │ {:<10} │ {:<50}", "No.", "Time", "Payload");
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
        
        // Render help line with quick filter status
        stdout.queue(MoveTo(0, status_start_row + 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        
        // 先顯示基本操作說明
        let help_text = "[←]back [Tab]focus [Enter]view [↑↓]navigate [PgUp/PgDn]page";
        stdout.queue(Print(help_text))?;
        
        // 計算快速過濾器狀態的位置（右對齊）
        let config = self.get_config();
        if config.quick_filters.enabled && !config.quick_filters.filters.is_empty() {
            let message_state = self.get_message_list_state();
            let mut filter_status_parts = Vec::new();
            
            for (index, filter) in config.quick_filters.filters.iter().enumerate() {
                if index < 5 { // 只顯示F1-F5
                    let status_symbol = if message_state.get_quick_filter_state(index) { "✓" } else { "✗" };
                    let status_text = format!("[F{}:{} {}]", index + 1, filter.name, status_symbol);
                    filter_status_parts.push((status_text, filter.color.as_str(), message_state.get_quick_filter_state(index)));
                }
            }
            
            // 計算所有狀態文字的總長度
            let total_filter_len: usize = filter_status_parts.iter()
                .map(|(text, _, _)| text.len() + 1) // +1 for space
                .sum();
            
            if total_filter_len > 0 {
                // 計算右對齊位置
                let help_len = help_text.len();
                let available_space = terminal_width.saturating_sub(help_len + total_filter_len + 3); // +3 for " | "
                
                if available_space > 0 {
                    stdout.queue(Print(&format!("{:<width$}", "", width = available_space)))?;
                    stdout.queue(Print(" | "))?;
                    
                    // 顯示每個過濾器狀態，使用對應顏色
                    for (i, (status_text, color_name, is_enabled)) in filter_status_parts.iter().enumerate() {
                        if i > 0 {
                            stdout.queue(Print(" "))?;
                        }
                        
                        // 根據狀態和顏色設置顯示顏色
                        if *is_enabled {
                            // 啟用時使用過濾器對應的顏色
                            let color = match *color_name {
                                "red" => crossterm::style::Color::Red,
                                "green" => crossterm::style::Color::Green,
                                "light_green" => crossterm::style::Color::DarkGreen,
                                "yellow" => crossterm::style::Color::Yellow,
                                "blue" => crossterm::style::Color::Blue,
                                "cyan" => crossterm::style::Color::Cyan,
                                "magenta" => crossterm::style::Color::Magenta,
                                "white" => crossterm::style::Color::White,
                                "dark_grey" => crossterm::style::Color::DarkGrey,
                                "grey" => crossterm::style::Color::Grey,
                                _ => crossterm::style::Color::White,
                            };
                            stdout.queue(SetForegroundColor(color))?;
                        } else {
                            // 停用時使用暗灰色
                            stdout.queue(SetForegroundColor(crossterm::style::Color::DarkGrey))?;
                        }
                        
                        stdout.queue(Print(status_text))?;
                        stdout.queue(ResetColor)?;
                    }
                }
            }
        }
        
        // Position cursor for input if editing
        if let Some((col, row)) = self.get_message_list_cursor_position() {
            stdout.queue(MoveTo(col, row))?;
            stdout.execute(Show)?;
        } else {
            stdout.execute(Hide)?;
        }
        
        stdout.flush()?;
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
        
        // Render separator line with payload selection indicator
        stdout.queue(MoveTo(0, 3))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let selection = self.get_payload_detail_selection();
        
        stdout.queue(Print("├"))?;
        match selection {
            crate::ui::app::PayloadDetailSelection::Payload => {
                stdout.queue(Print("─ Payload "))?;
                stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
                stdout.queue(Print(">> "))?;
                stdout.queue(ResetColor)?;
                let remaining_width = terminal_width.saturating_sub(13); // "├─ Payload >> " = 13 chars
                stdout.queue(Print(&format!("{:─<width$}┤", "", width = remaining_width)))?;
            }
            crate::ui::app::PayloadDetailSelection::FormattedJson => {
                stdout.queue(Print("─ Formatted JSON "))?;
                stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
                stdout.queue(Print(">> "))?;
                stdout.queue(ResetColor)?;
                let remaining_width = terminal_width.saturating_sub(20); // "├─ Formatted JSON >> " = 20 chars
                stdout.queue(Print(&format!("{:─<width$}┤", "", width = remaining_width)))?;
            }
            _ => {
                // Default display for Topic selection
                stdout.queue(Print("─ Payload "))?;
                let remaining_width = terminal_width.saturating_sub(12); // "├─ Payload " = 12 chars
                stdout.queue(Print(&format!("{:─<width$}┤", "", width = remaining_width)))?;
            }
        }
        
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
        stdout.queue(Print("[←]back [Tab]switch [Alt+C]copy [↑↓]scroll [PgUp/PgDn]page [F1]help"))?;
        
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
        
        let is_payload_focused = matches!(message_state.get_focus(), crate::ui::views::message_list::FocusTarget::PayloadFilter);
        
        // 渲染前綴
        stdout.queue(Print("│ Payload Filter: ["))?;
        
        // 如果正在編輯payload filter並且有焦點，渲染帶有遊標反白的文字
        if is_payload_focused && message_state.is_editing {
            let input_text = &message_state.payload_filter_input;
            let cursor_pos = message_state.cursor_position;
            
            // 設置焦點顏色
            stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
            
            // 渲染遊標前的文字
            if cursor_pos > 0 {
                let before_cursor: String = input_text.chars().take(cursor_pos).collect();
                stdout.queue(Print(&before_cursor))?;
            }
            
            // 渲染遊標位置的字元（反白顯示）
            if cursor_pos < input_text.chars().count() {
                let cursor_char = input_text.chars().nth(cursor_pos).unwrap_or(' ');
                stdout.queue(crossterm::style::SetBackgroundColor(crossterm::style::Color::White))?;
                stdout.queue(SetForegroundColor(crossterm::style::Color::Black))?;
                stdout.queue(Print(&cursor_char.to_string()))?;
                stdout.queue(crossterm::style::ResetColor)?;
                stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
                
                // 渲染遊標後的文字
                if cursor_pos + 1 < input_text.chars().count() {
                    let after_cursor: String = input_text.chars().skip(cursor_pos + 1).collect();
                    stdout.queue(Print(&after_cursor))?;
                }
            } else {
                // 遊標在文字末端，顯示一個反白的空格
                stdout.queue(crossterm::style::SetBackgroundColor(crossterm::style::Color::White))?;
                stdout.queue(SetForegroundColor(crossterm::style::Color::Black))?;
                stdout.queue(Print(" "))?;
                stdout.queue(crossterm::style::ResetColor)?;
                stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
            }
            
            stdout.queue(ResetColor)?;
        } else {
            // 正常渲染邏輯
            let payload_filter_text = if !message_state.payload_filter_input.is_empty() {
                &message_state.payload_filter_input
            } else if is_payload_focused {
                if message_state.is_editing {
                    "<<<EDITING>>>"
                } else {
                    "<<<FOCUSED>>>"
                }
            } else {
                "___________________"
            };
            
            if is_payload_focused {
                stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
            }
            stdout.queue(Print(payload_filter_text))?;
            if is_payload_focused {
                stdout.queue(ResetColor)?;
            }
        }
        
        stdout.queue(Print("]"))?;
        
        // 顯示錯誤訊息（如果有的話）
        let content_len = if is_payload_focused && message_state.is_editing {
            message_state.payload_filter_input.len() + 1 // +1 for cursor space
        } else if !message_state.payload_filter_input.is_empty() {
            message_state.payload_filter_input.len()
        } else {
            11 // Length of placeholder text
        };
        let mut line_len = 20 + content_len; // "│ Payload Filter: [" + content + "]"
        if let Some(error) = &message_state.filter_error {
            stdout.queue(SetForegroundColor(crossterm::style::Color::Red))?;
            stdout.queue(Print(&format!(" {}", error)))?;
            stdout.queue(ResetColor)?;
            line_len += error.len() + 1;
        }
        
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
        
        stdout.queue(Print("│ Time: From ["))?;
        
        // 渲染FROM欄位，如果在時間編輯模式則特殊顯示
        if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterFrom) && message_state.time_edit_mode {
            self.render_message_list_time_edit_field(stdout, &message_state.time_from_input, &message_state.time_edit_position)?;
        } else {
            let from_text = if !message_state.time_from_input.is_empty() {
                &message_state.time_from_input
            } else if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterFrom) {
                if message_state.is_editing {
                    "<<<EDITING>>>"
                } else {
                    "<<<FOCUS>>>"
                }
            } else {
                "___________________"
            };
            
            if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterFrom) {
                stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
            }
            stdout.queue(Print(&format!("{:19}", from_text)))?; // 固定19字符寬度
            if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterFrom) {
                stdout.queue(ResetColor)?;
            }
        }
        
        stdout.queue(Print("] To ["))?;
        
        // 渲染TO欄位，如果在時間編輯模式則特殊顯示  
        if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterTo) && message_state.time_edit_mode {
            self.render_message_list_time_edit_field(stdout, &message_state.time_to_input, &message_state.time_edit_position)?;
        } else {
            let to_text = if !message_state.time_to_input.is_empty() {
                &message_state.time_to_input
            } else if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterTo) {
                if message_state.is_editing {
                    "<<<EDITING>>>"
                } else {
                    "<<<FOCUS>>>"
                }
            } else {
                "___________________"
            };
            
            if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterTo) {
                stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
            }
            stdout.queue(Print(&format!("{:19}", to_text)))?; // 固定19字符寬度
            if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterTo) {
                stdout.queue(ResetColor)?;
            }
        }
        
        stdout.queue(Print("]"))?;
        
        // 如果在時間編輯模式，顯示提示
        if message_state.time_edit_mode {
            stdout.queue(Print(" "))?;
            stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
            stdout.queue(Print("[←→:切換 ↑↓:±1 PgUp/Dn:±10]"))?;
            stdout.queue(ResetColor)?;
        }
        
        // 計算剩餘空間並填充
        let base_len = 13 + 19 + 6 + 19 + 1; // "│ Time: From [" + 19 + "] To [" + 19 + "]"
        let hint_len = if message_state.time_edit_mode { 1 + 24 } else { 0 }; // " [←→:切換 ↑↓:±1 PgUp/Dn:±10]"
        let used_len = base_len + hint_len;
        let padding = terminal_width.saturating_sub(used_len + 1);
        stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
        stdout.queue(Print("│"))?;
        Ok(())
    }
    
    
    // 根據訊息內容返回對應的顏色
    fn get_message_color(&self, message: &crate::db::Message) -> Option<crossterm::style::Color> {
        let config = self.get_config();
        if !config.quick_filters.enabled {
            return None;
        }
        
        let content = format!("{} {}", message.topic, message.payload);
        
        // 檢查每個過濾器的模式並返回對應顏色
        for filter in &config.quick_filters.filters {
            let matches = if filter.case_sensitive {
                content.contains(&filter.pattern)
            } else {
                content.to_lowercase().contains(&filter.pattern.to_lowercase())
            };
            
            if matches {
                // 根據顏色名稱返回對應的crossterm顏色
                return match filter.color.as_str() {
                    "red" => Some(crossterm::style::Color::Red),
                    "green" => Some(crossterm::style::Color::Green),
                    "light_green" => Some(crossterm::style::Color::DarkGreen),
                    "yellow" => Some(crossterm::style::Color::Yellow),
                    "blue" => Some(crossterm::style::Color::Blue),
                    "cyan" => Some(crossterm::style::Color::Cyan),
                    "magenta" => Some(crossterm::style::Color::Magenta),
                    "white" => Some(crossterm::style::Color::White),
                    "dark_grey" => Some(crossterm::style::Color::DarkGrey),
                    "grey" => Some(crossterm::style::Color::Grey),
                    _ => None,
                };
            }
        }
        
        None
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
                
                // Check if this is the selected row and we're in delete confirmation mode
                if i as usize == selected_index && message_state.delete_confirmation {
                    // Show delete confirmation prompt instead of normal content
                    let confirmation_msg = format!("刪除此訊息? 再按一次Delete確認刪除");
                    let max_width = terminal_width.saturating_sub(20);
                    let padded_msg = if confirmation_msg.chars().count() > max_width {
                        let truncated: String = confirmation_msg.chars().take(max_width).collect();
                        format!("{}", truncated)
                    } else {
                        format!("{:<width$}", confirmation_msg, width = max_width)
                    };
                    
                    stdout.queue(SetForegroundColor(crossterm::style::Color::Red))?;
                    stdout.queue(Print(&padded_msg))?;
                    stdout.queue(ResetColor)?;
                } else {
                    // Normal message display
                    // 計算流水號（考慮分頁，越新的訊息數字越大）
                    // 總數 - ((當前頁-1) * 每頁數量 + 當前索引)
                    let page_offset = message_state.page.saturating_sub(1).saturating_mul(message_state.per_page);
                    let offset = page_offset.saturating_add(i as usize);
                    let sequence_number = if offset < message_state.total_count {
                        message_state.total_count - offset
                    } else {
                        error!("流水號計算錯誤: offset ({}) >= total_count ({}), page={}, per_page={}, i={}, page_offset={}", 
                               offset, message_state.total_count, message_state.page, message_state.per_page, i, page_offset);
                        1 // 防止下溢，使用最小值1
                    };
                    
                    // 顯示流水號
                    stdout.queue(Print(&format!("{:>5} │ ", sequence_number)))?;
                    
                    // Format timestamp (HH:MM:SS) - convert from UTC to Local time
                    let local_time = msg.timestamp.with_timezone(&chrono::Local);
                    let time_str = local_time.format("%H:%M:%S").to_string();
                    stdout.queue(Print(&format!("{:<10}", time_str)))?;
                    stdout.queue(Print(" │ "))?;
                    
                    // Truncate payload if too long
                    // 精確計算寬度：
                    // │ (2) + >> (3) + 4616 (5) +  │  (3) + 22:54:15 (10) +  │  (3) + payload + │ (1)
                    // = 2 + 3 + 5 + 3 + 10 + 3 + payload + 1 = 27 + payload
                    let max_payload_width = terminal_width.saturating_sub(20);
                    let payload_display = if msg.payload.chars().count() > max_payload_width {
                        // 使用 Unicode 安全的字符截斷
                        let truncate_len = max_payload_width.saturating_sub(3);
                        let truncated: String = msg.payload.chars().take(truncate_len).collect();
                        format!("{}...", truncated)
                    } else {
                        format!("{:<width$}", msg.payload, width = max_payload_width)
                    };
                    
                    // 檢查並應用快速過濾器顏色
                    let color = self.get_message_color(msg);
                    if let Some(c) = color {
                        stdout.queue(SetForegroundColor(c))?;
                    }
                    
                    stdout.queue(Print(&payload_display))?;
                    
                    // 重置顏色
                    if color.is_some() {
                        stdout.queue(ResetColor)?;
                    }
                }
                
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
        
        // Render metadata line 2 - Topic with selection indicator and QoS/Retain
        stdout.queue(MoveTo(0, 2))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        
        // 顯示Topic（帶選擇指示器）
        let is_topic_selected = matches!(self.get_payload_detail_selection(), crate::ui::app::PayloadDetailSelection::Topic);
        stdout.queue(Print("│ Topic: "))?;
        if is_topic_selected {
            stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
            stdout.queue(Print(">>"))?;
        } else {
            stdout.queue(Print("  "))?;
        }
        stdout.queue(Print(&selected_message.topic))?;
        if is_topic_selected {
            stdout.queue(ResetColor)?;
        }
        
        // FormattedJson selector on the same line
        let is_json_selected = matches!(self.get_payload_detail_selection(), crate::ui::app::PayloadDetailSelection::FormattedJson);
        stdout.queue(Print(" | JSON: "))?;
        if is_json_selected {
            stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
            stdout.queue(Print(">>"))?;
        } else {
            stdout.queue(Print("  "))?;
        }
        stdout.queue(Print("Formatted"))?;
        if is_json_selected {
            stdout.queue(ResetColor)?;
        }
        
        // QoS and Retain info
        let qos_retain_text = format!(" | QoS: {} | Retain: {}", selected_message.qos, selected_message.retain);
        stdout.queue(Print(&qos_retain_text))?;
        
        // 簡化padding計算，避免overflow
        let min_padding = if terminal_width > 50 { 10 } else { 1 };
        stdout.queue(Print(&format!("{:<width$}", "", width = min_padding)))?;
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
        
        // 計算最大行號的寬度（用於對齊）
        let total_lines = payload_lines.len();
        let line_number_width = if total_lines == 0 {
            1
        } else {
            ((total_lines as f64).log10().floor() as usize) + 1
        };
        
        // Render payload content with scroll offset and line numbers
        for i in 0..available_height {
            let row = content_start_row + i as u16;
            stdout.queue(MoveTo(0, row))?;
            stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
            
            stdout.queue(Print("│ "))?;
            
            let line_index = (i as usize) + current_scroll_offset;
            if let Some(line) = payload_lines.get(line_index) {
                // 顯示行號（從1開始）
                let line_number = line_index + 1;
                stdout.queue(SetForegroundColor(crossterm::style::Color::DarkGrey))?;
                stdout.queue(Print(&format!("{:>width$} ", line_number, width = line_number_width)))?;
                stdout.queue(ResetColor)?;
                
                // 計算內容可用寬度：終端寬度 - "│ " - 行號 - " " - " │"
                let line_number_space = line_number_width + 1; // 行號 + 一個空格
                let max_content_width = terminal_width.saturating_sub(4 + line_number_space); // "│ " + 行號空間 + " │"
                
                if line.chars().count() > max_content_width {
                    let truncate_len = max_content_width.saturating_sub(3);
                    let truncated_str: String = line.chars().take(truncate_len).collect();
                    let truncated = format!("{}...", truncated_str);
                    stdout.queue(Print(&truncated))?;
                    let padding = max_content_width.saturating_sub(truncated.chars().count());
                    stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
                } else {
                    stdout.queue(Print(line))?;
                    let padding = max_content_width.saturating_sub(line.len());
                    stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
                }
            } else {
                // Empty line - 只顯示空白，不顯示行號
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
    
    // 渲染時間編輯欄位的輔助方法
    fn render_message_list_time_edit_field(&self, stdout: &mut std::io::Stdout, value: &str, position: &crate::ui::views::message_list::TimeEditPosition) -> Result<()> {
        use crate::ui::views::message_list::TimeEditPosition;
        
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
                    stdout.queue(SetForegroundColor(crossterm::style::Color::Green))?;
                    stdout.queue(Print(part))?;
                    stdout.queue(ResetColor)?;
                } else {
                    stdout.queue(Print(part))?;
                }
                
                // 添加分隔符
                if i < 2 {
                    stdout.queue(Print("-"))?;
                } else if i == 2 {
                    stdout.queue(Print(" "))?;
                } else if i < 5 {
                    stdout.queue(Print(":"))?;
                }
            }
        } else {
            stdout.queue(Print(&format!("{:19}", value)))?;
        }
        
        Ok(())
    }
}