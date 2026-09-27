use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, Element, InteractiveElement, MouseButton, ParentElement, Pixels,
    Point, SharedString, Size, Styled, deferred, div, prelude::FluentBuilder, px,
};

use crate::theme::Theme;
use crate::views::glyph;

pub struct MenuItem {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub action: Rc<dyn Fn(&mut App)>,
    pub separator_before: bool,
    pub disabled: bool,
    pub selected: bool,
}

impl Clone for MenuItem {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            label: self.label.clone(),
            icon: self.icon,
            action: self.action.clone(),
            separator_before: self.separator_before,
            disabled: self.disabled,
            selected: self.selected,
        }
    }
}

pub fn context_menu(
    items: Vec<MenuItem>,
    anchor: Point<Pixels>,
    viewport: Size<Pixels>,
    dismiss: Rc<dyn Fn(&mut App)>,
    cx: &mut Context<impl Sized>,
) -> AnyElement {
    let p = Theme::of(cx).palette;
    let x = f32::from(anchor.x).clamp(4., (f32::from(viewport.width) - 248.).max(4.));
    let menu_height = (items.len() as f32 * 28. + 8.).min(f32::from(viewport.height));
    let y = f32::from(anchor.y).clamp(4., (f32::from(viewport.height) - menu_height).max(4.));
    let menu = items.into_iter().fold(
        div()
            .w(px(240.))
            .p(px(4.))
            .rounded(px(8.))
            .bg(p.glass_overlay())
            .border_1()
            .border_color(p.border)
            .shadow_lg(),
        |menu, item| {
            let action = item.action.clone();
            let color = if item.disabled { p.faint } else { p.text };
            let row = div()
                .id(item.id)
                .h(px(28.))
                .w_full()
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(color)
                .when(item.selected, |d| d.bg(p.selected()))
                .when(item.separator_before, |d| {
                    d.border_t_1().border_color(p.border)
                })
                .when(!item.disabled, |d| {
                    d.cursor_pointer().hover(|s| s.bg(p.glass_hover()))
                })
                .when_some(item.icon, |d, icon| d.child(glyph(icon, 14., color)))
                .child(item.label)
                .when(!item.disabled, |d| {
                    d.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        action(cx);
                    })
                });
            menu.child(row)
        },
    );
    deferred(
        div()
            .absolute()
            .w(viewport.width)
            .h(viewport.height)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| dismiss(cx)),
            )
            .child(div().absolute().left(px(x)).top(px(y)).child(menu)),
    )
    .with_priority(2)
    .into_any()
}
