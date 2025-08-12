use crate::db::models::{Message, FilterCriteria};
use crate::db::repository::MessageRepository;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq)]
pub enum FocusTarget {
    PayloadFilter,
    TimeFilterFrom,
    TimeFilterTo,
    MessageList,
}

pub struct MessageListState {
    pub messages: Vec<Message>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub current_topic: Option<String>,
    pub filter: FilterCriteria,
    pub total_count: usize,
    pub page: usize,
    pub per_page: usize,
    pub focus: FocusTarget,
    pub payload_filter_input: String,
    pub time_from_input: String,
    pub time_to_input: String,
    pub is_editing: bool,
    pub filter_error: Option<String>, // 用於顯示過濾器錯誤
    pub cursor_position: usize, // 當前遊標在輸入欄位中的位置
    pub delete_confirmation: bool, // 刪除確認標誌
}

impl MessageListState {
    pub fn new() -> Self {
        use chrono::{Duration, Local};
        let now = Local::now();
        let yesterday = now - Duration::days(1);
        let tomorrow = now + Duration::days(1);
        
        Self {
            messages: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            current_topic: None,
            filter: FilterCriteria::default(),
            total_count: 0,
            page: 1,
            per_page: 10, // Default value, will be updated based on terminal size
            focus: FocusTarget::MessageList,
            payload_filter_input: String::new(),
            time_from_input: yesterday.format("%Y-%m-%d %H:%M:%S").to_string(),
            time_to_input: tomorrow.format("%Y-%m-%d %H:%M:%S").to_string(),
            is_editing: false,
            filter_error: None,
            cursor_position: 0,
            delete_confirmation: false,
        }
    }
    
    pub fn calculate_per_page(terminal_height: u16) -> usize {
        let content_start_row = 5;  // Title, filters take 5 rows
        let status_rows = 2;        // Status bar takes 2 rows  
        let bottom_border = 1;      // Bottom border takes 1 row
        let available_height = terminal_height.saturating_sub(content_start_row + status_rows + bottom_border);
        available_height as usize
    }
    
    pub fn update_per_page(&mut self, terminal_height: u16) {
        let new_per_page = Self::calculate_per_page(terminal_height);
        if new_per_page != self.per_page {
            self.per_page = new_per_page;
            // If the terminal size changed significantly, we might need to reload
            // But let's keep it simple for now
        }
    }
    
    pub fn clear(&mut self) {
        self.messages.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.total_count = 0;
        self.page = 1;
    }
    
    pub fn set_topic(&mut self, topic: String) {
        tracing::info!("set_topic: topic set to {}", topic);
        self.current_topic = Some(topic);
        self.clear();
        // Don't clear filters here - let the user decide when to clear them
    }
    
    pub fn update_filter_from_inputs(&mut self) {
        // 更新 payload 過濾
        if !self.payload_filter_input.is_empty() {
            self.filter.payload_regex = Some(self.payload_filter_input.clone());
        } else {
            self.filter.payload_regex = None;
        }
        
        // 更新時間過濾 - From
        if !self.time_from_input.is_empty() {
            if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(&self.time_from_input, "%Y-%m-%d %H:%M:%S") {
                use chrono::{TimeZone, Local};
                if let Some(dt) = Local.from_local_datetime(&naive_dt).single() {
                    self.filter.start_time = Some(dt.with_timezone(&chrono::Utc));
                }
            }
        } else {
            self.filter.start_time = None;
        }
        
        // 更新時間過濾 - To
        if !self.time_to_input.is_empty() {
            if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(&self.time_to_input, "%Y-%m-%d %H:%M:%S") {
                use chrono::{TimeZone, Local};
                if let Some(dt) = Local.from_local_datetime(&naive_dt).single() {
                    self.filter.end_time = Some(dt.with_timezone(&chrono::Utc));
                }
            }
        } else {
            self.filter.end_time = None;
        }
    }
    
