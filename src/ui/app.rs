use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Quit,
    Refresh,
    NavigateUp,
    NavigateDown,
    NavigateLeft,
    NavigateRight,
    Enter,
    Escape,
    Filter,
    JsonToggle,
    PageUp,
    PageDown,
    Home,
    End,
    Help,
    Copy,
    Tab,
    Input(char),
    Backspace,
    Delete,
    Paste(String),
    Space,  // 空白鍵事件
    QuickFilter(usize),  // F1-F5快速過濾器
}

impl From<KeyEvent> for AppEvent {
    fn from(key_event: KeyEvent) -> Self {
        // 只處理按鍵按下事件，忽略按鍵釋放事件
        if key_event.kind != KeyEventKind::Press {
            return AppEvent::Input('\0'); // 忽略非按下事件
        }
        
        match key_event.code {
            KeyCode::F(1) => AppEvent::QuickFilter(0),  // F1-F5快速過濾器
            KeyCode::F(2) => AppEvent::QuickFilter(1),
            KeyCode::F(3) => AppEvent::QuickFilter(2),
            KeyCode::F(4) => AppEvent::QuickFilter(3),
            KeyCode::F(5) => AppEvent::QuickFilter(4),
            KeyCode::F(6) => AppEvent::Refresh,
            KeyCode::Char('/') => AppEvent::Filter,
            KeyCode::F(7) => AppEvent::JsonToggle,
            KeyCode::F(8) => AppEvent::Help,
            KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+C 會根據當前狀態決定行為：
                // - TopicList: 退出程式
                // - MessageList/PayloadDetail: 複製 payload
                AppEvent::Copy
            },
            KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::ALT) => {
                println!("Alt+C detected, generating Copy event");
                tracing::info!("Alt+C key combination detected, generating Copy event");
                AppEvent::Copy
            },
            KeyCode::Char('v') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                // 這裡我們先返回一個空的Paste事件，實際的剪貼簿內容需要在app.rs中獲取
                AppEvent::Paste(String::new())
            },
            KeyCode::Tab => AppEvent::Tab,
            KeyCode::Char(' ') => AppEvent::Space,  // 空白鍵特殊處理
            KeyCode::Char(c) => AppEvent::Input(c),
            KeyCode::Up => AppEvent::NavigateUp,
            KeyCode::Down => AppEvent::NavigateDown,
            KeyCode::Left => AppEvent::NavigateLeft,
            KeyCode::Right => AppEvent::NavigateRight,
            KeyCode::Enter => AppEvent::Enter,
            KeyCode::Esc => AppEvent::Escape,
            KeyCode::PageUp => AppEvent::PageUp,
            KeyCode::PageDown => AppEvent::PageDown,
            KeyCode::Home => {
                tracing::debug!("Home key detected in event conversion");
                AppEvent::Home
            },
            KeyCode::End => {
                tracing::debug!("End key detected in event conversion");
                AppEvent::End
            },
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Delete => AppEvent::Delete,
            _ => AppEvent::Input('\0'), // Ignore other keys
        }
    }
}
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags, KeyEventKind, KeyModifiers},
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PayloadDetailSelection {
    Topic,
    Payload,
    FormattedJson,
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
    payload_detail_selection: PayloadDetailSelection,
    
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

    // Clipboard context (needs to be kept alive for Wayland)
    clipboard_ctx: Option<copypasta::ClipboardContext>,
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
            payload_detail_selection: PayloadDetailSelection::Payload, // 預設選擇payload
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
            clipboard_ctx: {
                use copypasta::ClipboardProvider;
                match copypasta::ClipboardContext::new() {
                    Ok(ctx) => {
                        tracing::info!("Clipboard context initialized successfully");
                        Some(ctx)
                    }
                    Err(e) => {
                        tracing::warn!("Failed to initialize clipboard context: {}", e);
                        None
                    }
                }
            },
        };

        // Set initial help text
        StatusBar::set_help_text_for_view(&mut app.status_bar_state, &ViewType::TopicList);
        
        // 初始化快速過濾器狀態
        if app.config.quick_filters.enabled {
            app.status_bar_state.quick_filter_states = app.config.quick_filters.filters
                .iter()
                .take(5) // 只取前5個
                .map(|f| (f.name.clone(), f.color.clone(), false))
                .collect();
        }
        
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

        // Try to enable keyboard enhancement flags for better key detection
        // This may fail on some terminals/environments, so we ignore errors
        match stdout.queue(PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        )) {
            Ok(_) => {
                let _ = stdout.flush();
                tracing::debug!("Keyboard enhancement flags enabled");
            }
            Err(e) => {
                tracing::warn!("Failed to enable keyboard enhancement flags (continuing anyway): {}", e);
            }
        }

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

        // Try to enable keyboard enhancement flags for better key detection
        // This may fail on some terminals/environments, so we ignore errors
        match stdout.queue(PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        )) {
            Ok(_) => {
                let _ = stdout.flush();
                tracing::debug!("Keyboard enhancement flags enabled");
            }
            Err(e) => {
                tracing::warn!("Failed to enable keyboard enhancement flags (continuing anyway): {}", e);
            }
        }

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
            
            // 每0.25秒刷新資料 (第一層和第二層)
            let now = Instant::now();
            if now.duration_since(last_refresh) >= self.refresh_interval 
                && (self.state == AppState::TopicList || self.state == AppState::MessageList) {
                self.refresh_data().await?;
                // 不強制完全重繪，讓增量渲染決定
                self.render()?;
                last_refresh = now;
            }
            
            // Handle key repeat for MessageList navigation
            if self.state == AppState::MessageList {
                self.handle_key_repeat().await?;
            }
            
            // Windows API 直接按鍵檢測
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
                    tracing::debug!("LEFT key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateLeft).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x27) { // VK_RIGHT
                    tracing::debug!("RIGHT key detected via Windows API");
                    if self.handle_event(AppEvent::NavigateRight).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x24) { // VK_HOME
                    tracing::debug!("HOME key detected via Windows API");
                    if self.handle_event(AppEvent::Home).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x23) { // VK_END
                    tracing::debug!("END key detected via Windows API");
                    if self.handle_event(AppEvent::End).await? {
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
                
                if self.is_key_just_pressed(0x2E) { // VK_DELETE
                    tracing::info!("DELETE key detected via Windows API");
                    if self.handle_event(AppEvent::Delete).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x0D) { // VK_RETURN (Enter)
                    tracing::debug!("ENTER key detected via Windows API");
                    if self.handle_event(AppEvent::Enter).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x20) { // VK_SPACE (Space)
                    let field_debug = format!("SPACE key detected! Active field: {:?}, Is editing: {}", 
                                            self.filter_state.active_field, self.filter_state.is_editing);
                    tracing::debug!("SPACE key detected via Windows API - {}", field_debug);
                    if self.handle_event(AppEvent::Space).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x09) { // VK_TAB
                    let field_debug = format!("TAB key detected! Current field: {:?} -> ", self.filter_state.active_field);
                    std::fs::write("debug_key.txt", &field_debug).ok();
                    tracing::info!("TAB key detected via Windows API - {}", field_debug);
                    if self.handle_event(AppEvent::Tab).await? {
                        break;
                    }
                    self.render()?;
                }
                
                // 檢測 F1-F5 快速過濾器（在MessageList和TopicList狀態下）
                if self.state == AppState::MessageList || self.state == AppState::TopicList {
                    if self.is_key_just_pressed(0x70) { // VK_F1
                        tracing::info!("F1 key detected via Windows API - Quick Filter 0");
                        if self.handle_event(AppEvent::QuickFilter(0)).await? {
                            break;
                        }
                        self.render()?;
                    }
                    if self.is_key_just_pressed(0x71) { // VK_F2
                        tracing::info!("F2 key detected via Windows API - Quick Filter 1");
                        if self.handle_event(AppEvent::QuickFilter(1)).await? {
                            break;
                        }
                        self.render()?;
                    }
                    if self.is_key_just_pressed(0x72) { // VK_F3
                        tracing::info!("F3 key detected via Windows API - Quick Filter 2");
                        if self.handle_event(AppEvent::QuickFilter(2)).await? {
                            break;
                        }
                        self.render()?;
                    }
                    if self.is_key_just_pressed(0x73) { // VK_F4
                        tracing::info!("F4 key detected via Windows API - Quick Filter 3");
                        if self.handle_event(AppEvent::QuickFilter(3)).await? {
                            break;
                        }
                        self.render()?;
                    }
                    if self.is_key_just_pressed(0x74) { // VK_F5
                        tracing::info!("F5 key detected via Windows API - Quick Filter 4");
                        if self.handle_event(AppEvent::QuickFilter(4)).await? {
                            break;
                        }
                        self.render()?;
                    }
                }
                
                // 移除 Ctrl+C 檢測，避免誤觸關閉程式
                // 使用者可以使用 ESC 或 'q' 鍵來退出程式
            }

            // Handle crossterm events
            // On Linux/non-Windows, handle all keyboard events via crossterm
            // On Windows, only handle character input and Backspace via crossterm (navigation keys use WinAPI)
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key_event) => {
                        tracing::debug!("Raw key event detected: {:?}", key_event);
                        let app_event = AppEvent::from(key_event);
                        tracing::debug!("Converted to AppEvent: {:?}", app_event);

                        #[cfg(not(windows))]
                        {
                            // On Linux/non-Windows: handle all key events via crossterm
                            if !matches!(app_event, AppEvent::Input('\0')) {
                                tracing::debug!("Processing event: {:?}", app_event);
                                if self.handle_event(app_event).await? {
                                    break;
                                }
                                self.render()?;
                            } else {
                                tracing::debug!("Ignored null input event");
                            }
                        }

                        #[cfg(windows)]
                        {
                            // On Windows: only handle character input, Backspace, and Copy via crossterm
                            // (navigation keys are handled by WinAPI above)
                            if matches!(app_event, AppEvent::Input(c) if c != '\0') || matches!(app_event, AppEvent::Backspace) || matches!(app_event, AppEvent::Copy) {
                                tracing::debug!("Input/Backspace/Copy event detected: {:?}", app_event);
                                if self.handle_event(app_event).await? {
                                    break;
                                }
                                self.render()?;
                            } else {
                                tracing::debug!("Non-input event ignored (handled by WinAPI): {:?}", app_event);
                            }
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
            // 每0.25秒刷新資料 (第一層和第二層)
            let now = Instant::now();
            if now.duration_since(last_refresh) >= self.refresh_interval 
                && (self.state == AppState::TopicList || self.state == AppState::MessageList) {
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
                
                if self.is_key_just_pressed(0x24) { // VK_HOME
                    std::fs::write("debug_key.txt", "HOME key detected via WinAPI!").ok();
                    tracing::debug!("HOME key detected via Windows API");
                    if self.handle_event(AppEvent::Home).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x23) { // VK_END
                    std::fs::write("debug_key.txt", "END key detected via WinAPI!").ok();
                    tracing::debug!("END key detected via Windows API");
                    if self.handle_event(AppEvent::End).await? {
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
                
                if self.is_key_just_pressed(0x2E) { // VK_DELETE
                    std::fs::write("debug_key.txt", "DELETE key detected via WinAPI!").ok();
                    tracing::info!("DELETE key detected via Windows API");
                    if self.handle_event(AppEvent::Delete).await? {
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
                
                if self.is_key_just_pressed(0x20) { // VK_SPACE (Space)
                    let field_debug = format!("SPACE key detected! Active field: {:?}, Is editing: {}", 
                                            self.filter_state.active_field, self.filter_state.is_editing);
                    std::fs::write("debug_key.txt", &field_debug).ok();
                    tracing::debug!("SPACE key detected via Windows API - {}", field_debug);
                    if self.handle_event(AppEvent::Space).await? {
                        break;
                    }
                    self.render()?;
                }
                
                if self.is_key_just_pressed(0x09) { // VK_TAB
                    let field_debug = format!("TAB key detected! Current field: {:?} -> ", self.filter_state.active_field);
                    std::fs::write("debug_key.txt", &field_debug).ok();
                    tracing::debug!("TAB key detected via Windows API - {}", field_debug);
                    if self.handle_event(AppEvent::Tab).await? {
                        break;
                    }
                    self.render()?;
                }
                
                // 移除 Ctrl+C 檢測，避免誤觸關閉程式
                // 使用者可以使用 ESC 或 'q' 鍵來退出程式
            }

            // Handle crossterm events
            // On Linux/non-Windows, handle all keyboard events via crossterm
            // On Windows, only handle character input and Backspace via crossterm (navigation keys use WinAPI)
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key_event) => {
                        tracing::debug!("Raw key event detected: {:?}", key_event);
                        let app_event = AppEvent::from(key_event);
                        tracing::debug!("Converted to AppEvent: {:?}", app_event);

                        #[cfg(not(windows))]
                        {
                            // On Linux/non-Windows: handle all key events via crossterm
                            if !matches!(app_event, AppEvent::Input('\0')) {
                                tracing::debug!("Processing event: {:?}", app_event);
                                if self.handle_event(app_event).await? {
                                    break;
                                }
                                self.render()?;
                            } else {
                                tracing::debug!("Ignored null input event");
                            }
                        }

                        #[cfg(windows)]
                        {
                            // On Windows: only handle character input, Backspace, and Copy via crossterm
                            // (navigation keys are handled by WinAPI above)
                            if matches!(app_event, AppEvent::Input(c) if c != '\0') || matches!(app_event, AppEvent::Backspace) || matches!(app_event, AppEvent::Copy) {
                                tracing::debug!("Input/Backspace/Copy event detected: {:?}", app_event);
                                if self.handle_event(app_event).await? {
                                    break;
                                }
                                self.render()?;
                            } else {
                                tracing::debug!("Non-input event ignored (handled by WinAPI): {:?}", app_event);
                            }
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

        info!("Main loop exiting");
        Ok(())
    }
    
    async fn handle_event(&mut self, event: AppEvent) -> Result<bool> {
        tracing::debug!("handle_event called with: {:?}, current state: {:?}", event, self.state);
        let is_delete = matches!(event, AppEvent::Delete);
        if !is_delete {
            self.message_list_state.delete_confirmation = false; // 清除刪除確認狀態
            self.topic_list_state.delete_confirmation = false; // 清除刪除確認狀態
        }
        match event {
            AppEvent::Quit => return Ok(true),

            AppEvent::Copy => {
                // Ctrl+C/Alt+C 的行為根據當前狀態決定：
                // - TopicList: 不做任何事（不退出）
                // - MessageList: 複製當前選中訊息的 payload
                // - PayloadDetail: 由 handle_payload_detail_event 處理（複製選中的內容）
                match self.state {
                    AppState::TopicList | AppState::Help | AppState::Quit => {
                        // 在 TopicList/Help/Quit 按 Ctrl+C 不做任何事
                        tracing::info!("Ctrl+C pressed in {:?} - ignoring", self.state);
                    }
                    AppState::MessageList => {
                        // 在 MessageList 複製當前選中訊息的 payload
                        if let Some(message) = self.message_list_state.get_selected_message() {
                            let payload = message.payload.clone();
                            let payload_len = payload.len();
                            tracing::info!("Copying payload from MessageList");
                            match self.copy_to_clipboard(&payload) {
                                Ok(()) => {
                                    tracing::info!("Successfully copied payload to clipboard ({} chars)", payload_len);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to copy payload to clipboard: {}", e);
                                }
                            }
                        } else {
                            tracing::warn!("No message selected in MessageList for copy");
                        }
                    }
                    AppState::PayloadDetail => {
                        // PayloadDetail 的 Copy 行為由 handle_payload_detail_event 處理
                        // 繼續向下傳遞到狀態特定的處理器
                        tracing::debug!("Routing Copy event to handle_payload_detail_event");
                        self.handle_payload_detail_event(event).await?;
                    }
                }
            }

            AppEvent::Refresh => {
                self.refresh_data().await?;
            }

            AppEvent::Filter => {
                self.toggle_filter_mode();
            }

            AppEvent::Escape => {
                if self.filter_state.is_editing {
                    self.filter_state.is_editing = false;
                } else if self.state == AppState::TopicList {
                    // 在 TopicList 狀態按 ESC/Ctrl+C 退出程式
                    tracing::info!("User requested exit from TopicList");
                    return Ok(true); // 返回 true 表示退出程式
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
            
            AppEvent::QuickFilter(index) => {
                // 快速過濾器只在MessageList和TopicList狀態下生效
                if self.state == AppState::MessageList {
                    // 記錄切換前是否處於即時刷新狀態
                    let was_in_auto_update_mode = self.should_auto_update_messages();
                    
                    self.message_list_state.toggle_quick_filter(index);
                    tracing::info!("Toggled quick filter {} - new state: {}, was_in_auto_update_mode: {}", 
                                 index, self.message_list_state.get_quick_filter_state(index), was_in_auto_update_mode);
                    
                    // 重新載入訊息以應用過濾器
                    if let Err(e) = self.message_list_state.reload_after_filter_change(&self.repository).await {
                        tracing::error!("Failed to reload messages after quick filter toggle: {}", e);
                    }
                    
                    // 如果之前處於即時刷新狀態，切換過濾器後自動focus到最新訊息
                    if was_in_auto_update_mode && !self.message_list_state.messages.is_empty() {
                        self.message_list_state.selected_index = 0; // 最新訊息在index 0
                        self.message_list_state.page = 1; // 回到第一頁（頁數從1開始）
                        tracing::info!("Auto-focused to latest message after filter toggle");
                    }
                    
                    // 更新狀態欄的快速過濾器狀態
                    if self.config.quick_filters.enabled && index < self.status_bar_state.quick_filter_states.len() {
                        self.status_bar_state.quick_filter_states[index].2 = self.message_list_state.get_quick_filter_state(index);
                    }
                } else if self.state == AppState::TopicList {
                    // 在TopicList狀態下也允許切換快速過濾器
                    self.message_list_state.toggle_quick_filter(index);
                    tracing::info!("Toggled quick filter {} in TopicList - new state: {}", 
                                 index, self.message_list_state.get_quick_filter_state(index));
                    // 更新狀態欄的快速過濾器狀態
                    if self.config.quick_filters.enabled && index < self.status_bar_state.quick_filter_states.len() {
                        self.status_bar_state.quick_filter_states[index].2 = self.message_list_state.get_quick_filter_state(index);
                    }
                }
            }
            
            _ => {
                // Handle state-specific events
                tracing::debug!("Routing event {:?} to state-specific handler for state {:?}", event, self.state);
                match self.state {
                    AppState::TopicList => self.handle_topic_list_event(event).await?,
                    AppState::MessageList => self.handle_message_list_event(event).await?,
                    AppState::PayloadDetail => {
                        tracing::debug!("Calling handle_payload_detail_event with event: {:?}", event);
                        self.handle_payload_detail_event(event).await?;
                    },
                    _ => {}
                }
            }
        }
        
        Ok(false)
    }
    
    async fn handle_topic_list_event(&mut self, event: AppEvent) -> Result<()> {
        if self.filter_state.is_editing {
            // 在 Topic 或 Payload 欄位時，上下鍵應該切換 topic
            // 在時間欄位時，上下左右鍵由過濾器處理邏輯處理
            if matches!(self.filter_state.active_field, 
                       crate::ui::widgets::FilterField::Topic | 
                       crate::ui::widgets::FilterField::Payload) &&
               matches!(event, AppEvent::NavigateUp | AppEvent::NavigateDown) {
                // 讓上下鍵事件傳遞到下面的處理邏輯
            } else {
                let should_apply_filter = matches!(event, AppEvent::Input(_) | AppEvent::Backspace);
                // 在時間編輯模式下，方向鍵需要重新渲染以更新高亮和光標
                let should_rerender = (self.filter_state.time_edit_mode || 
                                      matches!(self.filter_state.active_field, 
                                              crate::ui::widgets::FilterField::StartTime | 
                                              crate::ui::widgets::FilterField::EndTime)) && 
                                     matches!(event, AppEvent::NavigateLeft | AppEvent::NavigateRight | 
                                                    AppEvent::NavigateUp | AppEvent::NavigateDown |
                                                    AppEvent::PageUp | AppEvent::PageDown);
                let time_filter_changed = self.handle_filter_input(event);
                // 即時應用第一層的過濾器
                if should_apply_filter || time_filter_changed {
                    self.apply_filters().await?;
                }
                // 時間編輯模式下的導航需要重新渲染以更新高亮
                if should_rerender {
                    self.needs_full_redraw = true;
                }
                return Ok(());
            }
        }

        tracing::debug!("Handling topic list event: {:?}", event);
        match event {
            AppEvent::Tab => {
                tracing::debug!("Tab pressed in topic list - switching filter focus");
                if !self.filter_state.is_editing {
                    // 從主列表切換到Topic filter
                    self.filter_state.active_field = crate::ui::widgets::FilterField::Topic;
                } else {
                    // 已經在編輯模式，切換到下一個欄位
                    self.filter_state.next_field();
                }
                self.filter_state.is_editing = true;
                tracing::debug!("Auto-started editing after Tab in topic list, active field: {:?}", self.filter_state.active_field);
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
            AppEvent::PageUp => {
                self.topic_list_state.page_up();
            },
            AppEvent::PageDown => {
                self.topic_list_state.page_down();
            },
            AppEvent::Home => {
                tracing::debug!("Home key pressed in topic list - moving to top");
                self.topic_list_state.move_to_top();
            },
            AppEvent::End => {
                tracing::debug!("End key pressed in topic list - moving to bottom");
                self.topic_list_state.move_to_bottom();
            },
            AppEvent::Delete => {
                // 刪除選中的topic的所有記錄
                if let Some(selected_topic) = self.topic_list_state.get_selected_topic() {
                    let topic = selected_topic.topic.clone();
                    tracing::info!("Delete key pressed - preparing to delete all messages for topic: {}", topic);
                    tracing::info!("Current delete_confirmation state: {}", self.topic_list_state.delete_confirmation);
                    
                    // 顯示確認對話框（簡單實作：需要再按一次Delete確認）
                    if self.topic_list_state.delete_confirmation {
                        tracing::info!("Second Delete press detected - executing deletion for topic: {}", topic);
                        // 執行刪除
                        match self.repository.delete_messages_by_topic(&topic).await {
                            Ok(deleted_count) => {
                                tracing::info!("Successfully deleted {} messages for topic: {}", deleted_count, topic);
                                // 重新載入資料
                                self.refresh_data().await?;
                                self.topic_list_state.delete_confirmation = false;
                                tracing::info!("Delete confirmation state reset to false");
                            }
                            Err(e) => {
                                tracing::error!("Failed to delete messages for topic {}: {}", topic, e);
                                self.topic_list_state.delete_confirmation = false;
                                tracing::info!("Delete confirmation state reset to false after error");
                            }
                        }
                    } else {
                        // 第一次按Delete，設定確認標誌
                        self.topic_list_state.delete_confirmation = true;
                        self.needs_full_redraw = true; // 強制重新渲染以顯示確認提示
                        tracing::info!("First Delete press - setting confirmation flag to true for topic: {}", topic);
                        tracing::info!("Forced full redraw to display confirmation prompt");
                        tracing::info!("Press Delete again to confirm deletion of topic: {}", topic);
                    }
                } else {
                    tracing::warn!("Delete key pressed but no topic selected");
                }
            },
            _ => {
                // 任何其他按鍵都清除刪除確認狀態
                if self.topic_list_state.delete_confirmation {
                    tracing::info!("Non-Delete key pressed, clearing delete confirmation state");
                    self.topic_list_state.delete_confirmation = false;
                }
            }
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
                        // 在filter欄位按Enter開始編輯（如果尚未編輯）或結束編輯
                        if self.message_list_state.is_editing {
                            self.message_list_state.stop_editing();
                        } else {
                            self.message_list_state.start_editing();
                        }
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
                // 上方向鍵總是用於導航訊息，即使在編輯狀態下
                tracing::debug!("Navigate up in message list");
                let old_page = self.message_list_state.page;
                self.message_list_state.move_up_with_pagination(&self.repository).await?;
                if old_page != self.message_list_state.page {
                    self.needs_full_redraw = true; // Force redraw after page change
                }
            }
            AppEvent::NavigateDown => {
                // 下方向鍵總是用於導航訊息，即使在編輯狀態下
                tracing::debug!("Navigate down in message list");
                let old_page = self.message_list_state.page;
                self.message_list_state.move_down_with_pagination(&self.repository).await?;
                if old_page != self.message_list_state.page {
                    self.needs_full_redraw = true; // Force redraw after page change
                }
            }
            AppEvent::PageUp => {
                // PageUp總是用於翻頁，即使在編輯狀態下
                self.message_list_state.page_up(&self.repository).await?;
                self.needs_full_redraw = true; // Force redraw after page change
            }
            AppEvent::PageDown => {
                // PageDown總是用於翻頁，即使在編輯狀態下
                self.message_list_state.page_down(&self.repository).await?;
                self.needs_full_redraw = true; // Force redraw after page change
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
            AppEvent::Home => {
                // 在非編輯模式下，Home鍵用於移動到訊息列表頂部
                if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::MessageList) {
                    tracing::debug!("Home key pressed in message list - moving to first page first item");
                    self.message_list_state.move_to_top(&self.repository).await?;
                }
            }
            AppEvent::End => {
                // 在非編輯模式下，End鍵用於移動到訊息列表底部
                if matches!(self.message_list_state.get_focus(), crate::ui::views::message_list::FocusTarget::MessageList) {
                    tracing::debug!("End key pressed in message list - moving to last page last item");
                    self.message_list_state.move_to_bottom(&self.repository).await?;
                    
                    // 當用戶主動跳到最新訊息時，立即刷新以確保數據是最新的
                    tracing::debug!("User moved to latest message - refreshing data");
                    self.message_list_state.load_messages(&self.repository).await?;
                }
            }
            AppEvent::Delete => {
                // 刪除選中的單筆訊息
                if let Some(selected_msg) = self.message_list_state.get_selected_message() {
                    let topic = selected_msg.topic.clone();
                    let timestamp = selected_msg.timestamp.clone();
                    let id = selected_msg.id;
                    
                    tracing::info!("Delete key pressed - preparing to delete message: topic={}, timestamp={}", topic, timestamp);
                    tracing::info!("Current message delete_confirmation state: {}", self.message_list_state.delete_confirmation);
                    
                    // 顯示確認對話框（簡單實作：需要再按一次Delete確認）
                    if self.message_list_state.delete_confirmation {
                        tracing::info!("Second Delete press detected - executing message deletion: topic={}, timestamp={}", topic, timestamp);
                        // 執行刪除
                        let delete_result = if let Some(msg_id) = id {
                            tracing::info!("Deleting message by ID: {}", msg_id);
                            self.repository.delete_message_by_id(msg_id).await
                        } else {
                            tracing::info!("Deleting message by topic and timestamp: {} at {}", topic, timestamp);
                            self.repository.delete_message_by_topic_and_timestamp(&topic, &timestamp).await
                        };
                        
                        match delete_result {
                            Ok(success) => {
                                if success {
                                    tracing::info!("Successfully deleted message: topic={}, timestamp={}", topic, timestamp);
                                    // 重新載入當前頁面
                                    self.message_list_state.load_messages(&self.repository).await?;
                                } else {
                                    tracing::info!("Message not found for deletion: topic={}, timestamp={}", topic, timestamp);
                                }
                            }
                            Err(e) => {
                                tracing::info!("Failed to delete message: topic={}, timestamp={}, error={}", topic, timestamp, e);
                            }
                        }
                        self.message_list_state.delete_confirmation = false; // 清除刪除確認狀態
                    } else {
                        // 第一次按Delete，設定確認標誌
                        self.message_list_state.delete_confirmation = true;
                        self.needs_full_redraw = true; // 強制重新渲染以顯示確認提示
                        tracing::info!("First Delete press - setting message confirmation flag to true");
                        tracing::info!("Forced full redraw to display message confirmation prompt");
                        tracing::info!("Press Delete again to confirm message deletion");
                    }
                } else {
                    tracing::warn!("Delete key pressed in MessageList but no message selected");
                }
            }
            _ => {
                // 任何其他按鍵都清除刪除確認狀態
                if self.message_list_state.delete_confirmation {
                    tracing::info!("Non-Delete key pressed in message list, clearing delete confirmation state");
                }
            }
        }
        Ok(())
    }
    
    async fn handle_message_list_filter_input(&mut self, event: AppEvent) -> Result<()> {
        // 如果在時間編輯模式，特殊處理
        if self.message_list_state.time_edit_mode {
            match event {
                AppEvent::NavigateLeft => {
                    self.message_list_state.prev_time_position();
                    self.needs_full_redraw = true;
                }
                AppEvent::NavigateRight => {
                    self.message_list_state.next_time_position();
                    self.needs_full_redraw = true;
                }
                AppEvent::NavigateUp => {
                    self.message_list_state.adjust_time_value(-1);
                    self.apply_message_list_filters().await?;
                    self.needs_full_redraw = true;
                }
                AppEvent::NavigateDown => {
                    self.message_list_state.adjust_time_value(1);
                    self.apply_message_list_filters().await?;
                    self.needs_full_redraw = true;
                }
                AppEvent::PageUp => {
                    self.message_list_state.adjust_time_value(-10);
                    self.apply_message_list_filters().await?;
                    self.needs_full_redraw = true;
                }
                AppEvent::PageDown => {
                    self.message_list_state.adjust_time_value(10);
                    self.apply_message_list_filters().await?;
                    self.needs_full_redraw = true;
                }
                AppEvent::Space => {
                    // 再次按空白鍵關閉時間編輯模式
                    self.message_list_state.toggle_time_edit_mode();
                    self.apply_message_list_filters().await?;
                }
                AppEvent::Enter => {
                    // 確認並關閉時間編輯模式
                    self.message_list_state.time_edit_mode = false;
                    self.message_list_state.temp_datetime = None;
                    self.apply_message_list_filters().await?;
                }
                AppEvent::Escape => {
                    // 取消編輯，恢復原值
                    self.message_list_state.time_edit_mode = false;
                    self.message_list_state.temp_datetime = None;
                }
                AppEvent::Tab => {
                    // 在時間編輯模式下，Tab切換到下一個欄位並關閉時間編輯
                    self.message_list_state.time_edit_mode = false;
                    self.message_list_state.temp_datetime = None;
                    self.message_list_state.next_focus();
                    self.apply_message_list_filters().await?;
                }
                _ => {}
            }
            return Ok(());
        }
        
        // 如果在時間欄位，直接處理上下左右鍵
        if matches!(self.message_list_state.focus, 
                   crate::ui::views::message_list::FocusTarget::TimeFilterFrom | 
                   crate::ui::views::message_list::FocusTarget::TimeFilterTo) {
            match event {
                AppEvent::NavigateLeft => {
                    // 如果還沒進入時間編輯模式，先進入
                    if !self.message_list_state.time_edit_mode {
                        self.message_list_state.enter_time_edit_mode();
                    }
                    self.message_list_state.prev_time_position();
                    self.needs_full_redraw = true;
                    return Ok(());
                }
                AppEvent::NavigateRight => {
                    // 如果還沒進入時間編輯模式，先進入
                    if !self.message_list_state.time_edit_mode {
                        self.message_list_state.enter_time_edit_mode();
                    }
                    self.message_list_state.next_time_position();
                    self.needs_full_redraw = true;
                    return Ok(());
                }
                AppEvent::NavigateUp => {
                    // 如果還沒進入時間編輯模式，先進入
                    if !self.message_list_state.time_edit_mode {
                        self.message_list_state.enter_time_edit_mode();
                    }
                    self.message_list_state.adjust_time_value(-1);
                    self.apply_message_list_filters().await?;
                    self.needs_full_redraw = true;
                    return Ok(());
                }
                AppEvent::NavigateDown => {
                    // 如果還沒進入時間編輯模式，先進入
                    if !self.message_list_state.time_edit_mode {
                        self.message_list_state.enter_time_edit_mode();
                    }
                    self.message_list_state.adjust_time_value(1);
                    self.apply_message_list_filters().await?;
                    self.needs_full_redraw = true;
                    return Ok(());
                }
                AppEvent::PageUp => {
                    // 如果還沒進入時間編輯模式，先進入
                    if !self.message_list_state.time_edit_mode {
                        self.message_list_state.enter_time_edit_mode();
                    }
                    self.message_list_state.adjust_time_value(-10);
                    self.apply_message_list_filters().await?;
                    self.needs_full_redraw = true;
                    return Ok(());
                }
                AppEvent::PageDown => {
                    // 如果還沒進入時間編輯模式，先進入
                    if !self.message_list_state.time_edit_mode {
                        self.message_list_state.enter_time_edit_mode();
                    }
                    self.message_list_state.adjust_time_value(10);
                    self.apply_message_list_filters().await?;
                    self.needs_full_redraw = true;
                    return Ok(());
                }
                AppEvent::Space => {
                    // 空白鍵可以開關時間編輯模式
                    self.message_list_state.toggle_time_edit_mode();
                    return Ok(());
                }
                _ => {}
            }
        }
        
        // 正常的過濾器編輯模式
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
                self.message_list_state.insert_char_at_cursor(c);
                tracing::debug!("Added character '{}' at cursor position {}", c, self.message_list_state.cursor_position);
                // 即時應用過濾器
                self.apply_message_list_filters().await?;
            }
            AppEvent::Backspace => {
                self.message_list_state.delete_char_at_cursor();
                tracing::debug!("Removed character at cursor position");
                // 即時應用過濾器
                self.apply_message_list_filters().await?;
            }
            AppEvent::NavigateLeft => {
                // 在編輯模式下，左方向鍵移動遊標
                self.message_list_state.move_cursor_left();
                tracing::debug!("Cursor moved left to position {}", self.message_list_state.cursor_position);
            }
            AppEvent::NavigateRight => {
                // 在編輯模式下，右方向鍵移動遊標
                self.message_list_state.move_cursor_right();
                tracing::debug!("Cursor moved right to position {}", self.message_list_state.cursor_position);
            }
            AppEvent::Home => {
                self.message_list_state.move_cursor_home();
                tracing::debug!("Cursor moved to home position");
            }
            AppEvent::End => {
                self.message_list_state.move_cursor_end();
                tracing::debug!("Cursor moved to end position");
            }
            AppEvent::Paste(_) => {
                // 取得剪貼簿內容並貼上
                match self.get_clipboard_content() {
                    Ok(text) => {
                        self.message_list_state.insert_string_at_cursor(&text);
                        tracing::debug!("Pasted text '{}' at cursor position", text);
                        // 即時應用過濾器
                        self.apply_message_list_filters().await?;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to get clipboard content: {}", e);
                    }
                }
            }
            AppEvent::Enter => {
                tracing::debug!("Filter input submitted - stop editing");
                self.message_list_state.stop_editing();
                // Enter現在只是結束編輯，過濾已經即時應用了
            }
            AppEvent::Escape => {
                tracing::debug!("Filter input cancelled");
                self.message_list_state.stop_editing();
            }
            AppEvent::NavigateUp => {
                // 在編輯模式下也允許導航訊息
                tracing::debug!("Navigate up in message list (editing mode)");
                let old_page = self.message_list_state.page;
                self.message_list_state.move_up_with_pagination(&self.repository).await?;
                if old_page != self.message_list_state.page {
                    self.needs_full_redraw = true; // Force redraw after page change
                }
            }
            AppEvent::NavigateDown => {
                // 在編輯模式下也允許導航訊息
                tracing::debug!("Navigate down in message list (editing mode)");
                let old_page = self.message_list_state.page;
                self.message_list_state.move_down_with_pagination(&self.repository).await?;
                if old_page != self.message_list_state.page {
                    self.needs_full_redraw = true; // Force redraw after page change
                }
            }
            AppEvent::PageUp => {
                // 在編輯模式下也允許翻頁
                tracing::debug!("Page up in message list (editing mode)");
                self.message_list_state.page_up(&self.repository).await?;
                self.needs_full_redraw = true; // Force redraw after page change
            }
            AppEvent::PageDown => {
                // 在編輯模式下也允許翻頁
                tracing::debug!("Page down in message list (editing mode)");
                self.message_list_state.page_down(&self.repository).await?;
                self.needs_full_redraw = true; // Force redraw after page change
            }
            _ => {}
        }
        Ok(())
    }
    
    async fn apply_message_list_filters(&mut self) -> Result<()> {
        // 清除之前的錯誤
        self.message_list_state.filter_error = None;
        
        // 更新message list的filter criteria
        if !self.message_list_state.payload_filter_input.is_empty() {
            // 驗證regex語法
            if let Err(e) = regex::Regex::new(&self.message_list_state.payload_filter_input) {
                self.message_list_state.filter_error = Some(format!("Regex error: {}", e));
                // 清除無效的regex，避免在資料庫查詢時出錯
                self.message_list_state.filter.payload_regex = None;
                tracing::warn!("Invalid payload regex: {}", e);
                return Ok(()); // 不繼續處理
            } else {
                self.message_list_state.filter.payload_regex = Some(self.message_list_state.payload_filter_input.clone());
                tracing::info!("Setting payload_regex filter: {}", self.message_list_state.payload_filter_input);
            }
        } else {
            self.message_list_state.filter.payload_regex = None;
            tracing::info!("Clearing payload_regex filter");
        }
        
        // 處理時間過濾器
        if !self.message_list_state.time_from_input.is_empty() {
            if let Ok(parsed_time) = chrono::NaiveDateTime::parse_from_str(&self.message_list_state.time_from_input, "%Y-%m-%d %H:%M:%S") {
                self.message_list_state.filter.start_time = Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(parsed_time, chrono::Utc));
            } else {
                self.message_list_state.filter.start_time = None;
            }
        } else {
            self.message_list_state.filter.start_time = None;
        }
        
        if !self.message_list_state.time_to_input.is_empty() {
            if let Ok(parsed_time) = chrono::NaiveDateTime::parse_from_str(&self.message_list_state.time_to_input, "%Y-%m-%d %H:%M:%S") {
                self.message_list_state.filter.end_time = Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(parsed_time, chrono::Utc));
            } else {
                self.message_list_state.filter.end_time = None;
            }
        } else {
            self.message_list_state.filter.end_time = None;
        }
        
        tracing::info!("Final filter state: {:?}", self.message_list_state.filter);
        
        // 重設到第一頁並重新載入訊息
        self.message_list_state.page = 1;
        self.message_list_state.selected_index = 0;
        self.message_list_state.load_messages(&self.repository).await?;
        
        tracing::info!("Applied message list filters and reloaded messages");
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
            AppEvent::Tab => {
                // Tab鍵切換選擇模式 (topic -> payload -> formatted json -> topic...)
                self.payload_detail_selection = match self.payload_detail_selection {
                    PayloadDetailSelection::Topic => PayloadDetailSelection::Payload,
                    PayloadDetailSelection::Payload => PayloadDetailSelection::FormattedJson,
                    PayloadDetailSelection::FormattedJson => PayloadDetailSelection::Topic,
                };
                tracing::debug!("Payload detail selection switched to: {:?}", self.payload_detail_selection);
            }
            AppEvent::Copy => {
                // Alt+C 複製選中的內容
                tracing::info!("Copy event received in PayloadDetail view");
                tracing::info!("Current selection: {:?}", self.payload_detail_selection);
                
                match self.payload_detail_selection {
                    PayloadDetailSelection::Topic => {
                        tracing::info!("Attempting to copy topic");
                        if let Some(message) = self.get_selected_message() {
                            let topic = message.topic.clone();
                            tracing::info!("Found message with topic: {}", topic);
                            match self.copy_to_clipboard(&topic) {
                                Ok(()) => {
                                    tracing::info!("Successfully copied topic to clipboard: {}", topic);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to copy topic to clipboard: {}", e);
                                }
                            }
                        } else {
                            tracing::warn!("No message selected for topic copy");
                        }
                    }
                    PayloadDetailSelection::Payload => {
                        tracing::info!("Attempting to copy payload");
                        if let Some(message) = self.get_selected_message() {
                            let payload = message.payload.clone();
                            let payload_len = payload.len();
                            tracing::info!("Found message with payload length: {} chars", payload_len);
                            match self.copy_to_clipboard(&payload) {
                                Ok(()) => {
                                    tracing::info!("Successfully copied payload to clipboard ({} chars)", payload_len);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to copy payload to clipboard: {}", e);
                                }
                            }
                        } else {
                            tracing::warn!("No message selected for payload copy");
                        }
                    }
                    PayloadDetailSelection::FormattedJson => {
                        tracing::info!("Attempting to copy formatted JSON");
                        if let Some(message) = self.get_selected_message() {
                            let payload = message.payload.clone();
                            let payload_len = payload.len();
                            tracing::info!("Found message with payload length: {} chars", payload_len);
                            // Try to parse and format as JSON
                            match serde_json::from_str::<serde_json::Value>(&payload) {
                                Ok(json_value) => {
                                    match serde_json::to_string_pretty(&json_value) {
                                        Ok(formatted_json) => {
                                            let formatted_len = formatted_json.len();
                                            tracing::info!("Successfully parsed payload as JSON, formatted length: {} chars", formatted_len);
                                            match self.copy_to_clipboard(&formatted_json) {
                                                Ok(()) => {
                                                    tracing::info!("Successfully copied formatted JSON to clipboard ({} chars)", formatted_len);
                                                }
                                                Err(e) => {
                                                    tracing::error!("Failed to copy formatted JSON to clipboard: {}", e);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Failed to format JSON: {}, copying original payload", e);
                                            match self.copy_to_clipboard(&payload) {
                                                Ok(()) => {
                                                    tracing::info!("Successfully copied original payload to clipboard ({} chars)", payload_len);
                                                }
                                                Err(e) => {
                                                    tracing::error!("Failed to copy original payload to clipboard: {}", e);
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Payload is not valid JSON: {}, copying original payload", e);
                                    match self.copy_to_clipboard(&payload) {
                                        Ok(()) => {
                                            tracing::info!("Successfully copied original payload to clipboard ({} chars)", payload_len);
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to copy original payload to clipboard: {}", e);
                                        }
                                    }
                                }
                            }
                        } else {
                            tracing::warn!("No message selected for formatted JSON copy");
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    
    fn handle_filter_input(&mut self, event: AppEvent) -> bool {
        // 如果在時間編輯模式，特殊處理
        if self.filter_state.time_edit_mode {
            let mut should_apply_filter = false;
            match event {
                AppEvent::NavigateLeft => {
                    self.filter_state.prev_time_position();
                }
                AppEvent::NavigateRight => {
                    self.filter_state.next_time_position();
                }
                AppEvent::NavigateUp => {
                    self.filter_state.adjust_time_value(-1);
                }
                AppEvent::NavigateDown => {
                    self.filter_state.adjust_time_value(1);
                }
                AppEvent::PageUp => {
                    self.filter_state.adjust_time_value(-10);
                }
                AppEvent::PageDown => {
                    self.filter_state.adjust_time_value(10);
                }
                AppEvent::Space => {
                    // 再次按空白鍵關閉時間編輯模式並套用過濾
                    self.filter_state.toggle_time_edit_mode();
                    should_apply_filter = true;
                }
                AppEvent::Enter => {
                    // 確認並關閉時間編輯模式，套用過濾
                    self.filter_state.time_edit_mode = false;
                    self.filter_state.temp_datetime = None;
                    should_apply_filter = true;
                }
                AppEvent::Escape => {
                    // 取消編輯，恢復原值
                    self.filter_state.time_edit_mode = false;
                    self.filter_state.temp_datetime = None;
                    // TODO: 恢復原始值
                }
                AppEvent::Tab => {
                    // 在時間編輯模式下，Tab切換到下一個欄位並關閉時間編輯
                    self.filter_state.time_edit_mode = false;
                    self.filter_state.temp_datetime = None;
                    self.filter_state.next_field();
                    should_apply_filter = true;
                }
                _ => {}
            }
            return should_apply_filter;
        } else {
            // 正常的過濾器編輯模式
            // 如果在時間欄位，直接處理上下左右鍵
            if matches!(self.filter_state.active_field, 
                       crate::ui::widgets::FilterField::StartTime | 
                       crate::ui::widgets::FilterField::EndTime) {
                match event {
                    AppEvent::NavigateLeft => {
                        // 如果還沒進入時間編輯模式，先進入
                        if !self.filter_state.time_edit_mode {
                            self.filter_state.enter_time_edit_mode();
                        }
                        self.filter_state.prev_time_position();
                    }
                    AppEvent::NavigateRight => {
                        // 如果還沒進入時間編輯模式，先進入
                        if !self.filter_state.time_edit_mode {
                            self.filter_state.enter_time_edit_mode();
                        }
                        self.filter_state.next_time_position();
                    }
                    AppEvent::NavigateUp => {
                        // 如果還沒進入時間編輯模式，先進入
                        if !self.filter_state.time_edit_mode {
                            self.filter_state.enter_time_edit_mode();
                        }
                        self.filter_state.adjust_time_value(-1);
                    }
                    AppEvent::NavigateDown => {
                        // 如果還沒進入時間編輯模式，先進入
                        if !self.filter_state.time_edit_mode {
                            self.filter_state.enter_time_edit_mode();
                        }
                        self.filter_state.adjust_time_value(1);
                    }
                    AppEvent::PageUp => {
                        // 如果還沒進入時間編輯模式，先進入
                        if !self.filter_state.time_edit_mode {
                            self.filter_state.enter_time_edit_mode();
                        }
                        self.filter_state.adjust_time_value(-10);
                    }
                    AppEvent::PageDown => {
                        // 如果還沒進入時間編輯模式，先進入
                        if !self.filter_state.time_edit_mode {
                            self.filter_state.enter_time_edit_mode();
                        }
                        self.filter_state.adjust_time_value(10);
                    }
                    AppEvent::Tab => {
                        // Tab 鍵切換到下一個欄位
                        if self.filter_state.time_edit_mode {
                            self.filter_state.time_edit_mode = false;
                            self.filter_state.temp_datetime = None;
                        }
                        self.filter_state.next_field();
                    }
                    AppEvent::Space => {
                        // 空白鍵可以開關時間編輯模式
                        self.filter_state.toggle_time_edit_mode();
                    }
                    _ => {}
                }
            } else {
                // Topic 或 Payload 欄位的處理
                match event {
                    AppEvent::Space => {
                        // 在其他欄位，空白鍵視為正常輸入
                        self.filter_state.get_active_field_value_mut().push(' ');
                    }
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
        }
        return false; // 正常編輯模式不需要套用過濾
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
                    
                    // 同步快速過濾器狀態到狀態欄
                    if self.config.quick_filters.enabled {
                        for index in 0..self.status_bar_state.quick_filter_states.len().min(5) {
                            if index < self.config.quick_filters.filters.len() {
                                self.status_bar_state.quick_filter_states[index].0 = self.config.quick_filters.filters[index].name.clone();
                                self.status_bar_state.quick_filter_states[index].1 = self.config.quick_filters.filters[index].color.clone();
                                self.status_bar_state.quick_filter_states[index].2 = self.message_list_state.get_quick_filter_state(index);
                            }
                        }
                    }
                    
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
    
    // 判斷是否應該自動更新MessageList中的訊息
    fn should_auto_update_messages(&self) -> bool {
        let state = &self.message_list_state;
        
        // 如果沒有訊息，則可以更新
        if state.messages.is_empty() {
            return true;
        }
        
        // 由於訊息按timestamp DESC排序，index 0 是最新訊息
        let is_on_newest_message = state.selected_index == 0;
        
        // 檢查當前是否在最新頁（第一頁包含最新訊息）
        let is_on_latest_page = state.page == 1;
        
        // 只有當用戶在最新頁面且focus在最新訊息（index 0）時才自動更新
        let should_update = is_on_latest_page && is_on_newest_message;
        
        tracing::debug!(
            "should_auto_update_messages: page={}, selected={}, total={}, is_latest_page={}, is_newest_msg={}, should_update={}",
            state.page, state.selected_index, state.messages.len(), 
            is_on_latest_page, is_on_newest_message, should_update
        );
        
        should_update
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
                // 只有當用戶focus在最新訊息時才自動更新
                if self.should_auto_update_messages() {
                    tracing::debug!("refresh_data called in MessageList state - auto update");
                    self.message_list_state.load_messages(&self.repository).await?;
                    tracing::debug!("MessageList messages refreshed successfully");
                } else {
                    tracing::debug!("MessageList auto-update skipped - user browsing history");
                }
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
        
        // 解析時間過濾器
        if !self.filter_state.start_time.is_empty() {
            // 嘗試解析開始時間
            if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(&self.filter_state.start_time, "%Y-%m-%d %H:%M:%S") {
                use chrono::{TimeZone, Local};
                if let Some(dt) = Local.from_local_datetime(&naive_dt).single() {
                    criteria.start_time = Some(dt.with_timezone(&chrono::Utc));
                    tracing::debug!("Applied start time filter: {}", dt);
                }
            }
        }
        
        if !self.filter_state.end_time.is_empty() {
            // 嘗試解析結束時間
            if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(&self.filter_state.end_time, "%Y-%m-%d %H:%M:%S") {
                use chrono::{TimeZone, Local};
                if let Some(dt) = Local.from_local_datetime(&naive_dt).single() {
                    criteria.end_time = Some(dt.with_timezone(&chrono::Utc));
                    tracing::debug!("Applied end time filter: {}", dt);
                }
            }
        }
        
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
        prev.help_text == current.help_text &&
        prev.quick_filter_states == current.quick_filter_states
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
    
    pub fn get_config(&self) -> &Config {
        &self.config
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
    
    pub fn get_message_list_filter_error(&self) -> &Option<String> {
        &self.message_list_state.filter_error
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
    
    pub fn get_payload_detail_selection(&self) -> PayloadDetailSelection {
        self.payload_detail_selection
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
    
    pub fn get_clipboard_content(&self) -> Result<String> {
        #[cfg(windows)]
        {
            use std::ffi::OsString;
            use std::os::windows::ffi::OsStringExt;
            use winapi::um::winuser::{OpenClipboard, GetClipboardData, CloseClipboard};
            use winapi::um::winbase::GlobalLock;
            use winapi::shared::minwindef::HGLOBAL;
            use winapi::um::winnt::HANDLE;
            use winapi::um::winuser::CF_UNICODETEXT;
            
            unsafe {
                if OpenClipboard(std::ptr::null_mut()) == 0 {
                    return Err(anyhow::anyhow!("Failed to open clipboard"));
                }
                
                let handle: HANDLE = GetClipboardData(CF_UNICODETEXT);
                if handle.is_null() {
                    CloseClipboard();
                    return Err(anyhow::anyhow!("No text data in clipboard"));
                }
                
                let data_ptr = GlobalLock(handle as HGLOBAL);
                if data_ptr.is_null() {
                    CloseClipboard();
                    return Err(anyhow::anyhow!("Failed to lock clipboard data"));
                }
                
                // Convert wide string to Rust string
                let wide_ptr = data_ptr as *const u16;
                let mut len = 0;
                while *wide_ptr.offset(len) != 0 {
                    len += 1;
                }
                
                let wide_slice = std::slice::from_raw_parts(wide_ptr, len as usize);
                let os_string = OsString::from_wide(wide_slice);
                let result = os_string.to_string_lossy().to_string();
                
                CloseClipboard();
                Ok(result)
            }
        }
        
        #[cfg(not(windows))]
        {
            // For non-Windows platforms, return empty string for now
            // TODO: Implement clipboard support for other platforms
            Ok(String::new())
        }
    }
    
    pub fn copy_to_clipboard(&mut self, text: &str) -> Result<()> {
        use copypasta::ClipboardProvider;

        tracing::info!("copy_to_clipboard called with text length: {} chars", text.len());
        tracing::debug!("Text to copy: {}", text);

        // Always save to clipboard.txt first
        std::fs::write("clipboard.txt", text)?;
        tracing::info!("Content saved to clipboard.txt ({} chars)", text.len());

        // Try to also copy to system clipboard using the persistent context
        if let Some(ref mut ctx) = self.clipboard_ctx {
            match ctx.set_contents(text.to_owned()) {
                Ok(()) => {
                    tracing::info!("Successfully copied {} chars to system clipboard", text.len());
                }
                Err(e) => {
                    tracing::warn!("Failed to set system clipboard contents: {}", e);
                }
            }
        } else {
            tracing::warn!("Clipboard context not available");
        }

        Ok(())
    }
    
}