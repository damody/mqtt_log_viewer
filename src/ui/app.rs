use std::time::{Duration, Instant};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode, size, Clear, ClearType},
    cursor::{Hide, Show, MoveTo},
    style::{Print, SetForegroundColor, ResetColor},
    ExecutableCommand, QueueableCommand,
};
use tokio::sync::mpsc;
use anyhow::Result;
use tracing::{info, error, warn};
use std::io::{stdout, Write};

#[cfg(windows)]
use winapi::um::winuser::{GetAsyncKeyState, VK_ESCAPE};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::Arc;

use crate::config::Config;
use crate::db::{MessageRepository, FilterCriteria};
use crate::ui::events::AppEvent;
use crate::ui::widgets::{FilterState, FilterBar, StatusBarState, StatusBar, ViewType, ConnectionStatus};
use crate::mqtt::MqttClient;
use crate::ui::views::{TopicListState, TopicListView, MessageListState};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppState {
    TopicList,
    MessageList,
    PayloadDetail,
    Help,
    Quit,
}

pub struct App {
    state: AppState,
    config: Config,
    repository: MessageRepository,
    
    // UI State
    filter_state: FilterState,
    status_bar_state: StatusBarState,
    topic_list_state: TopicListState,
    message_list_state: MessageListState,
    
    // MQTT connection info
    mqtt_host: String,
    mqtt_port: u16,
    
    // Previous state for diff detection
    prev_filter_state: Option<FilterState>,
    prev_status_bar_state: Option<StatusBarState>,
    prev_topic_list_state: Option<TopicListState>,
    
    // Timing
    last_refresh: Instant,
    refresh_interval: Duration,
    
    // Terminal
    terminal_width: u16,
    terminal_height: u16,
    needs_full_redraw: bool,
    
    // Windows API key state tracking
    #[cfg(windows)]
    last_key_state: std::collections::HashMap<i32, bool>,
}

impl App {
    pub async fn new(config: Config) -> Result<Self> {
        let repository = MessageRepository::new(&config.database.path).await?;
        let (width, height) = size()?;
        
        let mut app = Self {
            state: AppState::TopicList,
            config: config.clone(),
            repository,
            filter_state: FilterState::default(),
            status_bar_state: StatusBarState::default(),
            topic_list_state: TopicListState::default(),
            message_list_state: MessageListState::new(),
            mqtt_host: config.mqtt.host.clone(),
            mqtt_port: config.mqtt.port,
            prev_filter_state: None,
            prev_status_bar_state: None,
            prev_topic_list_state: None,
            last_refresh: Instant::now(),
            refresh_interval: Duration::from_millis(config.ui.refresh_interval_ms),
            terminal_width: width,
            terminal_height: height,
            needs_full_redraw: true,
            #[cfg(windows)]
            last_key_state: std::collections::HashMap::new(),
        };
        
        // Set initial help text
        StatusBar::set_help_text_for_view(&mut app.status_bar_state, &ViewType::TopicList);
        
        Ok(app)
    }
    
    pub async fn run(&mut self) -> Result<()> {
        info!("Starting MQTT Log Viewer application");
        
        // Initialize terminal
        enable_raw_mode()?;
        let mut stdout = stdout();
        stdout.execute(Hide)?;
        stdout.execute(Clear(ClearType::All))?;
        stdout.execute(MoveTo(0, 0))?;
        
        let result = self.main_loop().await;
        
        // Cleanup terminal
        std::io::stdout().execute(Show)?;
        disable_raw_mode()?;
        std::io::stdout().execute(Clear(ClearType::All))?;
        
        result
    }
    
    pub async fn run_with_connection_status(
        &mut self, 
        connection_status: std::sync::Arc<std::sync::Mutex<bool>>
    ) -> Result<()> {
        info!("Starting MQTT Log Viewer application with connection monitoring");
        
        // Initialize terminal
        enable_raw_mode()?;
        let mut stdout = stdout();
        stdout.execute(Hide)?;
        stdout.execute(Clear(ClearType::All))?;
        stdout.execute(MoveTo(0, 0))?;
        
        let result = self.main_loop_with_status(connection_status).await;
        
        // Cleanup terminal
        std::io::stdout().execute(Show)?;
        disable_raw_mode()?;
        std::io::stdout().execute(Clear(ClearType::All))?;
        
        result
    }
    
