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

#[derive(Debug, Clone)]
struct KeyRepeatState {
    current_key: Option<crossterm::event::KeyCode>,
    key_pressed_at: Option<Instant>,
    last_repeat_at: Option<Instant>,
    is_repeating: bool,
}

impl Default for KeyRepeatState {
    fn default() -> Self {
        Self {
            current_key: None,
            key_pressed_at: None,
            last_repeat_at: None,
            is_repeating: false,
        }
    }
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
    payload_detail_scroll_offset: usize,
    
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
    
    // Key repeat functionality for MessageList navigation
    key_repeat_state: KeyRepeatState,
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
            payload_detail_scroll_offset: 0,
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
            key_repeat_state: KeyRepeatState::default(),
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
            
            // Handle key repeat for MessageList navigation
            if self.state == AppState::MessageList {
                self.handle_key_repeat().await?;
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
                
                
                // 檢測上下箭頭鍵進行導航 (支援重複按鍵)
                if self.is_key_pressed(0x26) { // VK_UP - use continuous detection
                    if self.handle_key_repeat_start(crossterm::event::KeyCode::Up).await? {
                        break;
                    }
                } else if self.key_repeat_state.current_key == Some(crossterm::event::KeyCode::Up) {
                    self.handle_key_release(crossterm::event::KeyCode::Up);
                }
                
                if self.is_key_pressed(0x28) { // VK_DOWN - use continuous detection  
                    if self.handle_key_repeat_start(crossterm::event::KeyCode::Down).await? {
                        break;
                    }
                } else if self.key_repeat_state.current_key == Some(crossterm::event::KeyCode::Down) {
                    self.handle_key_release(crossterm::event::KeyCode::Down);
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
                
                if self.is_key_pressed(0x21) { // VK_PRIOR (Page Up) - use continuous detection
                    if self.handle_key_repeat_start(crossterm::event::KeyCode::PageUp).await? {
                        break;
                    }
                } else if self.key_repeat_state.current_key == Some(crossterm::event::KeyCode::PageUp) {
                    self.handle_key_release(crossterm::event::KeyCode::PageUp);
                }
                
                if self.is_key_pressed(0x22) { // VK_NEXT (Page Down) - use continuous detection
                    if self.handle_key_repeat_start(crossterm::event::KeyCode::PageDown).await? {
                        break;
                    }
                } else if self.key_repeat_state.current_key == Some(crossterm::event::KeyCode::PageDown) {
                    self.handle_key_release(crossterm::event::KeyCode::PageDown);
                }
                
                if self.is_key_just_pressed(0x0D) { // VK_RETURN (Enter)
                    std::fs::write("debug_key.txt", "ENTER key detected via WinAPI!").ok();
                    tracing::debug!("ENTER key detected via Windows API");
                    if self.handle_event(AppEvent::Enter).await? {
                        break;
                    }
                    self.render()?;
                }
                
                // Tab events are handled by crossterm to avoid double processing
                
                // 檢測 Ctrl+C (需要同時檢測兩個鍵)
                unsafe {
                    if (GetAsyncKeyState(0x11) & (0x8000u16 as i16) != 0) && (GetAsyncKeyState(67) & (0x8000u16 as i16) != 0) {
                        std::fs::write("debug_key.txt", "Ctrl+C detected via WinAPI!").ok();
                        tracing::info!("Ctrl+C detected via Windows API");
                        break;
                    }
                }
            }
            
            // Handle crossterm events (for character input and resize)
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key_event) => {
                        tracing::debug!("Raw key event detected: {:?}", key_event);
                        // 處理字符輸入事件和Backspace事件
                        let app_event = AppEvent::from(key_event);
                        tracing::debug!("Converted to AppEvent: {:?}", app_event);
                        if matches!(app_event, AppEvent::Input(c) if c != '\0') || matches!(app_event, AppEvent::Backspace) || matches!(app_event, AppEvent::Tab) {
                            tracing::debug!("Input/Backspace/Tab event detected: {:?}", app_event);
                            if self.handle_event(app_event).await? {
                                break;
                            }
                            self.render()?;
                        } else {
                            tracing::debug!("Non-input event ignored: {:?}", app_event);
                        }
                    }
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
                
                
                // 檢測上下箭頭鍵進行導航 (支援重複按鍵)
                if self.is_key_pressed(0x26) { // VK_UP - use continuous detection
                    if self.handle_key_repeat_start(crossterm::event::KeyCode::Up).await? {
                        break;
                    }
                } else if self.key_repeat_state.current_key == Some(crossterm::event::KeyCode::Up) {
                    self.handle_key_release(crossterm::event::KeyCode::Up);
                }
                
