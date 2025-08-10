# MQTT Log Viewer

一個基於 Rust 和 crossterm 的互動式 MQTT 訊息記錄檢視器，能夠即時接收、儲存和展示 MQTT 訊息。

## 特色功能

- **三層介面設計**：主題總覽 → 訊息列表 → 詳細內容
- **即時更新**：第一層和第二層每 0.25 秒自動更新（增量渲染，無閃爍）
- **Windows API 按鍵偵測**：使用 Windows API 直接偵測按鍵，確保所有按鍵正常運作
- **強大過濾**：支援 Topic、Payload、時間範圍的正則表達式過濾
- **刪除功能**：支援刪除整個 Topic 或單筆訊息（雙重確認機制）
- **複製功能**：支援複製訊息內容到剪貼簿
- **JSON 美化**：自動偵測並美化顯示 JSON 內容
- **高效儲存**：使用 SQLite + rbatis 進行資料持久化
- **智慧顯示**：第一、二層顯示 JSON 鍵名，第三層顯示完整內容

## 系統需求

- Rust 1.70+
- MQTT Broker (如 Mosquitto)
- Windows 10+ (主要支援 Windows，使用 Windows API 進行按鍵偵測)

## 安裝與使用

### 編譯

```bash
git clone <repository_url>
cd mqtt_log_view
cargo build --release
```

### 執行

```bash
cargo run
```

### 設定檔

程式會自動創建 `config.toml` 設定檔：

```toml
[mqtt]
host = "127.0.0.1"
port = 1883
username = ""
password = ""
client_id = "mqtt_log_viewer"

[database]
path = "./mqtt_logs.db"
max_messages = 100000
auto_cleanup = true
cleanup_days = 30

[ui]
refresh_interval_ms = 250
max_payload_preview = 50
theme = "dark"
enable_json_highlight = true

[performance]
max_memory_mb = 100
cache_size = 1000
batch_size = 100
```

## 介面說明

### 第一層：Topic 總覽

顯示所有 MQTT 主題的統計資訊：

```
┌─ MQTT Log Viewer ─────────────────────────────────────────────────────┐
│ Connection: ●Connected (127.0.0.1:1883)                              │
│ Topic Filter: [___________] [Apply] [Clear]                           │
│ Payload Filter: [___________] [Apply] [Clear]                         │  
│ Time: From [__________] To [__________] [Apply]                       │
├───────────────────────────────────────────────────────────────────────┤
│ Last Message │ Topic              │ Count  │ Latest Payload           │
│ 10:30:01     │ sensors/temp       │ 42     │ {"temperature","unit"}   │
│ 10:30:02     │ devices/status     │ 9999+  │ {"status","timestamp"}   │
│ 10:30:03     │ system/heartbeat   │ 1      │ ping                     │
└───────────────────────────────────────────────────────────────────────┘
```

### 第二層：訊息列表

顯示選定主題的訊息歷史：

```
┌─ Topic: sensors/temp ─────────────────────────────────────────────────┐
│ Payload Filter: [___________] [Apply] [Clear]                         │
│ Time: From [__________] To [__________] [Apply]                       │
├───────────────────────────────────────────────────────────────────────┤
│ Time     │ Payload                                                    │
│ 10:29:55 │ {"temperature","unit"}                                     │
│ 10:29:58 │ {"temperature","unit"}                                     │
│ 10:30:01 │ {"temperature","unit"}                                     │
└───────────────────────────────────────────────────────────────────────┘
```

### 第三層：Payload 詳細檢視

完整顯示選定訊息的內容：

```
┌─ Payload Detail ──────────────────────────────────────────────────────┐
│ Topic: sensors/temp                                                   │
│ Time: 2024-01-20 10:30:01                                            │
├───────────────────────────────────────────────────────────────────────┤
│ {                                                                      │
│   "temperature": 25.5,                                               │
│   "unit": "C",                                                        │
│   "sensor_id": "TEMP001",                                            │
│   "location": "Living Room",                                          │
│   "battery": 85                                                       │
│ }                                                                      │
└───────────────────────────────────────────────────────────────────────┘
```

## 鍵盤快捷鍵

### 全域快捷鍵
- `q`: 退出程式
- `h`: 顯示說明
- `r`: 手動刷新
- `Ctrl+C`: 強制退出

