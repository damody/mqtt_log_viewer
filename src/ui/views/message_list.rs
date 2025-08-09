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
}

impl MessageListState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            current_topic: None,
            filter: FilterCriteria::default(),
            total_count: 0,
            page: 1,
            per_page: 100,
            focus: FocusTarget::MessageList,
            payload_filter_input: String::new(),
            time_from_input: String::new(),
            time_to_input: String::new(),
            is_editing: false,
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
        self.current_topic = Some(topic);
        self.clear();
    }
    
    pub async fn load_messages(&mut self, repo: &MessageRepository) -> anyhow::Result<()> {
        if let Some(topic) = &self.current_topic {
            let mut filter = self.filter.clone();
            filter.limit = Some(self.per_page as i64);
            filter.offset = Some(((self.page - 1) * self.per_page) as i64);
            
            self.messages = repo.get_messages_by_topic(topic, &filter).await?;
            self.total_count = self.messages.len();
            
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
    
    pub fn move_down(&mut self) {
        if self.selected_index < self.messages.len().saturating_sub(1) {
            self.selected_index += 1;
        }
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
        // Load next page to check if there are more messages
        let current_page = self.page;
        self.page += 1;
        let old_messages_len = self.messages.len();
        self.load_messages(repo).await?;
        
        if self.messages.is_empty() || self.messages.len() < self.per_page {
            // No more messages, revert to previous page
            self.page = current_page;
            self.load_messages(repo).await?;
        } else {
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
        }
    }
    
    pub fn stop_editing(&mut self) {
        self.is_editing = false;
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
        
        let cursor_offset = match self.focus {
            FocusTarget::PayloadFilter => self.payload_filter_input.len().min(11) as u16,
            FocusTarget::TimeFilterFrom => self.time_from_input.len().min(11) as u16,
            FocusTarget::TimeFilterTo => self.time_to_input.len().min(11) as u16,
            FocusTarget::MessageList => 0,
        };
        
        Some((field_col + cursor_offset, field_row))
    }
}

pub struct MessageListView;

impl MessageListView {
    // View rendering will be handled in app.rs
}