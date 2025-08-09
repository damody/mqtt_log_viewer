# MQTT Log Viewer - 產品需求文件 (PRD)

## 1. 產品概述

### 1.1 產品目標
開發一個基於 Rust 和 crossterm 的互動式 MQTT 訊息記錄檢視器，能夠即時接收、儲存和展示 MQTT 訊息，並提供強大的過濾和檢索功能。

### 1.2 核心價值
- 即時監控 MQTT 訊息流量
- 提供直觀的三層介面設計
- 支援複雜的過濾和搜尋條件
- 高效的訊息儲存和查詢
- 優秀的 JSON 訊息格式化顯示

## 2. 技術架構

### 2.1 技術選型
- **語言**: Rust
- **TUI 框架**: crossterm
- **資料庫**: SQLite
- **ORM**: rbatis
- **MQTT 客戶端**: rumqttc
- **異步運行時**: tokio
- **JSON 處理**: serde_json
- **正則表達式**: regex

### 2.2 系統架構
```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   MQTT Broker   │    │  MQTT Client    │    │   SQLite DB     │
│  (127.0.0.1)    │◄──►│   (Subscribe)   │◄──►│   (Storage)     │
└─────────────────┘    └─────────────────┘    └─────────────────┘
                                │
                                ▼
                       ┌─────────────────┐
                       │   TUI Interface │
                       │  (3-Layer View) │
                       └─────────────────┘
```

## 3. 功能需求

### 3.1 MQTT 連接功能
- **連接目標**: 127.0.0.1:1883 (預設 MQTT 埠)
- **訂閱主題**: # (萬用字元，訂閱所有主題)
- **連接狀態**: 顯示連接狀態指示器
- **重連機制**: 自動重連機制，連接失敗時自動重試

### 3.2 資料儲存功能
- **資料庫**: SQLite 本地資料庫
- **資料表結構**:
  ```sql
  CREATE TABLE messages (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      topic TEXT NOT NULL,
      payload TEXT NOT NULL,
      timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
      qos INTEGER DEFAULT 0,
      retain BOOLEAN DEFAULT 0
  );
  ```
- **索引優化**: 在 topic, timestamp 欄位建立索引

### 3.3 三層介面設計

#### 3.3.1 第一層：Topic 總覽界面
**功能描述**: 顯示所有主題的統計資訊和過濾功能