### 第一層（Topic 總覽）
- `↑↓`: 選擇主題
- `Enter`: 進入選定主題
- `←`: 返回上一層（如適用）
- `→`: 進入下一層
- `Tab`: 切換焦點到過濾器
- `Delete`: 刪除選定的 Topic（所有訊息），需要按兩次確認
- `Page Up/Down`: 翻頁導航
- `Home/End`: 跳到第一項/最後一項

### 第二層（訊息列表）
- `↑↓`: 選擇訊息
- `Enter`: 查看訊息詳情
- `←`: 返回主題列表
- `→`: 進入訊息詳情
- `Tab`: 切換過濾器焦點
- `Delete`: 刪除選定的訊息，需要按兩次確認
- `Page Up/Down`: 翻頁導航
- `Home/End`: 跳到第一項/最後一項

### 第三層（Payload 詳細檢視）
- `↑↓`: 上下滾動
- `←`: 返回訊息列表
- `Page Up/Down`: 整頁滾動
- `Home/End`: 跳轉到開頭/結尾
- `Alt+C`: 複製內容到剪貼簿
- `Tab`: 切換複製模式（原始/美化/鍵值）

## 過濾功能

### Topic 過濾器
使用正則表達式過濾主題名稱：
- `^sensors/` - 所有 sensors 開頭的主題
- `temperature$` - 所有以 temperature 結尾的主題
- `device[0-9]+` - 符合 device 後跟數字的主題

### Payload 過濾器
使用正則表達式過濾訊息內容：
- `"temperature":\s*[0-9]+` - 包含溫度數值的 JSON
- `error|warning` - 包含錯誤或警告的訊息
- `^\{.*\}$` - 完整的 JSON 物件格式

### 時間過濾器
支援時間範圍過濾（格式：YYYY-MM-DD HH:MM:SS）：
- From: `2024-01-20 09:00:00`
- To: `2024-01-20 18:00:00`

## JSON 顯示

程式會自動偵測 JSON 格式並提供三種顯示模式：

1. **簡化模式**（第一、二層）：只顯示鍵名
   - 原始：`{"temperature": 25.5, "unit": "C"}`
   - 簡化：`{"temperature","unit"}`

2. **第一層模式**（第三層）：顯示第一層鍵值對
   - 巢狀物件顯示為 `{...}` 或 `[...]`

3. **完整模式**（第三層）：顯示完整 JSON 結構
   - 包含語法高亮和縮排

## 開發狀態

目前實作狀態：

- ✅ PRD 文件
- ✅ 專案架構
- ✅ 資料庫模型與 CRUD 操作
- ✅ MQTT 客戶端（完整支援）
- ✅ 三層 UI 架構（完整實現）
- ✅ 增量渲染系統（防閃爍）
- ✅ Windows API 按鍵偵測
- ✅ JSON 格式化與顯示
- ✅ 訊息刪除功能（Topic 級別與單筆訊息）
- ✅ 複製功能（多種模式）
- ✅ 過濾功能（Topic、Payload、時間範圍）
- ✅ 分頁導航與快捷鍵
- ✅ 即時資料更新

## 最新改進

### v1.0 完整功能實現
- **Windows API 按鍵偵測**: 解決 crossterm 在 Windows 環境下按鍵偵測不穩定的問題
- **增量渲染系統**: 消除了每 0.25 秒全畫面重繪造成的閃爍，提供流暢的用戶體驗
- **完整刪除功能**: 支援 Topic 級別和單筆訊息刪除，具備雙重確認機制防止誤操作
- **多模式複製**: 第三層支援原始格式、JSON 美化和鍵值對三種複製模式
- **即時資料同步**: 第二層 UI 現在每 0.25 秒自動更新，確保新訊息即時顯示
- **完善的導航**: Home/End 鍵、分頁功能、方向鍵導航全面支援

## 授權

MIT License

## 貢獻

歡迎提交 Issues 和 Pull Requests！

## 技術架構

- **語言**: Rust 2021 Edition
- **TUI**: crossterm
- **MQTT**: rumqttc
- **資料庫**: SQLite + rbatis
- **JSON 處理**: serde_json
- **非同步**: tokio
- **正則表達式**: regex