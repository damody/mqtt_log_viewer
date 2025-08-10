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
                
                if self.is_key_just_pressed(0x21) { // VK_PRIOR (Page Up)
                    std::fs::write("debug_key.txt", "PAGE UP key detected via WinAPI!").ok();
                    tracing::debug!("PAGE UP key detected via Windows API");
                    if self.handle_event(AppEvent::PageUp).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x22) { // VK_NEXT (Page Down)
                    std::fs::write("debug_key.txt", "PAGE DOWN key detected via WinAPI!").ok();
                    tracing::debug!("PAGE DOWN key detected via Windows API");
                    if self.handle_event(AppEvent::PageDown).await? {
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
                
                if self.is_key_just_pressed(0x21) { // VK_PRIOR (Page Up)
                    std::fs::write("debug_key.txt", "PAGE UP key detected via WinAPI!").ok();
                    tracing::debug!("PAGE UP key detected via Windows API");
                    if self.handle_event(AppEvent::PageUp).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x22) { // VK_NEXT (Page Down)
                    std::fs::write("debug_key.txt", "PAGE DOWN key detected via WinAPI!").ok();
                    tracing::debug!("PAGE DOWN key detected via Windows API");
                    if self.handle_event(AppEvent::PageDown).await? {
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
                    self.message_list_state.move_up();
                }
            }
            AppEvent::NavigateDown => {
                if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::MessageList) {
                    tracing::debug!("Navigate down in message list");
                    self.message_list_state.move_down();
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
        // Render payload filter line with focus indicator and content
        stdout.queue(MoveTo(0, 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let payload_filter_text = if !self.message_list_state.payload_filter_input.is_empty() {
            &self.message_list_state.payload_filter_input
        } else if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::PayloadFilter) {
            if self.message_list_state.is_editing {
                "<<<EDITING>>>"
            } else {
                "<<<FOCUSED>>>"
            }
        } else {
            "___________"
        };
        
        if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::PayloadFilter) {
            stdout.queue(SetForegroundColor(crossterm::style::Color::Cyan))?;
        }
        stdout.queue(Print(&format!("│ Payload Filter: [{}] [Apply] [Clear]", payload_filter_text)))?;
        if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::PayloadFilter) {
            stdout.queue(ResetColor)?;
        }
        let line_len = 23 + payload_filter_text.len() + 17; // "│ Payload Filter: [" + content + "] [Apply] [Clear]"
        let padding = terminal_width.saturating_sub(line_len + 1);
        stdout.queue(Print(&format!("{:<width$}", "", width = padding)))?;
        stdout.queue(Print("│"))?;
        
        // Render time filter line with focus indicators and content
        stdout.queue(MoveTo(0, 2))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let focus = self.message_list_state.get_focus();
        
        let from_text = if !self.message_list_state.time_from_input.is_empty() {
            &self.message_list_state.time_from_input
        } else if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterFrom) {
            if self.message_list_state.is_editing {
                "<<<EDITING>>>"
            } else {
                "<<<FOCUS>>>"
            }
        } else {
            "__________"
        };
        
        let to_text = if !self.message_list_state.time_to_input.is_empty() {
            &self.message_list_state.time_to_input
        } else if matches!(focus, crate::ui::views::message_list::FocusTarget::TimeFilterTo) {
            if self.message_list_state.is_editing {
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
        
        let line_len = 16 + from_text.len() + 6 + to_text.len() + 9; // "│ Time: From [" + from + "] To [" + to + "] [Apply]"
        let padding = terminal_width.saturating_sub(line_len + 1);
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
                // Highlight selected row with focus indication
                if i as usize == selected_index {
                    let is_message_list_focused = matches!(self.message_list_state.get_focus(), 
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
        stdout.queue(Print("[←][ESC]back [Tab]focus [Enter]view [↑↓]navigate [PgUp/PgDn]page [F1]help"))?;
        
        // Position cursor for input if editing
        if let Some((col, row)) = self.message_list_state.get_cursor_position() {
            stdout.queue(MoveTo(col, row))?;
        }
        
        stdout.flush()?;
        info!("render_message_list() completed - MessageList UI should now be visible");
        self.needs_full_redraw = false; // Reset the redraw flag after successful render
        Ok(())
    }
    
    fn render_payload_detail(&mut self) -> Result<()> {
        info!("render_payload_detail() called - starting PayloadDetail UI rendering");
        let mut stdout = stdout();
        
        // Always clear screen when entering payload detail (third layer)
        stdout.execute(Clear(crossterm::terminal::ClearType::All))?;
        stdout.execute(MoveTo(0, 0))?;
        info!("Screen cleared and cursor moved to 0,0");
        
        let terminal_width: usize = self.terminal_width as usize;
        
        // Get selected message
        let selected_message = match self.message_list_state.get_selected_message() {
            Some(msg) => msg,
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
        
        // Render separator line
        stdout.queue(MoveTo(0, 3))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let separator = format!("├{:─<width$}┤", "─ Payload ", width = terminal_width.saturating_sub(2));
        stdout.queue(Print(&separator))?;
        
        // Calculate content area
        let content_start_row = 4;
        let status_rows = 2;
        let available_height = self.terminal_height.saturating_sub(content_start_row + status_rows + 1);
        
        // Try to parse payload as JSON for formatting
        let payload_lines: Vec<String> = match serde_json::from_str::<serde_json::Value>(&selected_message.payload) {
            Ok(json_value) => {
                // Format JSON with proper indentation
                match serde_json::to_string_pretty(&json_value) {
                    Ok(pretty_json) => pretty_json.lines().map(|line| line.to_string()).collect(),
                    Err(_) => selected_message.payload.lines().map(|line| line.to_string()).collect(),
                }
            }
            Err(_) => {
                // Not JSON, display as plain text
                selected_message.payload.lines().map(|line| line.to_string()).collect()
            }
        };
        
        // Ensure scroll offset doesn't exceed content
        let max_scroll = payload_lines.len().saturating_sub(available_height as usize);
        if self.payload_detail_scroll_offset > max_scroll {
            self.payload_detail_scroll_offset = max_scroll;
        }
        
        // Render payload content with scroll offset
        for i in 0..available_height {
            let row = content_start_row + i as u16;
            stdout.queue(MoveTo(0, row))?;
            stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
            
            stdout.queue(Print("│ "))?;
            
            let line_index = (i as usize) + self.payload_detail_scroll_offset;
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
        
        // Render bottom border
        let bottom_row = content_start_row + available_height as u16;
        stdout.queue(MoveTo(0, bottom_row))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let bottom_border = format!("└{:─<width$}┘", "", width = terminal_width.saturating_sub(2));
        stdout.queue(Print(&bottom_border))?;
        
        // Render status lines
        let status_start_row = self.terminal_height.saturating_sub(status_rows);
        
        // Render info line
        stdout.queue(MoveTo(0, status_start_row))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        let payload_size = selected_message.payload.len();
        let line_count = payload_lines.len();
        let scroll_info = if line_count > available_height as usize {
            format!(" | Lines: {}-{}/{}", 
                    self.payload_detail_scroll_offset + 1,
                    std::cmp::min(self.payload_detail_scroll_offset + available_height as usize, line_count),
                    line_count)
        } else {
            format!(" | Lines: {}", line_count)
        };
        stdout.queue(Print(format!("Payload: {} bytes{} | Topic: {}", 
                                 payload_size, scroll_info, selected_message.topic)))?;
        
        // Render help line
        stdout.queue(MoveTo(0, status_start_row + 1))?;
        stdout.queue(Clear(crossterm::terminal::ClearType::CurrentLine))?;
        stdout.queue(Print("[←][ESC]back [F2]json-depth [c]opy [↑↓]scroll [PgUp/PgDn]page [F1]help"))?;
        
        stdout.flush()?;
        info!("render_payload_detail() completed - PayloadDetail UI should now be visible");
        self.needs_full_redraw = false; // Reset the redraw flag after successful render
        Ok(())
    }
    
    fn get_payload_detail_page_size(&self) -> usize {
        let content_start_row = 4;
        let status_rows = 2;
        let available_height = self.terminal_height.saturating_sub(content_start_row + status_rows + 1);
        available_height as usize
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
}