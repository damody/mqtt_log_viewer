use crate::db::models::{Message, FilterCriteria};
use crate::db::repository::MessageRepository;
use chrono::{DateTime, Utc};

pub struct MessageListState {
    pub messages: Vec<Message>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub current_topic: Option<String>,
    pub filter: FilterCriteria,
    pub total_count: usize,
    pub page: usize,
    pub per_page: usize,
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
    
    pub fn page_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(10);
    }
    
    pub fn page_down(&mut self) {
        self.selected_index = (self.selected_index + 10).min(self.messages.len().saturating_sub(1));
    }
    
    pub fn get_selected_message(&self) -> Option<&Message> {
        self.messages.get(self.selected_index)
    }
}

pub struct MessageListView;

impl MessageListView {
    // View rendering will be handled in app.rs
}