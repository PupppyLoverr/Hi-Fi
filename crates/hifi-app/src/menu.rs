use gpui::{
    AnyElement, Context, Element, InteractiveElement, MouseButton, ParentElement, SharedString,
    Styled, div, px,
};

use crate::theme::Theme;

/// A deferred menu entry shared by pane peek and sidebar actions.
#[derive(Clone)]
pub struct ContextMenuItem {
    pub id: SharedString,
    pub label: SharedString,
}

/// Render a compact deferred menu surface using the Lumen chrome metrics.
pub fn context_menu(items: Vec<ContextMenuItem>, cx: &mut Context<impl Sized>) -> AnyElement {
    let p = Theme::of(cx).palette;
    let menu = items.into_iter().fold(
        div()
            .w(px(220.))
            .p(px(4.))
            .rounded(px(8.))
            .bg(p.glass_overlay())
            .border_1()
            .border_color(p.border)
            .shadow_lg(),
        |menu, item| {
            menu.child(
                div()
                    .id(item.id)
                    .h(px(28.))
                    .w_full()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .text_color(p.text)
                    .hover(|s| s.bg(p.glass_hover()))
                    .on_mouse_down(MouseButton::Left, |_: &_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(item.label),
            )
        },
    );
    gpui::deferred(menu).with_priority(1).into_any()
}