                if self.is_key_pressed(0x28) { // VK_DOWN - use continuous detection  
                    if self.handle_key_repeat_start(crossterm::event::KeyCode::Down).await? {
                        break;
                    }
                } else if self.key_repeat_state.current_key == Some(crossterm::event::KeyCode::Down) {
                    self.handle_key_release(crossterm::event::KeyCode::Down);
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
                
                if self.is_key_pressed(0x21) { // VK_PRIOR (Page Up) - use continuous detection
                    if self.handle_key_repeat_start(crossterm::event::KeyCode::PageUp).await? {
                        break;
                    }
                } else if self.key_repeat_state.current_key == Some(crossterm::event::KeyCode::PageUp) {
                    self.handle_key_release(crossterm::event::KeyCode::PageUp);
                }
                
                if self.is_key_pressed(0x22) { // VK_NEXT (Page Down) - use continuous detection
                    if self.handle_key_repeat_start(crossterm::event::KeyCode::PageDown).await? {
                        break;
                    }
                } else if self.key_repeat_state.current_key == Some(crossterm::event::KeyCode::PageDown) {
                    self.handle_key_release(crossterm::event::KeyCode::PageDown);
                }
                
                if self.is_key_just_pressed(0x0D) { // VK_RETURN (Enter)
                    std::fs::write("debug_key.txt", "ENTER key detected via WinAPI!").ok();
                    tracing::debug!("ENTER key detected via Windows API");
                    if self.handle_event(AppEvent::Enter).await? {
                        break;
                    }
                    self.render()?;
                }
                
                // Tab events are handled by crossterm to avoid double processing
                
                // 檢測 Ctrl+C (需要同時檢測兩個鍵)
                unsafe {
                    if (GetAsyncKeyState(0x11) & (0x8000u16 as i16) != 0) && (GetAsyncKeyState(67) & (0x8000u16 as i16) != 0) {
                        std::fs::write("debug_key.txt", "Ctrl+C detected via WinAPI!").ok();
                        tracing::info!("Ctrl+C detected via Windows API");
                        break;
                    }
                }
            }
            