    async fn main_loop_with_status(
        &mut self, 
        connection_status: std::sync::Arc<std::sync::Mutex<bool>>
    ) -> Result<()> {
        let mut last_refresh = Instant::now();
        
        // Initial data load
        self.refresh_data().await?;
        self.render()?;
        
        #[cfg(windows)]
        let exit_flag = Arc::new(AtomicBool::new(false));
        
        loop {
            // Check and update connection status
            if let Ok(is_connected) = connection_status.lock() {
                self.update_connection_status_from_mqtt(*is_connected);
            }
            
            // 每0.25秒刷新資料 (只在第一層)
            let now = Instant::now();
            if now.duration_since(last_refresh) >= self.refresh_interval 
                && self.state == AppState::TopicList {
                self.refresh_data().await?;
                // 不強制完全重繪，讓增量渲染決定
                self.render()?;
                last_refresh = now;
            }
            
            // Windows API 直接按鍵檢測（邊沿觸發）
            #[cfg(windows)]
            {
                // 檢測 ESC 鍵
                if self.is_key_just_pressed(VK_ESCAPE) {
                    std::fs::write("debug_key.txt", "ESC key detected via WinAPI!").ok();
                    tracing::info!("ESC key detected via Windows API");
                    break;
                }
                
                // 檢測 'Q' 鍵 (VK code 81)
                if self.is_key_just_pressed(81) {
                    std::fs::write("debug_key.txt", "Q key detected via WinAPI!").ok();
                    tracing::info!("Q key detected via Windows API");
                    break;
                }
                
                // 檢測上下箭頭鍵進行導航
                if self.is_key_just_pressed(0x26) { // VK_UP
                    std::fs::write("debug_key.txt", "UP key detected via WinAPI!").ok();
                    tracing::debug!("UP key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateUp).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x28) { // VK_DOWN
                    std::fs::write("debug_key.txt", "DOWN key detected via WinAPI!").ok();
                    tracing::debug!("DOWN key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateDown).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x25) { // VK_LEFT
                    std::fs::write("debug_key.txt", "LEFT key detected via WinAPI!").ok();
                    tracing::debug!("LEFT key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateLeft).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x27) { // VK_RIGHT
                    std::fs::write("debug_key.txt", "RIGHT key detected via WinAPI!").ok();
                    tracing::debug!("RIGHT key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateRight).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x0D) { // VK_RETURN (Enter)
                    std::fs::write("debug_key.txt", "ENTER key detected via WinAPI!").ok();
                    tracing::debug!("ENTER key detected via Windows API");
                    if self.handle_event(AppEvent::Enter).await? {
                        break;
                    }
                    self.render()?;
                }
                
                // 檢測 Ctrl+C (需要同時檢測兩個鍵)
                unsafe {
                    if (GetAsyncKeyState(0x11) & (0x8000u16 as i16) != 0) && (GetAsyncKeyState(67) & (0x8000u16 as i16) != 0) {
                        std::fs::write("debug_key.txt", "Ctrl+C detected via WinAPI!").ok();
                        tracing::info!("Ctrl+C detected via Windows API");
                        break;
                    }
                }
            }
            
            // Handle resize events only (key events handled by Windows API)
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Resize(width, height) => {
                        self.terminal_width = width;
                        self.terminal_height = height;
                        self.update_visible_rows();
                        self.needs_full_redraw = true;
                        self.render()?;
                    }
                    _ => {}
                }
            }
        }
        
        info!("Application shutting down");
        Ok(())
    }
    
    async fn main_loop(&mut self) -> Result<()> {
        let mut last_refresh = Instant::now();
        
        // Initial data load
        self.refresh_data().await?;
        self.render()?;
        
        #[cfg(windows)]
        let exit_flag = Arc::new(AtomicBool::new(false));
        
        loop {
            // 每0.25秒刷新資料 (只在第一層)
            let now = Instant::now();
            if now.duration_since(last_refresh) >= self.refresh_interval 
                && self.state == AppState::TopicList {
                self.refresh_data().await?;
                // 不強制完全重繪，讓增量渲染決定
                self.render()?;
                last_refresh = now;
            }
            
            // Windows API 直接按鍵檢測（邊沿觸發）
            #[cfg(windows)]
            {
                // 檢測 ESC 鍵
                if self.is_key_just_pressed(VK_ESCAPE) {
                    std::fs::write("debug_key.txt", "ESC key detected via WinAPI!").ok();
                    tracing::info!("ESC key detected via Windows API");
                    break;
                }
                
                // 檢測 'Q' 鍵 (VK code 81)
                if self.is_key_just_pressed(81) {
                    std::fs::write("debug_key.txt", "Q key detected via WinAPI!").ok();
                    tracing::info!("Q key detected via Windows API");
                    break;
                }
                
                // 檢測上下箭頭鍵進行導航
                if self.is_key_just_pressed(0x26) { // VK_UP
                    std::fs::write("debug_key.txt", "UP key detected via WinAPI!").ok();
                    tracing::debug!("UP key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateUp).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x28) { // VK_DOWN
                    std::fs::write("debug_key.txt", "DOWN key detected via WinAPI!").ok();
                    tracing::debug!("DOWN key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateDown).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x25) { // VK_LEFT
                    std::fs::write("debug_key.txt", "LEFT key detected via WinAPI!").ok();
                    tracing::debug!("LEFT key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateLeft).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x27) { // VK_RIGHT
                    std::fs::write("debug_key.txt", "RIGHT key detected via WinAPI!").ok();
                    tracing::debug!("RIGHT key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateRight).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x0D) { // VK_RETURN (Enter)
                    std::fs::write("debug_key.txt", "ENTER key detected via WinAPI!").ok();
                    tracing::debug!("ENTER key detected via Windows API");
                    if self.handle_event(AppEvent::Enter).await? {
                        break;
                    }
                    self.render()?;
                }
                
                // 檢測 Ctrl+C (需要同時檢測兩個鍵)
                unsafe {
                    if (GetAsyncKeyState(0x11) & (0x8000u16 as i16) != 0) && (GetAsyncKeyState(67) & (0x8000u16 as i16) != 0) {
                        std::fs::write("debug_key.txt", "Ctrl+C detected via WinAPI!").ok();
                        tracing::info!("Ctrl+C detected via Windows API");
                        break;
                    }
                }
            }
            
            // Handle resize events only (key events handled by Windows API)
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Resize(width, height) => {
                        self.terminal_width = width;
                        self.terminal_height = height;
                        self.update_visible_rows();
                        self.needs_full_redraw = true;
                        self.render()?;
                    }
                    _ => {}
                }
            }
        }
        
        info!("Application shutting down");
        Ok(())
    }
    
    async fn handle_event(&mut self, event: AppEvent) -> Result<bool> {
        match event {
            AppEvent::Quit => return Ok(true),
            
            AppEvent::Refresh => {
                self.refresh_data().await?;
            }
            
            AppEvent::Filter => {
                self.toggle_filter_mode();
            }
            
            AppEvent::Escape => {
                if self.filter_state.is_editing {
                    self.filter_state.is_editing = false;
                } else {
                    self.navigate_back()?;
                }
            }
            
            AppEvent::Enter => {
                tracing::debug!("Enter key pressed - is_editing: {}", self.filter_state.is_editing);
                if self.filter_state.is_editing {
                    self.apply_filters().await?;
                    self.filter_state.is_editing = false;
                } else {
                    self.navigate_forward().await?;
                }
            }
            
            _ => {
                // Handle state-specific events
                match self.state {
                    AppState::TopicList => self.handle_topic_list_event(event).await?,
                    AppState::MessageList => self.handle_message_list_event(event).await?,
                    AppState::PayloadDetail => self.handle_payload_detail_event(event).await?,
                    _ => {}
                }
            }
        }
        
        Ok(false)
    }
    
    async fn handle_topic_list_event(&mut self, event: AppEvent) -> Result<()> {
        if self.filter_state.is_editing {
            self.handle_filter_input(event);
            return Ok(());
        }
        
        tracing::debug!("Handling topic list event: {:?}", event);
        match event {
            AppEvent::NavigateUp => {
                tracing::debug!("Navigate up - topics count: {}, selected_index: {}", 
                               self.topic_list_state.topics.len(), self.topic_list_state.selected_index);
                self.topic_list_state.move_up();
            },
            AppEvent::NavigateDown => {
                tracing::debug!("Navigate down - topics count: {}, selected_index: {}", 
                               self.topic_list_state.topics.len(), self.topic_list_state.selected_index);
                self.topic_list_state.move_down();
            },
            AppEvent::NavigateLeft => {
                // Left key goes back to previous layer - same as Escape when not editing
                tracing::debug!("Navigate left - going back to previous layer");
                self.navigate_back()?;
            },
            AppEvent::PageUp => self.topic_list_state.page_up(),
            AppEvent::PageDown => self.topic_list_state.page_down(),
            _ => {}
        }
        
        Ok(())
    }
    
    async fn handle_message_list_event(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::NavigateUp => {
                tracing::debug!("Navigate up in message list");
                self.message_list_state.move_up();
            }
            AppEvent::NavigateDown => {
                tracing::debug!("Navigate down in message list");
                self.message_list_state.move_down();
            }
            AppEvent::PageUp => {
                self.message_list_state.page_up();
            }
            AppEvent::PageDown => {
                self.message_list_state.page_down();
            }
            AppEvent::NavigateLeft => {
                tracing::debug!("Navigate left from message list - returning to topic list");
                self.navigate_back()?;
            }
            AppEvent::NavigateRight | AppEvent::Enter => {
                if let Some(_msg) = self.message_list_state.get_selected_message() {
                    // Navigate to payload detail
                    self.state = AppState::PayloadDetail;
                    self.needs_full_redraw = true;
                    // TODO: Set payload detail state
                }
            }
            _ => {}
        }
        Ok(())
    }
    
    async fn handle_payload_detail_event(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::NavigateLeft => {
                tracing::debug!("Navigate left from payload detail - returning to message list");
                self.navigate_back()?;
            }
            // TODO: Implement other payload detail navigation
            _ => {}
        }
        Ok(())
    }
    
    fn handle_filter_input(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(c) if c != '\0' => {
                self.filter_state.get_active_field_value_mut().push(c);
            }
            AppEvent::Backspace => {
                self.filter_state.get_active_field_value_mut().pop();
            }
            AppEvent::NavigateRight => {
                self.filter_state.next_field();
            }
            AppEvent::NavigateLeft => {
                self.filter_state.previous_field();
            }
            _ => {}
        }
    }
    
    fn toggle_filter_mode(&mut self) {
        if self.filter_state.is_editing {
            self.filter_state.is_editing = false;
        } else {
            self.filter_state.is_editing = true;
            self.filter_state.active_field = crate::ui::widgets::FilterField::Topic;
        }
    }
    
    async fn apply_filters(&mut self) -> Result<()> {
        info!("Applying filters");
        self.refresh_data().await?;
        Ok(())
    }
    
    fn navigate_back(&mut self) -> Result<()> {
        match self.state {
            AppState::MessageList => {
                self.state = AppState::TopicList;
                self.needs_full_redraw = true; // 強制完全重繪
                StatusBar::set_help_text_for_view(&mut self.status_bar_state, &ViewType::TopicList);
                // 立即渲染第一層UI
                self.render()?;
            }
            AppState::PayloadDetail => {
                self.state = AppState::MessageList;
                self.needs_full_redraw = true; // 強制完全重繪
                // TODO: Set message list help text
                self.render()?;
            }
            _ => {}
        }
        Ok(())
    }
    
    async fn navigate_forward(&mut self) -> Result<()> {
        match self.state {
            AppState::TopicList => {
                if let Some(selected_topic) = self.topic_list_state.get_selected_topic() {
                    info!("Navigating to messages for topic: {}", selected_topic.topic);
                    
                    // Set topic and load messages
                    self.message_list_state.set_topic(selected_topic.topic.clone());
                    self.message_list_state.load_messages(&self.repository).await?;
                    
                    self.state = AppState::MessageList;
                    self.needs_full_redraw = true; // 強制完全重繪
                    StatusBar::set_help_text_for_view(
                        &mut self.status_bar_state, 
                        &ViewType::MessageList(selected_topic.topic.clone())
                    );
                    // 立即渲染第二層UI
                    info!("About to call render() for MessageList state");
                    self.render()?;
                    info!("render() call completed for MessageList state");
                } else {
                    tracing::debug!("No topic selected - topics list is empty");
                }
            }
            AppState::MessageList => {
                // TODO: Navigate to payload detail
                self.state = AppState::PayloadDetail;
                self.needs_full_redraw = true; // 強制完全重繪
            }
            _ => {}
        }
        
        Ok(())
    }
    
    async fn refresh_data(&mut self) -> Result<()> {
        match self.state {
            AppState::TopicList => {
                let criteria = self.build_filter_criteria();
                match self.repository.get_topic_stats(&criteria).await {
                    Ok(topics) => {
                        self.topic_list_state.update_topics(topics);
                        self.status_bar_state.total_topics = self.topic_list_state.topics.len();
                        // TODO: Update total messages count
                        self.status_bar_state.last_update = Some(chrono::Utc::now());
                    }
                    Err(e) => {
                        error!("Failed to load topic stats: {}", e);
                    }
                }
            }
            AppState::MessageList => {
                // TODO: Refresh message list data
            }
            _ => {}
        }
        
        Ok(())
    }
    
    fn build_filter_criteria(&self) -> FilterCriteria {
        let mut criteria = FilterCriteria::default();
        
        if !self.filter_state.topic_filter.is_empty() {
            criteria.topic_regex = Some(self.filter_state.topic_filter.clone());
        }
        
        if !self.filter_state.payload_filter.is_empty() {
            criteria.payload_regex = Some(self.filter_state.payload_filter.clone());
        }
        
        // TODO: Parse time filters
        
        criteria
    }
    
    fn render(&mut self) -> Result<()> {
        info!("render() called - current state: {:?}", self.state);
        // Update terminal size if needed
        let (width, height) = size()?;
        if width != self.terminal_width || height != self.terminal_height {
            self.terminal_width = width;
            self.terminal_height = height;
            self.update_visible_rows();
            self.needs_full_redraw = true;
        }
        
        // Only clear screen if full redraw is needed
        if self.needs_full_redraw {
            info!("Full redraw needed - clearing screen");
            let mut stdout = stdout();
            stdout.execute(Clear(ClearType::All))?;
            stdout.execute(MoveTo(0, 0))?;
        }
        
        match self.state {
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
                panic!("render_topic_list_incremental");
            }
        }
        self.needs_full_redraw = false;
        Ok(())
    }
    
