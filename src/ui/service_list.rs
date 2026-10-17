use crate::models::AppState;
use crate::ui::service_card::{CardActions, render_service_card};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, div, px};
use uuid::Uuid;

/// 卡片最小舒适宽度（含边距）：低于此值加列会挤压内容。
const CARD_MIN_WIDTH: f32 = 340.;
/// 列间距（gap_3 = 12px）。
const COLUMN_GAP: f32 = 12.;

/// 服务列表：按 next_reset_at 排序展示卡片，宽度跟随窗口宽度响应式分列。
/// 布局：按视口宽度计算列数（1~3 列），手动分行为 v_flex(h_flex(...)) 网格；
/// 每 tick 仅倒计时文本随状态变化而重建，卡片静态部分无独立 View 缓存。
pub fn render_service_list(
    state: &AppState,
    actions: &dyn Fn(Uuid) -> CardActions,
    viewport_width: f32,
    cx: &App,
) -> AnyElement {
    let mut services: Vec<_> = state.services.iter().collect();
    services.sort_by_key(|s| s.next_reset_at());

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
