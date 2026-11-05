//! The confirmation dialogs (删除, 释放永久删除) and the page helpers the
//! main pane's pages share (page header, empty state).
//! Deleting moves files to the system trash; the app keeps no trash of its own.

use crate::app::{Confirm, Workspace};
use crate::settings::display_title;
pub(crate) use crate::ui::{close_button, dialog, menu_label, primary_button, scrim, secondary_button};
use crate::theme::{self, Theme, craft};
use crate::ui::{TipExt, TitleBarExt, icon, ui_para, ui_text};
use gpui::{prelude::FluentBuilder, *};

/// The page header, as tall as the document's so the header pills sit in
/// the same place: a 17pt semibold view title, and the open-sidebar
/// button when the sidebar is hidden.
pub(crate) fn page_header(ws: &Workspace, title: &'static str, left: f32, cx: &mut Context<Workspace>) -> Stateful<Div> {
    div()
        .id("page-header")
        .h(px(crate::app::HEADER_H))
        .flex_none()
        .flex()
        .items_center()
        // Clear of the traffic lights, like the document header.
        .pl(if ws.sidebar_open { px(left) } else { px(theme::TRAFFIC_LIGHTS_W) })
        .pr(px(crate::ui::header_right_inset()))
        .title_bar()
        .when(!ws.sidebar_open && cfg!(target_os = "windows"), |d| d.child(div().mr(px(12.)).child(ws.header_navigation(cx))))
        .when(!ws.sidebar_open && !cfg!(target_os = "windows"), |d| {
            d.child(
                div()
                    .id("open-sidebar")
                    .no_drag()
                    .flex_none()
                    .ml(px(4.))
                    .mr(px(12.))
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover_strong()))
                    .tip(crate::settings::tr("展开侧栏", "Show sidebar"), Some(crate::shortcuts::hint("workspace::ToggleSidebar", "⌘ + ]")))
                    .child(icon("panel-left-open", 17., Theme::icon()))
                    .on_click(cx.listener(|w, _, _, cx| {
                        w.sidebar_open = true;
                        cx.notify();
                    })),
            )
        })
        .child(div().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(crate::settings::label(title)))
        .child(div().flex_1())
}

/// Centred empty-state illustration and caption.
pub(crate) fn empty_state(illus: &'static str, text: &'static str) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .pb(px(64.))
        .child(img(illus).size(px(120.)))
        .child(div().mt(px(16.)).text_size(px(13.)).line_height(px(20.)).text_color(Theme::text_caption()).child(text))
}