    fn render_topic_list_incremental(&mut self) -> Result<()> {
        let filter_rows = 5;  // Filter takes 5 rows (title + connection + topic + payload + time)
        let status_rows = 2;  // Status takes 2 rows
        let available_height = self.terminal_height.saturating_sub(filter_rows + status_rows + 1);
        
        let force_redraw = self.needs_full_redraw;
        // Check if filter state changed
        let filter_changed = self.prev_filter_state.as_ref()
            .map_or(true, |prev| !self.states_equal_filter(prev, &self.filter_state));
        if force_redraw || filter_changed {
            if force_redraw {
                FilterBar::render(&self.filter_state, 0, self.terminal_width)?;
            } else {
                FilterBar::render_incremental(
                    &self.filter_state, 
                    self.prev_filter_state.as_ref(), 
                    0, 
                    self.terminal_width
                )?;
            }
        }
        
        // Check if topic list changed
        let topics_changed = self.prev_topic_list_state.as_ref()
            .map_or(true, |prev| !self.states_equal_topics(prev, &self.topic_list_state));
        
        // 如果資料有變化就強制重繪主題列表
        let force_topic_redraw = topics_changed;
        
        if force_redraw || topics_changed || force_topic_redraw {
            let list_start_row = filter_rows;
            let list_end_row = list_start_row + available_height;
            
            if force_redraw {
                // Full redraw
                TopicListView::render(
                    &self.topic_list_state,
                    list_start_row,
                    list_end_row,
                    self.terminal_width
                )?;
            } else {
                // Incremental update
                TopicListView::render_incremental(
                    &self.topic_list_state,
                    self.prev_topic_list_state.as_ref(),
                    list_start_row,
                    list_end_row,
                    self.terminal_width
                )?;
            }
        }
        
        // Check if status bar changed
        let status_changed = self.prev_status_bar_state.as_ref()
            .map_or(true, |prev| !self.states_equal_status(prev, &self.status_bar_state));
        
        if force_redraw || status_changed {
            let status_start_row = self.terminal_height.saturating_sub(status_rows);
            if force_redraw {
                StatusBar::render(&self.status_bar_state, status_start_row, self.terminal_width)?;
            } else {
                StatusBar::render_incremental(
                    &self.status_bar_state, 
                    self.prev_status_bar_state.as_ref(), 
                    status_start_row, 
                    self.terminal_width
                )?;
            }
        }
        
        // Position cursor for filter editing
        if let Some((x, y)) = FilterBar::get_cursor_position(&self.filter_state, 0) {
            let mut stdout = stdout();
            stdout.execute(MoveTo(x, y))?;
            stdout.execute(Show)?;
        } else {
            let mut stdout = stdout();
            stdout.execute(Hide)?;
        }
        
        // Update previous states for next comparison
        self.prev_filter_state = Some(self.filter_state.clone());
        self.prev_status_bar_state = Some(self.status_bar_state.clone());
        self.prev_topic_list_state = Some(self.topic_list_state.clone());
        
        Ok(())
    }
    
