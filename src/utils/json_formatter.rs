use serde_json::{Value, Map};
use anyhow::Result;

#[derive(Debug, Clone, Copy)]
pub enum JsonDisplayMode {
    KeysOnly,    // 只顯示鍵名，用於第一、二層界面
    FirstLevel,  // 顯示第一層鍵值對，巢狀物件顯示為 {...}
    Full,        // 完整顯示
}

pub struct JsonFormatter;

impl JsonFormatter {
    /// 檢查字串是否為有效的 JSON
    pub fn is_valid_json(text: &str) -> bool {
        serde_json::from_str::<Value>(text).is_ok()
    }
    
    /// 根據指定模式格式化 JSON 字串
    pub fn format(text: &str, mode: JsonDisplayMode) -> Result<String> {
        if !Self::is_valid_json(text) {
            return Ok(text.to_string());
        }
        
        let value: Value = serde_json::from_str(text)?;
        
        match mode {
            JsonDisplayMode::KeysOnly => Self::format_keys_only(&value),
            JsonDisplayMode::FirstLevel => Self::format_first_level(&value, 0),
            JsonDisplayMode::Full => Ok(serde_json::to_string_pretty(&value)?),
        }
    }
    
    /// 只顯示鍵名的格式，例如 {"temperature","unit","metadata"}
    fn format_keys_only(value: &Value) -> Result<String> {
        match value {
            Value::Object(map) => {
                let keys: Vec<String> = map.keys()
                    .map(|k| format!("\"{}\"", k))
                    .collect();
                Ok(format!("{{{}}}", keys.join(",")))
            }
            Value::Array(_) => Ok("[...]".to_string()),
            _ => Ok(value.to_string()),
        }
    }
    
    /// 顯示第一層的格式，巢狀物件顯示為 {...} 或 [...]
    fn format_first_level(value: &Value, depth: usize) -> Result<String> {
        match value {
            Value::Object(map) => {
                if depth == 0 {
                    let mut items = Vec::new();
                    for (key, val) in map {
                        let formatted_val = match val {
                            Value::Object(_) => "{...}".to_string(),
                            Value::Array(_) => "[...]".to_string(),
                            Value::String(s) => format!("\"{}\"", s),
                            other => other.to_string(),
                        };
                        items.push(format!("  \"{}\": {}", key, formatted_val));
                    }
                    Ok(format!("{{\n{}\n}}", items.join(",\n")))
                } else {
                    Ok("{...}".to_string())
                }
            }
            Value::Array(_) => {
                if depth == 0 {
                    Ok("[...]".to_string())
                } else {
                    Ok("[...]".to_string())
                }
            }
            Value::String(s) => Ok(format!("\"{}\"", s)),
            other => Ok(other.to_string()),
        }
    }
    
    /// 簡化 payload 顯示，用於列表視圖
    pub fn simplify_payload(text: &str, max_length: usize) -> String {
        let trimmed = text.trim();
        
        // 如果是 JSON，顯示鍵名
        if Self::is_valid_json(trimmed) {
            match Self::format(trimmed, JsonDisplayMode::KeysOnly) {
                Ok(formatted) => {
                    if formatted.len() > max_length {
                        format!("{}...", &formatted[..max_length.saturating_sub(3)])
                    } else {
                        formatted
                    }
                }
                Err(_) => {
                    // 如果格式化失敗，回退到截斷顯示
                    if trimmed.len() > max_length {
                        format!("{}...", &trimmed[..max_length.saturating_sub(3)])
                    } else {
                        trimmed.to_string()
                    }
                }
            }
        } else {
            // 非 JSON，直接截斷
            if trimmed.is_empty() {
                "(empty)".to_string()
            } else if trimmed.len() > max_length {
                format!("{}...", &trimmed[..max_length.saturating_sub(3)])
            } else {
                trimmed.to_string()
            }
        }
    }
    
    /// 為 JSON 添加語法高亮（使用 ANSI 顏色代碼）
    pub fn highlight_json(text: &str) -> Result<String> {
        if !Self::is_valid_json(text) {
            return Ok(text.to_string());
        }
        
        let value: Value = serde_json::from_str(text)?;
        Self::colorize_value(&value, 0)
    }
    
    fn colorize_value(value: &Value, indent: usize) -> Result<String> {
        let indent_str = "  ".repeat(indent);
        
        match value {
            Value::Object(map) => {
                if map.is_empty() {
                    return Ok("{}".to_string());
                }
                
                let mut result = "{\n".to_string();
                let entries: Vec<_> = map.iter().collect();
                
                for (i, (key, val)) in entries.iter().enumerate() {
                    let comma = if i < entries.len() - 1 { "," } else { "" };
                    let colored_key = format!("\x1b[34m\"{}\"\x1b[0m", key); // 藍色鍵名
                    let colored_val = Self::colorize_value(val, indent + 1)?;
                    result.push_str(&format!("{}  {}: {}{}\n", indent_str, colored_key, colored_val, comma));
                }
                
                result.push_str(&format!("{}}}", indent_str));
                Ok(result)
            }
            Value::Array(arr) => {
                if arr.is_empty() {
                    return Ok("[]".to_string());
                }
                
                let mut result = "[\n".to_string();
                for (i, item) in arr.iter().enumerate() {
                    let comma = if i < arr.len() - 1 { "," } else { "" };
                    let colored_item = Self::colorize_value(item, indent + 1)?;
                    result.push_str(&format!("{}  {}{}\n", indent_str, colored_item, comma));
                }
                
                result.push_str(&format!("{}]", indent_str));
                Ok(result)
            }
            Value::String(s) => Ok(format!("\x1b[32m\"{}\"\x1b[0m", s)), // 綠色字串
            Value::Number(n) => Ok(format!("\x1b[33m{}\x1b[0m", n)), // 黃色數字
            Value::Bool(b) => Ok(format!("\x1b[31m{}\x1b[0m", b)), // 紅色布林值
            Value::Null => Ok("\x1b[90mnull\x1b[0m".to_string()), // 灰色 null
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_json() {
        assert!(JsonFormatter::is_valid_json(r#"{"key": "value"}"#));
        assert!(JsonFormatter::is_valid_json(r#"[1, 2, 3]"#));
        assert!(!JsonFormatter::is_valid_json("not json"));
    }

    #[test]
    fn test_format_keys_only() {
        let json = r#"{"temperature": 25.5, "unit": "C", "metadata": {"sensor": "DHT22"}}"#;
        let result = JsonFormatter::format(json, JsonDisplayMode::KeysOnly).unwrap();
        assert_eq!(result, r#"{"temperature","unit","metadata"}"#);
    }

    #[test]
    fn test_simplify_payload() {
        let json = r#"{"temperature": 25.5, "unit": "C"}"#;
        let result = JsonFormatter::simplify_payload(json, 30);
        assert_eq!(result, r#"{"temperature","unit"}"#);
        
        let long_text = "This is a very long text that should be truncated";
        let result = JsonFormatter::simplify_payload(long_text, 20);
        assert_eq!(result, "This is a very lo...");
        
        let empty = "";
        let result = JsonFormatter::simplify_payload(empty, 20);
        assert_eq!(result, "(empty)");
    }
}