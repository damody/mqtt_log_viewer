# 增量渲染改進

## 問題描述
原始版本的 TUI 介面會每 0.25 秒完全重繪整個畫面，導致：
1. 畫面閃爍現象
2. 不必要的 CPU 使用
3. 視覺體驗不佳

## 解決方案：增量渲染

### 核心概念
實作差異檢測和增量更新機制，只重繪真正發生變化的部分。

### 技術實現

#### 1. 狀態比較機制
```rust
// 在 App 中儲存前一個狀態
prev_filter_state: Option<FilterState>,
prev_status_bar_state: Option<StatusBarState>,
prev_topic_list_state: Option<TopicListState>,
needs_full_redraw: bool,
```

#### 2. 差異檢測方法
- `states_equal_filter()` - 比較過濾器狀態
- `states_equal_topics()` - 比較主題列表狀態  
- `states_equal_status()` - 比較狀態欄

#### 3. 增量渲染 API
每個 UI 元件都新增增量渲染方法：

##### FilterBar
```rust
pub fn render_incremental(
    state: &FilterState, 
    prev_state: Option<&FilterState>, 
    row: u16, 
    terminal_width: u16
) -> Result<()>
```

##### StatusBar  
```rust
pub fn render_incremental(
    state: &StatusBarState, 
    prev_state: Option<&StatusBarState>, 
    row: u16, 
    terminal_width: u16
) -> Result<()>
```

##### TopicListView
```rust
pub fn render_incremental(
    state: &TopicListState,
    prev_state: Option<&TopicListState>,
    start_row: u16, 
    end_row: u16, 
    terminal_width: u16
) -> Result<()>
```

### 優化細節

#### 1. 畫面清除策略
- **全重繪**: 只在初始化、終端大小變更時執行
- **增量更新**: 只清除並重繪變化的行

#### 2. 精確比較
- **過濾器**: 比較所有輸入欄位和編輯狀態
- **主題列表**: 比較主題內容、選擇索引、滾動位置
- **狀態欄**: 比較連接狀態、統計數據、更新時間

#### 3. 游標管理  
- 只在需要時顯示/隱藏游標
- 精確定位游標到編輯位置

### 渲染流程

#### 原始流程
```
每 0.25 秒:
├── 清除整個畫面 (Clear::All)
├── 重繪過濾器
├── 重繪主題列表  
└── 重繪狀態欄
```

#### 優化後流程
```
每 0.25 秒:
├── 比較狀態變化
├── 只重繪變化的過濾器行
├── 只重繪變化的主題列表項
└── 只重繪變化的狀態欄行
```

### 效能優勢

1. **消除閃爍**: 不再清除整個畫面
2. **減少 CPU**: 只處理變化的部分
3. **提升響應**: 更流暢的使用體驗
4. **節省頻寬**: 減少終端輸出量

### 兼容性

- 保留原始的 `render()` 方法確保向後兼容
- 新增 `render_incremental()` 方法提供增量功能
- 支援強制全重繪（如終端大小變更時）

## 使用方式

### 啟用增量渲染
增量渲染在 `App::render_topic_list_incremental()` 中自動啟用：

```rust
if force_redraw || topics_changed {
    if force_redraw {
        // 全重繪
        TopicListView::render(...)
    } else {
        // 增量渲染 
        TopicListView::render_incremental(...)
    }
}
```

### 觸發全重繪的情況
- 程式啟動
- 終端大小變更
- 切換介面層級

## 測試建議

1. **視覺測試**: 確認畫面不再閃爍
2. **功能測試**: 驗證所有 UI 元素正常更新
3. **效能測試**: 監控 CPU 使用率降低
4. **邊界測試**: 測試大量數據更新情況

## 未來改進

1. 更精細的行級別差異檢測
2. 支援部分行內容更新
3. 可配置的更新頻率
4. 記憶體使用優化