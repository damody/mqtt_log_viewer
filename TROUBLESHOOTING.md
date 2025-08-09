# 故障排除指南

## 問題：MQTT 訊息沒有顯示在畫面上

我已經修復了資料庫查詢的問題，現在程式應該可以正確顯示 MQTT 訊息了。

### 修復內容

1. **資料庫查詢修復**
   - `get_topic_stats()` 現在會正確執行 SQL 查詢
   - 增加了詳細的偵錯日誌

2. **時間戳處理改進**
   - 支援 RFC3339 和 SQLite datetime 格式
   - 自動處理時間格式轉換

3. **日誌級別調整**
   - 設定為 DEBUG 級別，可看到詳細的執行資訊

### 測試步驟

#### 1. 編譯並執行程式
```bash
cd D:\Nobu\mqtt_log_view
cargo build
cargo run
```

#### 2. 檢查日誌輸出
程式執行時會顯示以下除錯資訊：
- MQTT 連接狀態
- 訊息插入到資料庫
- 資料庫查詢結果
- UI 更新狀態

#### 3. 測試 MQTT 訊息
使用 MQTT 客戶端發送測試訊息：

**mosquitto_pub (如果已安裝)**：
```bash
mosquitto_pub -h 127.0.0.1 -t "test/topic" -m '{"temperature": 25.5, "unit": "C"}'
```

**MQTTX 或其他 MQTT 客戶端**：
- Host: 127.0.0.1
- Port: 1883  
- Topic: test/topic
- Payload: {"temperature": 25.5, "unit": "C"}

### 預期的日誌輸出

如果一切正常，您應該看到類似的日誌：

```
INFO mqtt_log_view: Starting MQTT Log Viewer
DEBUG mqtt_log_view::db::repository: Database schema initialized  
INFO mqtt_log_view::mqtt::client: Connecting to MQTT broker and subscribing to all topics...
INFO mqtt_log_view::mqtt::client: Successfully subscribed to all topics (#)
DEBUG mqtt_log_view::mqtt::handler: Processing batch of 1 messages
DEBUG mqtt_log_view::db::repository: Inserting message: topic=test/topic, payload_len=35, timestamp=2024-...
DEBUG mqtt_log_view::db::repository: Message inserted with ID: 1
DEBUG mqtt_log_view::db::repository: Executing topic stats query
DEBUG mqtt_log_view::db::repository: Found 1 topics in database
DEBUG mqtt_log_view::db::repository: Processing topic: test/topic, count: 1, time: 2024-...
```

### 檢查要點

#### 1. MQTT Broker 是否運行？
確認本地有 MQTT broker 在 127.0.0.1:1883 運行（如 Mosquitto）

#### 2. 資料庫檔案是否建立？
檢查目錄下是否有 `mqtt_logs.db` 檔案建立

#### 3. 畫面更新
- 程式每 0.25 秒會自動重新整理畫面
- 新的主題應該會出現在第一層介面中

### 常見問題

#### Q: 畫面顯示 "0 topics"
**A:** 可能原因：
- MQTT broker 未啟動
- 沒有發送任何 MQTT 訊息
- 資料庫查詢失敗（檢查日誌）

#### Q: 連接狀態顯示 "Connecting" 或 "Disconnected"
**A:** 檢查：
- MQTT broker 是否在 127.0.0.1:1883 運行
- 防火牆是否阻擋連接
- 網路連接是否正常

#### Q: 程式崩潰或無法啟動
**A:** 檢查：
- 資料庫檔案權限
- 磁碟空間是否足夠
- 執行 `RUST_BACKTRACE=1 cargo run` 查看詳細錯誤

### 手動驗證

如果仍有問題，可以手動檢查資料庫：

```bash
# 安裝 sqlite3（如果尚未安裝）
# 然後執行：
sqlite3 mqtt_logs.db "SELECT * FROM messages;"
```

這會顯示資料庫中儲存的所有訊息。

### 需要更多協助？

如果問題仍然存在，請：
1. 執行 `RUST_LOG=debug cargo run`
2. 發送一個測試 MQTT 訊息
3. 複製完整的日誌輸出
4. 檢查是否有任何錯誤訊息