impl Workspace {
    /// The confirmation dialog: a modal (white, radius 16), a 15pt
    /// semibold title, one line of 13pt body, 取消 and the action button on
    /// the right.
    pub(crate) fn render_confirm(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.confirm.as_ref()?;
        let title_of = |id: &str| self.entries.iter().find(|e| e.id == id).map(|e| display_title(&e.title)).unwrap_or_default();
        let (title, body, action, danger) = match c {
            Confirm::Delete { ids, batch } => (
                if *batch || ids.len() > 1 { crate::localized_format!("是否删除所选的 {} 项？", "Delete the {} selected items?", ids.len()) } else { crate::localized_format!("是否删除： {}？", "Delete {}?", title_of(&ids[0])) },
                crate::settings::text("文件会移到系统回收站，可在访达 / 资源管理器中恢复").to_string(),
                crate::settings::text("删除"),
                true,
            ),
            Confirm::DeleteFolder { path } => (
                crate::localized_format!("是否删除文件夹 {} 及其中所有笔记？", "Delete the folder {} and every note in it?", path.rsplit('/').next().unwrap_or(path)),
                crate::settings::text("文件夹和其中的笔记会一起移到系统回收站，可在访达 / 资源管理器中恢复").to_string(),
                crate::settings::text("删除"),
                true,
            ),
            Confirm::DeletePermanent { ids, folder } => (
                match folder {
                    Some(path) => crate::localized_format!("是否彻底删除文件夹 {}？", "Permanently delete the folder {}?", path.rsplit('/').next().unwrap_or(path)),
                    None => crate::localized_format!("是否彻底删除所选的 {} 项？", "Permanently delete the {} selected items?", ids.len()),
                },
                crate::settings::text("该位置不支持回收站，删除后无法恢复").to_string(),
                crate::settings::text("彻底删除"),
                true,
            ),
            Confirm::Refs { show: false, .. } => (
                crate::settings::text("是否隐藏引用列表？").to_string(),
                crate::settings::text("选择隐藏后，协作者将无法查看本文的引用列表。如需重新显示，可点击文档右上角的更多按钮“...”，打开“显示引用列表”开关。").to_string(),
                crate::settings::text("隐藏"),
                false,
            ),
            Confirm::Refs { show: true, .. } => (crate::settings::text("是否显示引用列表？").to_string(), crate::settings::text("选择显示后，协作者可再次查看本文的引用列表。").to_string(), crate::settings::text("显示"), false),
            Confirm::SaveFailed => (
                crate::settings::text("文档保存失败").to_string(),
                crate::localized_format!("{}。可以重试，或先取消并通过右上角“...”→“下载为”导出一份副本；放弃修改将丢失本次未保存的内容。", "{}. Retry, or cancel and export a copy using More → Download as. Discarding changes will lose your unsaved work.", self.save_error()),
                crate::settings::text("重试"),
                false,
            ),
        };
        let no_ask = match c {
            Confirm::Refs { show: false, no_ask } => Some(*no_ask),
            _ => None,
        };
        let ok = primary_button("confirm-ok", action);
        let ok = if danger { ok.bg(Theme::danger()).hover(|s| s.bg(theme::c(craft::R600))).active(|s| s.bg(theme::c(craft::R700))) } else { ok };
        let dialog = dialog("confirm-dialog")
            .w(px(400.))
            .px(px(24.))
            .pt(px(20.))
            .pb(px(24.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child(ui_para(title)))
            .child(div().mt(px(8.)).text_size(px(13.)).line_height(px(20.)).text_color(Theme::text()).child(ui_para(body)))
            .child(
                div()
                    .mt(px(24.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().children(no_ask.map(|on| {
                        div()
                            .id("confirm-no-ask")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .on_click(cx.listener(|w, _, _, cx| {
                                if let Some(Confirm::Refs { no_ask, .. }) = &mut w.confirm {
                                    *no_ask = !*no_ask;
                                }
                                cx.notify();
                            }))
                            .child(crate::ui::checkbox(on))
                            .child(ui_text(crate::settings::text("不再提示")))
                    })))
                    .child(secondary_button("confirm-cancel", crate::settings::text("取消")).on_click(cx.listener(|w, _, _, cx| {
                        w.confirm = None;
                        w.after_save = None;
                        cx.notify();
                    })))
                    .when(matches!(c, Confirm::SaveFailed), |d| {
                        d.child(secondary_button("confirm-discard", crate::settings::text("放弃修改")).text_color(Theme::danger()).on_click(cx.listener(|w, _, window, cx| {
                            w.confirm = None;
                            w.resolve_save_failed(false, window, cx);
                        })))
                    })
                    .child(ok.on_click(cx.listener(|w, _, window, cx| {
                        let Some(c) = w.confirm.take() else { return };
                        match c {
                            Confirm::Delete { ids, batch } => {
                                if batch {
                                    w.batch = None;
                                }
                                w.delete_docs(&ids, window, cx);
                            }
                            Confirm::DeleteFolder { path } => w.delete_folder(&path, cx),
                            Confirm::DeletePermanent { ids, folder } => w.delete_permanently(&ids, folder.as_deref(), window, cx),
                            Confirm::Refs { show, no_ask } => {
                                if no_ask {
                                    let mut s = w.store.settings();
                                    s.refs_hide_no_ask = true;
                                    let _ = w.store.save_settings(&s);
                                }
                                w.set_refs_hidden(!show, cx);
                            }
                            Confirm::SaveFailed => w.resolve_save_failed(true, window, cx),
                        }
                        cx.notify();
                    }))),
            );
        Some(
            deferred(
                div()
                    .id("confirm-mask")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(scrim())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(dialog),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}