    pub async fn load_messages(&mut self, repo: &MessageRepository) -> anyhow::Result<()> {
        if let Some(topic) = self.current_topic.clone() {
            // 更新過濾條件，包含時間過濾
            self.update_filter_from_inputs();
            
            let mut filter = self.filter.clone();
            filter.limit = Some(self.per_page as i64);
            filter.offset = Some(((self.page - 1) * self.per_page) as i64);
            
            tracing::info!("load_messages filter: {:?}", filter);
            self.messages = repo.get_messages_by_topic(&topic, &filter).await?;
            
            // Get total count for this topic with same filters (but without limit/offset)
            let count_filter = FilterCriteria {
                topic_regex: self.filter.topic_regex.clone(),
                payload_regex: self.filter.payload_regex.clone(),
                start_time: self.filter.start_time.clone(),
                end_time: self.filter.end_time.clone(),
                limit: None,
                offset: None,
            };
            tracing::info!("load_messages count_filter: {:?}", count_filter);
            if let Ok(count_messages) = repo.get_messages_by_topic(&topic, &count_filter).await {
                self.total_count = count_messages.len();
            }
            
            if self.selected_index >= self.messages.len() && !self.messages.is_empty() {
                self.selected_index = self.messages.len() - 1;
            }
        }
        Ok(())
    }
    
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }
    
    pub async fn move_up_with_pagination(&mut self, repo: &MessageRepository) -> anyhow::Result<()> {
        self.delete_confirmation = false; // 清除刪除確認狀態
        if self.selected_index > 0 {
            self.selected_index -= 1;
        } else if self.page > 1 {
            // We're at the top of current page and there are previous pages
            self.page -= 1;
            self.load_messages(repo).await?;
            // Move to the last item of the previous page
            if !self.messages.is_empty() {
                self.selected_index = self.messages.len() - 1;
            }
        }
        Ok(())
    }
    
    pub fn move_down(&mut self) {
        if self.selected_index < self.messages.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }
    
    pub async fn move_down_with_pagination(&mut self, repo: &MessageRepository) -> anyhow::Result<()> {
        self.delete_confirmation = false; // 清除刪除確認狀態
        tracing::info!("move_down_with_pagination {} {}", self.selected_index, self.messages.len().saturating_sub(1));
        if self.selected_index < self.messages.len().saturating_sub(1) {
            self.selected_index += 1;
        } else {
            // We're at the bottom of current page, check if there are more pages
            let current_messages_end = self.page * self.per_page;
            if current_messages_end < self.total_count {
                // There are more messages, load next page
                let current_page = self.page;
                self.page += 1;
                self.load_messages(repo).await?;
                
                if self.messages.is_empty() {
                    // Failed to load next page, revert
                    self.page = current_page;
                    self.load_messages(repo).await?;
                } else {
                    // Successfully loaded next page, move to first item
                    self.selected_index = 0;
                }
            }
        }
        Ok(())
    }
    
    pub async fn page_up(&mut self, repo: &MessageRepository) -> anyhow::Result<()> {
        if self.page > 1 {
            self.page -= 1;
            self.selected_index = 0;
            self.load_messages(repo).await?;
        }
        Ok(())
    }
    
    pub async fn page_down(&mut self, repo: &MessageRepository) -> anyhow::Result<()> {
        // Check if there are more pages based on total count
        let current_messages_end = self.page * self.per_page;
        if current_messages_end < self.total_count {
            // There are more messages, load next page
            self.page += 1;
            self.load_messages(repo).await?;
            self.selected_index = 0;
        }
        Ok(())
    }
    
    pub fn page_up_selection(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(10);
    }
    
    pub fn page_down_selection(&mut self) {
        self.selected_index = (self.selected_index + 10).min(self.messages.len().saturating_sub(1));
    }
    
    pub async fn move_to_top(&mut self, repo: &MessageRepository) -> anyhow::Result<()> {
        tracing::debug!("move_to_top called - jumping to first page");
        // Jump to first page and first element
        self.page = 1;
        self.load_messages(repo).await?;
        if !self.messages.is_empty() {
            self.selected_index = 0;
            tracing::debug!("Moved to top - page: {}, selected_index: {}", self.page, self.selected_index);
        }
        Ok(())
    }
    
    pub async fn move_to_bottom(&mut self, repo: &MessageRepository) -> anyhow::Result<()> {
        tracing::debug!("move_to_bottom called - jumping to last page");
        // Calculate last page number
        let last_page = if self.total_count == 0 {
            1
        } else {
            ((self.total_count - 1) / self.per_page) + 1
        };
        
        // Jump to last page
        self.page = last_page;
        self.load_messages(repo).await?;
        
        if !self.messages.is_empty() {
            self.selected_index = self.messages.len() - 1;
            tracing::debug!("Moved to bottom - page: {}, selected_index: {}, total_count: {}", 
                          self.page, self.selected_index, self.total_count);
        }
        Ok(())
    }
    
    pub fn get_selected_message(&self) -> Option<&Message> {
        self.messages.get(self.selected_index)
    }
    
    pub fn next_focus(&mut self) {
        self.focus = match self.focus {
            FocusTarget::PayloadFilter => FocusTarget::TimeFilterFrom,
            FocusTarget::TimeFilterFrom => FocusTarget::TimeFilterTo,
            FocusTarget::TimeFilterTo => FocusTarget::MessageList,
            FocusTarget::MessageList => FocusTarget::PayloadFilter,
        };
    }
    
    pub fn get_focus(&self) -> &FocusTarget {
        &self.focus
    }
    
    pub fn set_focus(&mut self, focus: FocusTarget) {
        self.focus = focus;
    }
    
    pub fn get_active_input(&self) -> &str {
        match self.focus {
            FocusTarget::PayloadFilter => &self.payload_filter_input,
            FocusTarget::TimeFilterFrom => &self.time_from_input,
            FocusTarget::TimeFilterTo => &self.time_to_input,
            FocusTarget::MessageList => "",
        }
    }
    
    pub fn get_active_input_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            FocusTarget::PayloadFilter => Some(&mut self.payload_filter_input),
            FocusTarget::TimeFilterFrom => Some(&mut self.time_from_input),
            FocusTarget::TimeFilterTo => Some(&mut self.time_to_input),
            FocusTarget::MessageList => None,
        }
    }
    
    pub fn start_editing(&mut self) {
        if matches!(self.focus, FocusTarget::PayloadFilter | FocusTarget::TimeFilterFrom | FocusTarget::TimeFilterTo) {
            self.is_editing = true;
            // 將遊標移到當前輸入欄位的末端
            self.cursor_position = self.get_active_input().len();
        }
    }
    
    pub fn stop_editing(&mut self) {
        self.is_editing = false;
    }
    
    // 遊標操作方法
    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }
    
    pub fn move_cursor_right(&mut self) {
        let current_text_len = self.get_active_input().len();
        if self.cursor_position < current_text_len {
            self.cursor_position += 1;
        }
    }
    
    pub fn move_cursor_home(&mut self) {
        self.cursor_position = 0;
    }
    
    pub fn move_cursor_end(&mut self) {
        self.cursor_position = self.get_active_input().len();
    }
    
    // 在遊標位置插入文字
    pub fn insert_char_at_cursor(&mut self, c: char) {
        let cursor_pos = self.cursor_position;
        if let Some(input) = self.get_active_input_mut() {
            if cursor_pos <= input.len() {
                input.insert(cursor_pos, c);
                self.cursor_position += 1;
            }
        }
    }
    
    // 在遊標位置刪除字元（向後刪除）
    pub fn delete_char_at_cursor(&mut self) {
        let cursor_pos = self.cursor_position;
        if let Some(input) = self.get_active_input_mut() {
            if cursor_pos > 0 && cursor_pos <= input.len() {
                input.remove(cursor_pos - 1);
                self.cursor_position -= 1;
            }
        }
    }
    
    // 在遊標位置插入字串（用於貼上）
    pub fn insert_string_at_cursor(&mut self, s: &str) {
        let cursor_pos = self.cursor_position;
        if let Some(input) = self.get_active_input_mut() {
            if cursor_pos <= input.len() {
                input.insert_str(cursor_pos, s);
                self.cursor_position += s.len();
            }
        }
    }
    
    pub fn get_cursor_position(&self) -> Option<(u16, u16)> {
        if !self.is_editing {
            return None;
        }
        
        let (field_row, field_col) = match self.focus {
            FocusTarget::PayloadFilter => (1, 17), // "│ Payload Filter: [" = 17 chars
            FocusTarget::TimeFilterFrom => (2, 13), // "│ Time: From [" = 13 chars  
            FocusTarget::TimeFilterTo => {
                let from_len = self.time_from_input.len().min(11);
                (2, 13 + from_len as u16 + 6) // "│ Time: From [xxx] To [" = 13 + len + 6
            },
            FocusTarget::MessageList => return None, // No cursor for message list
        };
        
        let cursor_offset = self.cursor_position.min(11) as u16;
        
        Some((field_col + cursor_offset, field_row))
    }
}

pub struct MessageListView;

impl MessageListView {
    // View rendering will be handled in app.rs
}