**介面佈局**:
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
│              │                    │        │                          │
└───────────────────────────────────────────────────────────────────────┘
Status: 1,234 topics | 45,678 messages | Last: 2024-01-20 10:30:03
[q]uit [f]ilter [Enter]select [↑↓]navigate [r]efresh [h]elp
```

**功能特性**:
- **即時更新**: 每 0.25 秒刷新顯示
- **過濾器**:
  - Topic Filter: 支援正則表達式過濾主題名稱
  - Payload Filter: 支援正則表達式過濾訊息內容
  - Time Filter: 支援時間範圍過濾 (YYYY-MM-DD HH:MM:SS 格式)
- **統計資訊**:
  - Last Message: 顯示該主題最後一筆符合過濾條件的訊息時間
  - Count: 顯示符合過濾條件的訊息數量 (最多顯示 9999，超過顯示 9999+)
  - Latest Payload: 顯示最新訊息的 payload，簡化顯示格式
- **Payload 簡化規則**:
  - JSON 格式: 只顯示第一層的鍵名，如 `{"temperature","unit"}` 
  - 非 JSON: 顯示前 30 字元，超出用 "..." 表示
  - 空內容: 顯示為 "(empty)"
- **排序**: 預設依最後訊息時間降序排列

#### 3.3.2 第二層：訊息列表界面
**功能描述**: 顯示選定主題下的所有訊息

**介面佈局**:
```
┌─ Topic: sensors/temp ─────────────────────────────────────────────────┐
│ Payload Filter: [___________] [Apply] [Clear]                         │
│ Time: From [__________] To [__________] [Apply]                       │
├───────────────────────────────────────────────────────────────────────┤
│ Time     │ Payload                                                    │
│ 10:29:55 │ {"temperature","unit"}                                     │
│ 10:29:58 │ {"temperature","unit"}                                     │
│ 10:30:01 │ {"temperature","unit"}                                     │
│ 10:30:04 │ {"temperature","unit"}                                     │
│          │                                                            │
└───────────────────────────────────────────────────────────────────────┘
Page 1/25 | 1,024 messages | Selected: sensors/temp
[←][ESC]back [f]ilter [Enter]view [↑↓]navigate [PgUp/PgDn]page [j]son [h]elp
```

**功能特性**:
- **即時更新**: 每 0.25 秒刷新顯示
- **繼承過濾**: 繼承第一層的過濾條件
- **額外過濾**:
  - Payload Filter: 針對該主題的 payload 進行額外過濾
  - Time Filter: 針對該主題的時間範圍進行額外過濾
- **分頁顯示**: 每頁顯示 100 條訊息
- **翻頁功能**:
  - PageUp: 載入上一頁訊息（若有）
  - PageDown: 載入下一頁訊息（若有）
  - 翻頁時重置選中項為第一筆
  - 自動檢測是否到達最後一頁
- **Payload 簡化顯示**: 
  - JSON 格式: 只顯示第一層鍵名，如 `{"temperature","unit"}`
  - 非 JSON: 顯示前 50 字元，超出用 "..." 表示
  - 空內容: 顯示為 "(empty)"

#### 3.3.3 第三層：Payload 詳細檢視界面
**功能描述**: 完整顯示選定訊息的 payload 內容

**介面佈局**:
```
┌─ Payload Detail ──────────────────────────────────────┐
│ Topic: sensors/temp                                   │
│ Time: 2024-01-20 10:30:01                            │
│ QoS: 0 | Retain: false                               │
├───────────────────────────────────────────────────────┤
│ {                                                     │
│   "temperature": 25.5,                              │
│   "unit": "C",                                       │
│   "sensor_id": "TEMP001",                           │
│   "location": "Living Room",                         │
│   "battery": 85,                                     │
│   "timestamp": 1705734601,                          │
│   "metadata": {                                      │
│     "version": "1.0",                               │
│     "calibration": {                                │
│       "offset": 0.1,                               │
│       "scale": 1.0                                  │
│     }                                                │
│   }                                                  │
│ }                                                     │
│                                                      │
│                                                      │
└───────────────────────────────────────────────────────┘
Line 1-18 of 18 | JSON Mode: Full Depth | Size: 245 bytes
[ESC]back [j]son-depth [c]opy [↑↓]scroll [PgUp/PgDn]page [h]elp
```

**功能特性**:
- **靜態顯示**: 不自動更新，避免使用者閱讀時內容跳動
- **JSON 美化**:
  - 自動偵測 JSON 格式
  - 語法高亮顯示
  - 支援切換顯示深度 (第一層 / 完整深度)
- **導航功能**:
  - 方向鍵上下滾動單行
  - Page Up/Down 整頁滾動
  - Home/End 跳轉到開頭/結尾
- **顯示資訊**:
  - 顯示目前行數範圍
  - 顯示 payload 大小
  - 顯示 JSON 模式狀態

## 4. 即時更新機制

### 4.1 更新頻率
- **第一層和第二層**: 每 0.25 秒更新一次
- **第三層**: 靜態顯示，不自動更新

### 4.2 更新策略
- **增量更新**: 只更新變化的資料
- **雙緩衝**: 避免畫面閃爍
- **效能優化**: 限制每次查詢的資料量

### 4.3 更新觸發
- **定時器**: 使用 tokio::time::interval 實現
- **事件驅動**: MQTT 訊息到達時觸發部分更新
- **使用者操作**: 過濾條件變更時立即更新

## 5. 過濾器功能

### 5.1 Topic 過濾器
- **支援格式**: 正則表達式
- **範例**: 
  - `^sensors/` - 所有 sensors 開頭的主題
  - `temperature$` - 所有以 temperature 結尾的主題
  - `device[0-9]+` - 符合 device 後跟數字的主題

### 5.2 Payload 過濾器
- **支援格式**: 正則表達式
- **範例**:
  - `"temperature":\s*[0-9]+` - 包含溫度數值的 JSON
  - `error|warning` - 包含錯誤或警告的訊息
  - `^\{.*\}$` - 完整的 JSON 物件格式

### 5.3 時間過濾器
- **支援格式**: YYYY-MM-DD HH:MM:SS
- **範例**:
  - From: `2024-01-20 09:00:00`
  - To: `2024-01-20 18:00:00`
- **相對時間**: 支援 "Last 1 hour", "Last 24 hours" 等快速選項

## 6. JSON 顯示功能

### 6.1 自動偵測
- 自動偵測 payload 是否為有效的 JSON 格式
- 偵測到 JSON 時在狀態列顯示 [JSON] 標記

### 6.2 美化顯示
- **縮排**: 使用 2 個空格縮排
- **語法高亮**: 
  - 鍵值: 藍色
  - 字串值: 綠色
  - 數字值: 黃色
  - 布林值: 紅色
  - null: 灰色

### 6.3 深度控制
- **簡化模式** (用於第一、二層介面):
  - 只顯示第一層的鍵名，如 `{"temperature","unit","metadata"}`
  - 原始: `{"temperature": 25.5, "unit": "C", "metadata": {"sensor": "DHT22"}}`
  - 簡化: `{"temperature","unit","metadata"}`
- **第一層模式** (用於第三層介面): 只顯示第一層的鍵值對，巢狀物件顯示為 `{...}` 或 `[...]`
- **完整模式** (用於第三層介面): 顯示完整的 JSON 結構
- **快速切換**: 按 `j` 鍵快速切換模式

## 7. 鍵盤快捷鍵

### 7.1 全域快捷鍵
- `q`: 退出程式
- `h`: 顯示說明
- `r`: 手動刷新
- `Ctrl+C`: 強制退出

### 7.2 第一層快捷鍵
- `↑↓`: 選擇主題
- `Enter`: 進入選定主題
- `f`: 焦點切換到過濾器
- `Esc`: 離開過濾器輸入模式

### 7.3 第二層快捷鍵
- `↑↓`: 選擇訊息
- `Enter`: 查看訊息詳情
- `Esc`: 返回主題列表
- `f`: 焦點切換到過濾器
- `j`: 切換 JSON 預覽模式

### 7.4 第三層快捷鍵
- `↑↓`: 上下滾動
- `Page Up/Down`: 整頁滾動
- `Home/End`: 跳轉到開頭/結尾
- `Esc`: 返回訊息列表
- `j`: 切換 JSON 顯示深度
- `c`: 複製內容到剪貼簿 (如果支援)

## 8. 效能需求

### 8.1 記憶體使用
- **最大記憶體**: < 100MB (在處理 10萬條訊息時)
- **緩存策略**: LRU 快取機制
- **垃圾回收**: 定期清理過期資料

### 8.2 回應時間
- **介面刷新**: < 50ms
- **過濾操作**: < 200ms
- **資料庫查詢**: < 100ms

### 8.3 吞吐量
- **MQTT 訊息處理**: > 1000 msg/sec
- **資料庫寫入**: > 500 insert/sec
- **UI 更新頻率**: 4 FPS (每 0.25 秒)

## 9. 錯誤處理

### 9.1 連接錯誤
- MQTT broker 無法連接時顯示錯誤狀態
- 自動重連機制，最多重試 10 次
- 重連間隔: 1, 2, 4, 8, 16 秒 (指數退避)

### 9.2 資料庫錯誤
- SQLite 檔案權限錯誤
- 磁碟空間不足警告
- 資料庫損壞時的修復機制

### 9.3 格式錯誤
- 非法的正則表達式輸入
- 無效的時間格式輸入
- 惡意或超大 payload 的處理

## 10. 設定檔案

### 10.1 設定檔位置
- **預設位置**: `./config.toml`
- **環境變數**: `MQTT_LOG_VIEWER_CONFIG`

### 10.2 設定項目
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

## 11. 測試計畫

### 11.1 單元測試
- MQTT 連接和訊息處理
- 資料庫 CRUD 操作
- 過濾器邏輯
- JSON 格式化功能

### 11.2 整合測試
- MQTT -> DB -> UI 完整流程
- 大量訊息處理測試
- 長時間運行穩定性測試

### 11.3 效能測試
- 記憶體洩漏檢測
- 高頻訊息處理測試
- UI 回應時間測試

## 12. 部署需求

### 12.1 系統需求
- **作業系統**: Windows 10+, Linux, macOS
- **記憶體**: 最少 50MB 可用記憶體
- **磁碟**: 最少 10MB 可用空間 (不含日誌)

### 12.2 依賴項目
- MQTT Broker (如 Mosquitto)
- 可寫入的本地目錄 (存放資料庫檔案)

### 12.3 編譯目標
```
cargo build --release
```

## 13. 未來擴展

### 13.1 短期目標 (v1.1)
- 匯出功能 (CSV, JSON 格式)
- 主題統計圖表
- 搜尋歷史記錄

### 13.2 中期目標 (v1.2)
- 多 broker 支援
- 訊息重放功能
- 警告和通知機制

### 13.3 長期目標 (v2.0)
- Web 介面版本
- 分散式部署支援
- 進階分析功能