    fn render_topic_list(&mut self) -> Result<()> {
        let filter_rows = 5;  // Filter takes 5 rows (title + connection + topic + payload + time)
        let status_rows = 2;  // Status takes 2 rows
        let available_height = self.terminal_height.saturating_sub(filter_rows + status_rows + 1);
        
        // Render filter bar
        FilterBar::render(&self.filter_state, 0, self.terminal_width)?;
        
        // Render topic list
        let list_start_row = filter_rows;
        let list_end_row = list_start_row + available_height;
        TopicListView::render(
            &self.topic_list_state,
            list_start_row,
            list_end_row,
            self.terminal_width
        )?;
        
        // Render status bar
        let status_start_row = self.terminal_height.saturating_sub(status_rows);
        StatusBar::render(&self.status_bar_state, status_start_row, self.terminal_width)?;
        
        // Position cursor for filter editing
        if let Some((x, y)) = FilterBar::get_cursor_position(&self.filter_state, 0) {
            let mut stdout = stdout();
            stdout.execute(MoveTo(x, y))?;
            stdout.execute(Show)?;
        } else {
            let mut stdout = stdout();
            stdout.execute(Hide)?;
        }
        
        Ok(())
    }
    
    fn render_message_list(&mut self) -> Result<()> {
        info!("render_message_list() called - starting MessageList UI rendering");
        let mut stdout = stdout();
        // Always clear screen when entering message list (second layer)
        stdout.execute(Clear(crossterm::terminal::ClearType::All))?;
        stdout.execute(MoveTo(0, 0))?;
        info!("Screen cleared and cursor moved to 0,0");
        
        let terminal_width: usize = self.terminal_width as usize;
        
        // Render title bar - Topic: xxx
        stdout.queue(MoveTo(0, 0))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        if let Some(selected_topic) = self.topic_list_state.get_selected_topic() {
            let title = format!("┌─ Topic: {} ", selected_topic.topic);
            let padding = terminal_width.saturating_sub(title.len() + 1);
            stdout.queue(Print(&title))?;
            stdout.queue(Print(&"─".repeat(padding)))?;
            stdout.queue(Print("┐"))?;
        } else {
            error!("No topic selected");
        }
        // Render payload filter line 
        stdout.queue(MoveTo(0, 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("│ Payload Filter: [___________] [Apply] [Clear]"))?;
        let padding = terminal_width.saturating_sub(48);
        stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
        stdout.queue(Print("│"))?;
        
        // Render time filter line
        stdout.queue(MoveTo(0, 2))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("│ Time: From [__________] To [__________] [Apply]"))?;
        let padding = terminal_width.saturating_sub(49);
        stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
        stdout.queue(Print("│"))?;
        
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
        let available_height = self.terminal_height.saturating_sub(content_start_row + status_rows + 1);
        
        // Render message list content
        let messages = &self.message_list_state.messages;
        let selected_index = self.message_list_state.selected_index;
        
        for i in 0..available_height {
            let row = content_start_row + i as u16;
            stdout.queue(MoveTo(0, row))?;
            stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
            
            stdout.queue(Print("│ "))?;
            
            if let Some(msg) = messages.get(i as usize) {
                // Highlight selected row
                if i as usize == selected_index {
                    stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
                    stdout.queue(Print("> "))?;
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
        
        // Render bottom border
        let bottom_row = self.terminal_height.saturating_sub(3);
        stdout.queue(MoveTo(0, bottom_row))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let bottom_border = format!("└{:─<width$}┘", "─", width = terminal_width.saturating_sub(2));
        stdout.queue(Print(&bottom_border))?;
        
        // Render status line
        let status_start_row = self.terminal_height.saturating_sub(2);
        stdout.queue(MoveTo(0, status_start_row))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("Status: "))?;
        
        let message_count = self.message_list_state.messages.len();
        let current_page = self.message_list_state.page;
        let total_pages = (self.message_list_state.total_count + self.message_list_state.per_page - 1) / self.message_list_state.per_page;
        
        if let Some(topic) = &self.message_list_state.current_topic {
            stdout.queue(Print(format!("Page {}/{} | {} messages | Topic: {}", 
                                     current_page, total_pages.max(1), message_count, topic)))?;
        }
        // Render help line
        stdout.queue(MoveTo(0, status_start_row + 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("[←][ESC]back [f]ilter [Enter]view [↑↓]navigate [j]son [h]elp"))?;
        
        stdout.flush()?;
        info!("render_message_list() completed - MessageList UI should now be visible");
        self.needs_full_redraw = false; // Reset the redraw flag after successful render
        Ok(())
    }
    
    fn render_payload_detail(&mut self) -> Result<()> {
        // TODO: Implement payload detail rendering
        Ok(())
    }
    
    fn update_visible_rows(&mut self) {
        let filter_rows = 5;
        let status_rows = 2;
        let header_rows = 2; // Header + separator
        let available_height = self.terminal_height
            .saturating_sub(filter_rows + status_rows + header_rows + 1);
        
        self.topic_list_state.set_visible_rows(available_height as usize);
    }
    
    pub fn set_connection_status(&mut self, status: ConnectionStatus) {
        self.status_bar_state.connection_status = status.clone();
        
        // Also update connection status in the UI based on current state
        match status {
            ConnectionStatus::Connected(_) => {
                self.status_bar_state.connection_status = ConnectionStatus::Connected(
                    format!("{}:{}", self.mqtt_host, self.mqtt_port)
                );
            }
            _ => {
                self.status_bar_state.connection_status = status;
            }
        }
    }
    
    pub fn update_connection_status_from_mqtt(&mut self, is_connected: bool) {
        if is_connected {
            self.set_connection_status(ConnectionStatus::Connected(
                format!("{}:{}", self.mqtt_host, self.mqtt_port)
            ));
        } else {
            self.set_connection_status(ConnectionStatus::Disconnected);
        }
    }
    
    // State comparison methods for incremental rendering
    fn states_equal_filter(&self, prev: &FilterState, current: &FilterState) -> bool {
        prev.topic_filter == current.topic_filter &&
        prev.payload_filter == current.payload_filter &&
        prev.start_time == current.start_time &&
        prev.end_time == current.end_time &&
        prev.active_field == current.active_field &&
        prev.is_editing == current.is_editing
    }
    
    fn states_equal_topics(&self, prev: &TopicListState, current: &TopicListState) -> bool {
        if prev.topics.len() != current.topics.len() {
            return false;
        }
        
        if prev.selected_index != current.selected_index {
            return false;
        }
        
        if prev.scroll_offset != current.scroll_offset {
            return false;
        }
        
        // Compare topic contents (simplified comparison)
        for (prev_topic, current_topic) in prev.topics.iter().zip(current.topics.iter()) {
            if prev_topic.topic != current_topic.topic ||
               prev_topic.message_count != current_topic.message_count ||
               prev_topic.last_message_time != current_topic.last_message_time ||
               prev_topic.latest_payload != current_topic.latest_payload {
                return false;
            }
        }
        
        true
    }
    
    fn states_equal_status(&self, prev: &StatusBarState, current: &StatusBarState) -> bool {
        // Compare connection status
        let connection_equal = match (&prev.connection_status, &current.connection_status) {
            (ConnectionStatus::Disconnected, ConnectionStatus::Disconnected) => true,
            (ConnectionStatus::Connecting, ConnectionStatus::Connecting) => true,
            (ConnectionStatus::Connected(a), ConnectionStatus::Connected(b)) => a == b,
            _ => false,
        };
        
        connection_equal &&
        prev.total_topics == current.total_topics &&
        prev.total_messages == current.total_messages &&
        prev.last_update == current.last_update &&
        prev.help_text == current.help_text
    }
    
    #[cfg(windows)]
    fn is_key_just_pressed(&mut self, vk_code: i32) -> bool {
        let is_pressed = unsafe { GetAsyncKeyState(vk_code) & (0x8000u16 as i16) != 0 };
        let was_pressed = self.last_key_state.get(&vk_code).copied().unwrap_or(false);
        
        // 更新按鍵狀態
        self.last_key_state.insert(vk_code, is_pressed);
        
        // 只有從未按下到按下的狀態轉換才返回true（邊沿觸發）
        is_pressed && !was_pressed
    }
}