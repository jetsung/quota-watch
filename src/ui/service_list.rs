use crate::models::{AppState, ServiceEntry};
use crate::ui::service_card::{CardActions, render_service_card};
use chrono::{DateTime, Utc};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ScrollHandle, div, px};
use std::cmp::Ordering;
use uuid::Uuid;

/// 卡片最小舒适宽度（含边距）：低于此值加列会挤压内容。
const CARD_MIN_WIDTH: f32 = 340.;
/// 列间距（gap_3 = 12px）。
const COLUMN_GAP: f32 = 12.;

/// 复合排序比较：置顶条目在前；置顶区内按 `pinned_at` 倒序（后置顶者在前，
/// `None` 因数据损坏回退为最末置顶位）；未置顶区按 `next_reset_at` 升序。
pub fn compare_entries(a: &ServiceEntry, b: &ServiceEntry) -> Ordering {
    match (a.is_pinned, b.is_pinned) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    if a.is_pinned {
        // 倒序：None 视为最早（最小时间戳），在倒序中回退为最末置顶位
        b.pinned_at
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
            .cmp(&a.pinned_at.unwrap_or(DateTime::<Utc>::MIN_UTC))
    } else {
        a.next_reset_at().cmp(&b.next_reset_at())
    }
}

/// 服务列表：按 next_reset_at 排序展示卡片，宽度跟随窗口宽度响应式分列。
/// 布局：按视口宽度计算列数（1~3 列），手动分行为 v_flex(h_flex(...)) 网格；
/// 每 tick 仅倒计时文本随状态变化而重建，卡片静态部分无独立 View 缓存。
pub fn render_service_list(
    state: &AppState,
    actions: &dyn Fn(Uuid) -> CardActions,
    viewport_width: f32,
    scroll_handle: &ScrollHandle,
    cx: &App,
) -> AnyElement {
    let mut services: Vec<_> = state.services.iter().collect();
    services.sort_by(|a, b| compare_entries(a, b));

    if services.is_empty() {
        return empty_placeholder();
    }

    // 可用宽度扣除列表左右内边距（p_4 = 16px×2）；
    // 列数 = 能放下多少张最小宽度卡片（含列间距），不设上限；
    // 多列时卡片在行内等宽均分。
    let available = viewport_width - 32.;
    let cols = ((available + COLUMN_GAP) / (CARD_MIN_WIDTH + COLUMN_GAP)).floor() as usize;
    let cols = cols.max(1);

    let children: Vec<AnyElement> = services
        .iter()
        .map(|entry| {
            let actions = actions(entry.id);
            render_service_card(entry, actions, cx, cols > 1).into_any_element()
        })
        .collect();

    let grid = if cols == 1 {
        v_flex()
            .w_full()
            .gap_3()
            .children(children)
            .into_any_element()
    } else {
        // AnyElement 不可 Clone，用迭代器按列宽切分出行
        let row_count = children.len().div_ceil(cols);
        let mut it = children.into_iter();
        let rows: Vec<AnyElement> = (0..row_count)
            .map(|_| {
                let row: Vec<AnyElement> = (0..cols).filter_map(|_| it.next()).collect();
                h_flex().w_full().gap_3().children(row).into_any_element()
            })
            .collect();
        v_flex().w_full().gap_3().children(rows).into_any_element()
    };

    div()
        .id("service-list")
        .track_scroll(scroll_handle)
        .w_full()
        .flex_grow_1()
        .overflow_y_scroll()
        .p_4()
        .child(grid)
        .into_any_element()
}

/// 空状态占位：图标 + 引导文案。
fn empty_placeholder() -> AnyElement {
    div()
        .w_full()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .gap_2()
                .items_center()
                .child(div().text_size(px(48.)).child("🕐"))
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .child("暂无服务"),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .child("点击右下角 “新建服务” 添加第一个服务"),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn entry(name: &str, interval_hours: f64) -> ServiceEntry {
        ServiceEntry {
            id: Uuid::new_v4(),
            service_name: name.into(),
            interval_hours,
            last_reset_time: Utc.with_ymd_and_hms(2026, 10, 25, 0, 0, 0).unwrap(),
            reminder_threshold_minutes: 60,
            secondary_reminder_interval_minutes: 5,
            is_muted: false,
            reset_config_mode: Default::default(),
            url: None,
            is_pinned: false,
            pinned_at: None,
        }
    }

    fn names(services: &[&ServiceEntry]) -> Vec<String> {
        services.iter().map(|s| s.service_name.clone()).collect()
    }

    #[test]
    fn later_pinned_comes_first_unpinned_by_next_reset() {
        // A 先置顶、B 后置顶 → B, A 在前；未置顶 C, D 按 next_reset_at 升序
        let mut a = entry("A", 5.0);
        let mut b = entry("B", 7.0);
        let c = entry("C", 3.0);
        let d = entry("D", 9.0);
        a.is_pinned = true;
        a.pinned_at = Some(Utc.with_ymd_and_hms(2026, 10, 25, 10, 0, 0).unwrap());
        b.is_pinned = true;
        b.pinned_at = Some(Utc.with_ymd_and_hms(2026, 10, 25, 11, 0, 0).unwrap());

        let mut services: Vec<&ServiceEntry> = vec![&d, &c, &b, &a];
        services.sort_by(|x, y| compare_entries(x, y));
        assert_eq!(names(&services), vec!["B", "A", "C", "D"]);
    }

    #[test]
    fn unpin_restores_next_reset_order() {
        // A 取消置顶后按 next_reset_at 回归未置顶序列
        let mut a = entry("A", 5.0);
        let c = entry("C", 3.0);
        let d = entry("D", 9.0);
        a.is_pinned = false;
        a.pinned_at = None;

        let mut services: Vec<&ServiceEntry> = vec![&d, &c, &a];
        services.sort_by(|x, y| compare_entries(x, y));
        assert_eq!(names(&services), vec!["C", "A", "D"]);
    }

    #[test]
    fn pinned_with_none_pinned_at_falls_to_last_pinned_slot() {
        // 数据损坏：is_pinned=true 而 pinned_at=None → 排在置顶区最末
        let mut a = entry("A", 5.0);
        let mut bad = entry("BAD", 7.0);
        a.is_pinned = true;
        a.pinned_at = Some(Utc.with_ymd_and_hms(2026, 10, 25, 10, 0, 0).unwrap());
        bad.is_pinned = true;
        bad.pinned_at = None;

        let mut services: Vec<&ServiceEntry> = vec![&bad, &a];
        services.sort_by(|x, y| compare_entries(x, y));
        assert_eq!(names(&services), vec!["A", "BAD"]);
    }
}
