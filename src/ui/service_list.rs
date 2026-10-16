use crate::models::AppState;
use crate::ui::service_card::{CardActions, render_service_card};
use gpui_kit::base::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{App, AnyElement, div, px};
use uuid::Uuid;

/// 服务列表：按 next_reset_at 排序展示卡片。
/// 当前实现为普通滚动列表（div + overflow_y_scroll）：每 tick 仅倒计时文本随状态变化而重建，
/// 卡片静态部分无独立 View 缓存（gpui 的 RenderOnce 语义下重建成本可忽略）。
/// 50+ 条目时如帧时间不达标，可升级为 `gpui_kit::component::virtual_list` 做可视区虚拟化。
pub fn render_service_list(state: &AppState, actions: &dyn Fn(Uuid) -> CardActions, cx: &App) -> AnyElement {
    let sorted: Vec<_> = state
        .services
        .iter()
        .collect::<Vec<_>>()
        .into_iter()
        .collect();
    let mut services: Vec<_> = sorted;
    services.sort_by_key(|s| s.next_reset_at());

    let children: Vec<_> = services
        .iter()
        .map(|entry| {
            let actions = actions(entry.id);
            let el = render_service_card(entry, actions, cx);
            el.into_any_element()
        })
        .collect();

    if services.is_empty() {
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
                    .child(
                        div()
                            .text_size(px(48.))
                            .child("🕐"),
                    )
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
    } else {
        div()
            .id("service-list")
            .w_full()
            .flex_grow_1()
            .overflow_y_scroll()
            .p_4()
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .children(children),
            )
            .into_any_element()
    }
}
