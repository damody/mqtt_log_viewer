use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, KeyEventKind};

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
}

impl From<KeyEvent> for AppEvent {
    fn from(key_event: KeyEvent) -> Self {
        // 只處理按鍵按下事件，忽略按鍵釋放事件
        if key_event.kind != KeyEventKind::Press {
            return AppEvent::Input('\0'); // 忽略非按下事件
        }
        
        match key_event.code {
            KeyCode::F(5) => AppEvent::Refresh,
            KeyCode::Char('/') => AppEvent::Filter,
            KeyCode::F(2) => AppEvent::JsonToggle,
            KeyCode::F(1) => AppEvent::Help,
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