            // Handle crossterm events (for character input and resize)
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key_event) => {
                        tracing::debug!("Raw key event detected: {:?}", key_event);
                        // 處理字符輸入事件和Backspace事件
                        let app_event = AppEvent::from(key_event);
                        tracing::debug!("Converted to AppEvent: {:?}", app_event);
                        if matches!(app_event, AppEvent::Input(c) if c != '\0') || matches!(app_event, AppEvent::Backspace) || matches!(app_event, AppEvent::Tab) {
                            tracing::debug!("Input/Backspace/Tab event detected: {:?}", app_event);
                            if self.handle_event(app_event).await? {
                                break;
                            }
                            self.render()?;
                        } else {
                            tracing::debug!("Non-input event ignored: {:?}", app_event);
                        }
                    }
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
            AppEvent::Tab => {
                tracing::debug!("Tab pressed in topic list - switching filter focus");
                self.filter_state.next_field();
                self.filter_state.is_editing = true;
                tracing::debug!("Auto-started editing after Tab in topic list");
            },
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
        // 如果正在編輯模式，處理輸入
        if self.message_list_state.is_editing {
            return self.handle_message_list_filter_input(event).await;
        }
        
        match event {
            AppEvent::Tab => {
                tracing::debug!("Tab pressed - switching focus");
                self.message_list_state.next_focus();
                // Tab到filter欄位時自動進入編輯模式
                match self.message_list_state.get_focus() {
                    crate::ui::views::message_list::FocusTarget::PayloadFilter |
                    crate::ui::views::message_list::FocusTarget::TimeFilterFrom |
                    crate::ui::views::message_list::FocusTarget::TimeFilterTo => {
                        self.message_list_state.start_editing();
                        tracing::debug!("Auto-started editing after Tab");
                    }
                    crate::ui::views::message_list::FocusTarget::MessageList => {
                        self.message_list_state.stop_editing();
                        tracing::debug!("Stopped editing when Tab to message list");
                    }
                }
            }
            AppEvent::Enter => {
                // Enter鍵的處理
                match self.message_list_state.get_focus() {
                    crate::ui::views::message_list::FocusTarget::PayloadFilter |
                    crate::ui::views::message_list::FocusTarget::TimeFilterFrom |
                    crate::ui::views::message_list::FocusTarget::TimeFilterTo => {
                        // 在filter欄位按Enter應用過濾器
                        self.message_list_state.stop_editing();
                        self.apply_message_list_filters().await?;
                    }
                    crate::ui::views::message_list::FocusTarget::MessageList => {
                        // 導航到payload detail
                        if let Some(_msg) = self.message_list_state.get_selected_message() {
                            self.state = AppState::PayloadDetail;
                            self.payload_detail_scroll_offset = 0; // 重置滾動偏移
                            self.needs_full_redraw = true;
                        }
                    }
                }
            }
            AppEvent::NavigateUp => {
                if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::MessageList) {
                    tracing::debug!("Navigate up in message list");
                    let old_page = self.message_list_state.page;
                    self.message_list_state.move_up_with_pagination(&self.repository).await?;
                    if old_page != self.message_list_state.page {
                        self.needs_full_redraw = true; // Force redraw after page change
                    }
                }
            }
            AppEvent::NavigateDown => {
                if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::MessageList) {
                    tracing::debug!("Navigate down in message list");
                    let old_page = self.message_list_state.page;
                    self.message_list_state.move_down_with_pagination(&self.repository).await?;
                    if old_page != self.message_list_state.page {
                        self.needs_full_redraw = true; // Force redraw after page change
                    }
                }
            }
            AppEvent::PageUp => {
                if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::MessageList) {
                    self.message_list_state.page_up(&self.repository).await?;
                }
            }
            AppEvent::PageDown => {
                if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::MessageList) {
                    self.message_list_state.page_down(&self.repository).await?;
                }
            }
            AppEvent::NavigateLeft => {
                tracing::debug!("Navigate left from message list - returning to topic list");
                self.navigate_back()?;
            }
            AppEvent::NavigateRight => {
                if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::MessageList) {
                    if let Some(_msg) = self.message_list_state.get_selected_message() {
                        self.state = AppState::PayloadDetail;
                        self.payload_detail_scroll_offset = 0; // 重置滾動偏移
                        self.needs_full_redraw = true;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    
    async fn handle_message_list_filter_input(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::Tab => {
                tracing::debug!("Tab pressed in editing mode - switching focus");
                self.message_list_state.next_focus();
                // 切換到其他filter欄位時繼續編輯模式，切換到訊息列表時停止編輯
                match self.message_list_state.get_focus() {
                    crate::ui::views::message_list::FocusTarget::MessageList => {
                        self.message_list_state.stop_editing();
                        tracing::debug!("Stopped editing when Tab to message list");
                    }
                    _ => {
                        tracing::debug!("Continue editing in new filter field");
                    }
                }
            }
            AppEvent::Input(c) if c != '\0' => {
                if let Some(input) = self.message_list_state.get_active_input_mut() {
                    input.push(c);
                    tracing::debug!("Added character '{}' to filter input", c);
                }
            }
            AppEvent::Backspace => {
                if let Some(input) = self.message_list_state.get_active_input_mut() {
                    input.pop();
                    tracing::debug!("Removed character from filter input");
                }
            }
            AppEvent::Enter => {
                tracing::debug!("Filter input submitted");
                self.message_list_state.stop_editing();
                // 應用過濾器並重新載入訊息
                self.apply_message_list_filters().await?;
            }
            AppEvent::Escape => {
                tracing::debug!("Filter input cancelled");
                self.message_list_state.stop_editing();
            }
            _ => {}
        }
        Ok(())
    }
    
    async fn apply_message_list_filters(&mut self) -> Result<()> {
        // 更新message list的filter criteria
        if !self.message_list_state.payload_filter_input.is_empty() {
            self.message_list_state.filter.payload_regex = Some(self.message_list_state.payload_filter_input.clone());
        } else {
            self.message_list_state.filter.payload_regex = None;
        }
        
        // 處理時間過濾器
        if !self.message_list_state.time_from_input.is_empty() {
            if let Ok(parsed_time) = chrono::NaiveDateTime::parse_from_str(&self.message_list_state.time_from_input, "%Y-%m-%d %H:%M:%S") {
                self.message_list_state.filter.start_time = Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(parsed_time, chrono::Utc));
            }
        }
        
        if !self.message_list_state.time_to_input.is_empty() {
            if let Ok(parsed_time) = chrono::NaiveDateTime::parse_from_str(&self.message_list_state.time_to_input, "%Y-%m-%d %H:%M:%S") {
                self.message_list_state.filter.end_time = Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(parsed_time, chrono::Utc));
            }
        }
        
        // 重設到第一頁並重新載入訊息
        self.message_list_state.page = 1;
        self.message_list_state.selected_index = 0;
        self.message_list_state.load_messages(&self.repository).await?;
        
        tracing::debug!("Applied message list filters and reloaded messages");
        Ok(())
    }

    async fn handle_payload_detail_event(&mut self, event: AppEvent) -> Result<()> {
        match event {
            AppEvent::NavigateLeft => {
                tracing::debug!("Navigate left from payload detail - returning to message list");
                self.navigate_back()?;
            }
            AppEvent::NavigateUp => {
                if self.payload_detail_scroll_offset > 0 {
                    self.payload_detail_scroll_offset -= 1;
                }
            }
            AppEvent::NavigateDown => {
                // We'll check max scroll in render function
                self.payload_detail_scroll_offset += 1;
            }
            AppEvent::PageUp => {
                let page_size = self.get_payload_detail_page_size();
                self.payload_detail_scroll_offset = self.payload_detail_scroll_offset.saturating_sub(page_size);
            }
            AppEvent::PageDown => {
                let page_size = self.get_payload_detail_page_size();
                self.payload_detail_scroll_offset += page_size;
            }
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
            AppEvent::Tab => {
                tracing::debug!("Tab pressed in filter edit mode - switching to next field");
                self.filter_state.next_field();
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
                self.payload_detail_scroll_offset = 0; // 重置滾動偏移
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
                    // Update per_page based on current terminal size
                    self.message_list_state.update_per_page(self.terminal_height);
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
                self.payload_detail_scroll_offset = 0; // 重置滾動偏移
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
        
    pub fn update_visible_rows(&mut self) {
        let filter_rows = 5;
        let status_rows = 2;
        let header_rows = 2; // Header + separator
        let available_height = self.terminal_height
            .saturating_sub(filter_rows + status_rows + header_rows + 1);
        
        self.topic_list_state.set_visible_rows(available_height as usize);
        
        // Also update MessageList per_page when terminal size changes
        self.message_list_state.update_per_page(self.terminal_height);
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
    
    #[cfg(windows)]
    fn is_key_pressed(&self, vk_code: i32) -> bool {
        unsafe { GetAsyncKeyState(vk_code) & (0x8000u16 as i16) != 0 }
    }
    
    // Key repeat handling methods
    async fn handle_key_repeat_start(&mut self, key: crossterm::event::KeyCode) -> Result<bool> {
        let now = Instant::now();
        
        // If this is a new key press
        if self.key_repeat_state.current_key != Some(key) {
            self.key_repeat_state.current_key = Some(key);
            self.key_repeat_state.key_pressed_at = Some(now);
            self.key_repeat_state.last_repeat_at = None;
            self.key_repeat_state.is_repeating = false;
            
            // Handle the initial key press immediately
            let event = match key {
                crossterm::event::KeyCode::Up => AppEvent::NavigateUp,
                crossterm::event::KeyCode::Down => AppEvent::NavigateDown,
                crossterm::event::KeyCode::PageUp => AppEvent::PageUp,
                crossterm::event::KeyCode::PageDown => AppEvent::PageDown,
                _ => return Ok(false),
            };
            
            if self.handle_event(event).await? {
                return Ok(true);
            }
            self.render()?;
        }
        
        Ok(false)
    }
    
    fn handle_key_release(&mut self, key: crossterm::event::KeyCode) {
        if self.key_repeat_state.current_key == Some(key) {
            self.key_repeat_state.current_key = None;
            self.key_repeat_state.key_pressed_at = None;
            self.key_repeat_state.last_repeat_at = None;
            self.key_repeat_state.is_repeating = false;
        }
    }
    
    async fn handle_key_repeat(&mut self) -> Result<()> {
        let now = Instant::now();
        
        if let (Some(key), Some(pressed_at)) = (self.key_repeat_state.current_key, self.key_repeat_state.key_pressed_at) {
            // Check if we should start repeating (after 0.5 seconds)
            if !self.key_repeat_state.is_repeating && now.duration_since(pressed_at) >= Duration::from_millis(500) {
                self.key_repeat_state.is_repeating = true;
                self.key_repeat_state.last_repeat_at = Some(now);
            }
            
            // If we're in repeat mode, check if it's time for the next repeat (every 0.2 seconds)
            if self.key_repeat_state.is_repeating {
                let last_repeat = self.key_repeat_state.last_repeat_at.unwrap_or(pressed_at);
                if now.duration_since(last_repeat) >= Duration::from_millis(50) {
                    let event = match key {
                        crossterm::event::KeyCode::Up => AppEvent::NavigateUp,
                        crossterm::event::KeyCode::Down => AppEvent::NavigateDown,
                        crossterm::event::KeyCode::PageUp => AppEvent::PageUp,
                        crossterm::event::KeyCode::PageDown => AppEvent::PageDown,
                        _ => return Ok(()),
                    };
                    
                    self.handle_event(event).await?;
                    self.render()?;
                    self.key_repeat_state.last_repeat_at = Some(now);
                }
            }
        }
        
        Ok(())
    }
    
    // Public getter methods for render module
    pub fn get_state(&self) -> AppState {
        self.state
    }
    
    pub fn get_terminal_size(&self) -> (u16, u16) {
        (self.terminal_width, self.terminal_height)
    }
    
    pub fn set_terminal_size(&mut self, width: u16, height: u16) {
        self.terminal_width = width;
        self.terminal_height = height;
    }
    
    pub fn needs_full_redraw(&self) -> bool {
        self.needs_full_redraw
    }
    
    pub fn set_needs_full_redraw(&mut self, value: bool) {
        self.needs_full_redraw = value;
    }
    
    pub fn get_filter_state(&self) -> &FilterState {
        &self.filter_state
    }
    
    pub fn get_prev_filter_state(&self) -> Option<&FilterState> {
        self.prev_filter_state.as_ref()
    }
    
    pub fn get_topic_list_state(&self) -> &TopicListState {
        &self.topic_list_state
    }
    
    pub fn get_prev_topic_list_state(&self) -> Option<&TopicListState> {
        self.prev_topic_list_state.as_ref()
    }
    
    pub fn get_status_bar_state(&self) -> &StatusBarState {
        &self.status_bar_state
    }
    
    pub fn get_prev_status_bar_state(&self) -> Option<&StatusBarState> {
        self.prev_status_bar_state.as_ref()
    }
    
    pub fn has_filter_state_changed(&self) -> bool {
        self.prev_filter_state.as_ref()
            .map_or(true, |prev| !self.states_equal_filter(prev, &self.filter_state))
    }
    
    pub fn has_topic_list_state_changed(&self) -> bool {
        self.prev_topic_list_state.as_ref()
            .map_or(true, |prev| !self.states_equal_topics(prev, &self.topic_list_state))
    }
    
    pub fn has_status_bar_state_changed(&self) -> bool {
        self.prev_status_bar_state.as_ref()
            .map_or(true, |prev| !self.states_equal_status(prev, &self.status_bar_state))
    }
    
    pub fn update_prev_states(&mut self) {
        self.prev_filter_state = Some(self.filter_state.clone());
        self.prev_status_bar_state = Some(self.status_bar_state.clone());
        self.prev_topic_list_state = Some(self.topic_list_state.clone());
    }
    
    pub fn get_selected_topic(&self) -> Option<&crate::db::TopicStat> {
        self.topic_list_state.get_selected_topic()
    }
    
    pub fn get_selected_message(&self) -> Option<&crate::db::Message> {
        self.message_list_state.get_selected_message()
    }
    
    pub fn get_message_list_cursor_position(&self) -> Option<(u16, u16)> {
        self.message_list_state.get_cursor_position()
    }
    
    pub fn get_message_list_state(&self) -> &MessageListState {
        &self.message_list_state
    }
    
    pub fn get_payload_detail_scroll_offset(&self) -> usize {
        self.payload_detail_scroll_offset
    }
    
    pub fn set_payload_detail_scroll_offset(&mut self, offset: usize) {
        self.payload_detail_scroll_offset = offset;
    }
    
    pub fn format_payload_content(&self, payload: &str) -> Vec<String> {
        // Try to parse payload as JSON for formatting
        match serde_json::from_str::<serde_json::Value>(payload) {
            Ok(json_value) => {
                // Format JSON with proper indentation
                match serde_json::to_string_pretty(&json_value) {
                    Ok(pretty_json) => pretty_json.lines().map(|line| line.to_string()).collect(),
                    Err(_) => payload.lines().map(|line| line.to_string()).collect(),
                }
            }
            Err(_) => {
                // Not JSON, display as plain text
                payload.lines().map(|line| line.to_string()).collect()
            }
        }
    }
    
}