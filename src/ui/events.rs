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
}

impl From<KeyEvent> for AppEvent {
    fn from(key_event: KeyEvent) -> Self {
        // 只處理按鍵按下事件，忽略按鍵釋放事件
        if key_event.kind != KeyEventKind::Press {
            return AppEvent::Input('\0'); // 忽略非按下事件
        }
        
        match key_event.code {
            KeyCode::Char('q') => AppEvent::Quit,
            KeyCode::Char('r') => AppEvent::Refresh,
            KeyCode::Char('f') => AppEvent::Filter,
            KeyCode::Char('j') => AppEvent::JsonToggle,
            KeyCode::Char('h') => AppEvent::Help,
            KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => AppEvent::Copy,
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
            KeyCode::Home => AppEvent::Home,
            KeyCode::End => AppEvent::End,
            KeyCode::Backspace => AppEvent::Backspace,
            KeyCode::Delete => AppEvent::Delete,
            _ => AppEvent::Input('\0'), // Ignore other keys
        }
    }
}