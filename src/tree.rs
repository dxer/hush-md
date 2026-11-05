//! 我的文档 as a page tree: parent links kept in the
//! index, the sidebar's expand state, 新建子页面, 移动到 and its dialog, and
//! the ancestor chain used by the header breadcrumbs.

use crate::app::Workspace;
use crate::settings::display_title;
use crate::editor::view::SubPage;
use crate::input::{TextField, TextFieldEvent};
use crate::store::Entry;
use crate::theme::{self, Theme, craft};
use crate::dialogs::{close_button, dialog, primary_button, scrim, secondary_button};
use crate::ui::{ROW_RADIUS, icon, ui_text};
use gpui::{prelude::FluentBuilder, *};
use std::collections::{HashMap, HashSet};

/// One row of the sidebar's 我的文档 tree: a folder, or a note.
#[derive(Clone)]
pub enum Node<'a> {
    /// A folder in the vault. `path` is vault-relative and is also its key
    /// in the sidebar's expanded set.
    Folder { path: String, name: String, depth: usize },
    Note { entry: &'a Entry, depth: usize },
}

/// The vault's notes as the sidebar shows them: real folders, with notes in
/// them. Folder rows come from `vault::folders()`; notes are placed by their
/// path, so the tree always matches the disk. Pinned notes are listed above
/// and not repeated here.
pub(crate) struct Tree<'a> {
    /// Notes by the folder they live in (`None` = vault root), in list order.
    notes: HashMap<Option<String>, Vec<&'a Entry>>,
    /// Folders by their parent (`None` = root), sorted by name.
    folders: HashMap<Option<String>, Vec<(String, String)>>,
    roots: Vec<&'a Entry>,
    kids: HashMap<&'a str, Vec<&'a Entry>>,
}

impl<'a> Tree<'a> {
    pub fn new(entries: &'a [Entry]) -> Self {
        Self::with_folders(entries, &[])
    }

    pub fn with_folders(entries: &'a [Entry], folders: &[crate::vault::Folder]) -> Self {
        let mut notes: HashMap<Option<String>, Vec<&'a Entry>> = HashMap::new();
        for e in entries {
            notes.entry(folder_of(&e.id)).or_default().push(e);
        }
        let mut fs: HashMap<Option<String>, Vec<(String, String)>> = HashMap::new();
        for f in folders {
            fs.entry(f.parent.clone()).or_default().push((f.name.clone(), f.path.clone()));
        }
        for v in fs.values_mut() {
            v.sort_by_key(|a| a.0.to_lowercase());
        }
        // The 移动到 dialog and breadcrumbs still work on note-parent links;
        // this keeps their shape until they are rewritten for folders.
        let ids: HashSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        let parent = |e: &'a Entry| e.meta.parent.as_deref().filter(|p| ids.contains(p) && *p != e.id);
        let mut kids: HashMap<&str, Vec<&Entry>> = HashMap::new();
        for e in entries {
            if let Some(p) = parent(e) {
                kids.entry(p).or_default().push(e);
            }
        }
        for v in kids.values_mut() {
            v.sort_by_key(|e| e.meta.sibling_key());
        }
        let roots = entries.iter().filter(|e| parent(e).is_none()).collect();
        Tree { notes, folders: fs, roots, kids }
    }

    /// Rows in display order, descending into folders the caller says are open.
    pub fn nodes(&self, open: impl Fn(&str) -> bool) -> Vec<Node<'a>> {
        let mut out = vec![];
        self.push_level(None, 0, &open, &mut out);
        out
    }

    fn push_level(&self, folder: Option<&str>, depth: usize, open: &impl Fn(&str) -> bool, out: &mut Vec<Node<'a>>) {
        // Folders first, then the notes that sit directly in this one.
        if let Some(list) = self.folders.get(&folder.map(str::to_string)) {
            for (name, path) in list {
                out.push(Node::Folder { path: path.clone(), name: name.clone(), depth });
                if open(path) {
                    self.push_level(Some(path), depth + 1, open, out);
                }
            }
        }
        if let Some(list) = self.notes.get(&folder.map(str::to_string)) {
            for e in list {
                out.push(Node::Note { entry: e, depth });
            }
        }
    }

    /// Notes whose `meta.parent` is `id` (the 移动到 dialog's view).
    pub fn children(&self, id: Option<&str>) -> &[&'a Entry] {
        match id {
            None => &self.roots,
            Some(id) => self.kids.get(id).map_or(&[], |v| v.as_slice()),
        }
    }

    /// `id` and every page under it.
    pub fn subtree(&self, id: &str) -> Vec<String> {
        let mut out = vec![id.to_string()];
        let mut i = 0;
        while i < out.len() {
            let kids: Vec<String> = self.children(Some(&out[i])).iter().map(|e| e.id.clone()).collect();
            out.extend(kids);
            i += 1;
        }
        out
    }
}

/// The folder a note lives in, from its vault-relative path.
pub fn folder_of(id: &str) -> Option<String> {
    id.rsplit_once('/').map(|(d, _)| d.to_string())
}

/// The 移动到 dialog.
pub struct MoveDialog {
    /// Pages being moved (each with its subtree).
    pub ids: Vec<String>,
    /// Chosen destination: `Some(None)` is the top level of 我的文档.
    pub target: Option<Option<String>>,
    pub expanded: HashSet<String>,
    pub search: Entity<TextField>,
    pub scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

impl Workspace {
    /// Ancestors of `id`, top level first.
    pub(crate) fn ancestors(&self, id: &str) -> Vec<&Entry> {
        let mut out: Vec<&Entry> = vec![];
        let mut cur = self.entries.iter().find(|e| e.id == id);
        while let Some(p) = cur.and_then(|e| e.meta.parent.as_deref()).and_then(|p| self.entries.iter().find(|e| e.id == p)) {
            if p.id == id || out.iter().any(|e| e.id == p.id) {
                break;
            }
            out.push(p);
            cur = Some(p);
        }
        out.reverse();
        out
    }

    /// Expand the sidebar down to `id`.
    pub(crate) fn reveal_in_tree(&mut self, id: &str) {
        let ids: Vec<String> = self.ancestors(id).iter().map(|e| e.id.clone()).collect();
        self.expanded.extend(ids);
    }

    /// Hand the open document its child pages (for 子页面列表 blocks): the
    /// notes in the folder that shares its name.
    pub(crate) fn sync_sub_pages(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(ed)) = (self.current.clone(), self.editor.clone()) else { return };
        let stem = hush_core::vault::path_stem_of(&id);
        let pages = self
            .entries
            .iter()
            .filter(|e| crate::tree::folder_of(&e.id).as_deref() == Some(stem.as_str()))
            .map(|e| SubPage { id: e.id.clone(), title: e.title.clone(), icon: e.icon.clone() })
            .collect();
        ed.update(cx, |e, cx| e.set_sub_pages(pages, cx));
        self.sync_links(cx);
    }

    /// The row "+" → 文档: a new page under `parent`, opened.
    pub fn new_child(&mut self, parent: &str, window: &mut Window, cx: &mut Context<Self>) {
        let p = parent.to_string();
        if !self.save_before(cx, move |w, window, cx| w.new_child(&p, window, cx)) {
            return;
        }
        let made = self.store.create().and_then(|(id, _)| self.store.set_parent(&id, Some(parent)).map(|_| id));
        match made {
            Ok(id) => {
                self.entries = self.store.list();
                self.sort();
                self.expanded.insert(parent.to_string());
                self.open(&id, window, cx);
            }
            Err(e) => self.status = crate::app::SaveStatus::Failed(crate::localized_format!("新建失败：{}", "Could not create document: {}", crate::settings::diagnostic(&e.to_string()))),
        }
        cx.notify();
    }

    pub(crate) fn open_move(&mut self, ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.row_menu = None;
        let search = cx.new(|cx| {
            let mut f = TextField::new("搜索目标位置", cx);
            f.placeholder_color = Theme::text_placeholder();
            f
        });
        let subs = vec![cx.subscribe_in(&search, window, |w, _, ev: &TextFieldEvent, window, cx| {
            if let TextFieldEvent::Cancel = ev {
                w.close_move(window, cx);
            }
            cx.notify();
        })];
        window.focus(&search.focus_handle(cx));
        self.move_dlg = Some(MoveDialog { ids, target: None, expanded: HashSet::new(), search, scroll: ScrollHandle::new(), _subs: subs });
        cx.notify();
    }

    fn close_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.move_dlg = None;
        if let Some(ed) = &self.editor {
            ed.read(cx).focus(window);
        }
        cx.notify();
    }

    /// 移动到: hang each page under `target` (`None`: top level), skipping
    /// moves into its own subtree.
    /// 移动到: put each note's file in `target` (a folder path; `None` = the
    /// vault root). Moving a note into a folder it is already in does nothing.
    pub(crate) fn move_docs(&mut self, ids: &[String], target: Option<String>, cx: &mut Context<Self>) {
        let folder = target.unwrap_or_default();
        if !folder.is_empty() {
            self.expanded.insert(folder.clone());
        }
        let mut moved: Vec<(String, String)> = vec![];
        for id in ids {
            match self.store.move_to_folder(id, &folder) {
                Ok(new_id) => {
                    if new_id != *id {
                        moved.push((id.clone(), new_id));
                    }
                }
                Err(e) => self.status = crate::app::SaveStatus::Failed(crate::localized_format!("移动失败：{}", "Could not move document: {}", crate::settings::diagnostic(&e.to_string()))),
            }
        }
        // A moved note's id is its new path: the open document, the selection
        // and the list all follow the file.
        for (old, new) in &moved {
            self.retarget(old, new);
        }
        self.entries = self.store.list();
        self.folders = self.store.folders();
        self.sort();
        self.sync_sub_pages(cx);
        self.show_toast(crate::settings::text("移动成功"), cx);
        cx.notify();
    }

    /// The 移动到 dialog: 我的文档 on the left, its page tree on the right.
    /// Quiet selection fills, a 30pt search field, the modal buttons.
    pub(crate) fn render_move_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dlg = self.move_dlg.as_ref()?;
        let tree = Tree::new(&self.entries);
        let hidden: HashSet<String> = dlg.ids.iter().flat_map(|id| tree.subtree(id)).collect();
        let q = dlg.search.read(cx).text.trim().to_lowercase();
        let border = Theme::line_border();

        let search = div()
            .mt(px(12.))
            .mx(px(8.))
            .mb(px(10.))
            .h(px(30.))
            .flex_none()
            .pl(px(10.))
            .pr(px(8.))
            .flex()
            .items_center()
            .rounded(px(9.))
            .border_1()
            .border_color(Theme::line_border())
            .bg(Theme::bg_body())
            .child(icon("search", 15., Theme::text_caption()))
            .child(div().flex_1().min_w_0().ml(px(6.)).flex().child(dlg.search.clone()));
        // The location column reads like a sidebar: a grey surface and
        // a quiet pill for the selected location.
        let left = div()
            .w(px(240.))
            .flex_none()
            .flex()
            .flex_col()
            .bg(Theme::bg_overlay())
            .rounded_l(px(9.))
            .child(search)
            .child(
                div()
                    .mx(px(8.))
                    .h(px(30.))
                    .pl(px(10.))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(9.))
                    .bg(Theme::fill_hover_strong())
                    .child(icon("library", 15., Theme::text_code()))
                    .child(div().ml(px(8.)).text_size(px(13.)).line_height(px(20.)).child(crate::settings::tr("我的文档", "My documents"))),
            );

        let row = |key: String, selected: bool| {
            div()
                .id(SharedString::from(key))
                .h(px(30.))
                .mb(px(2.))
                .pr(px(5.))
                .flex()
                .items_center()
                .rounded(px(ROW_RADIUS))
                .cursor_pointer()
                .when(selected, |d| d.bg(Theme::fill_hover_strong()))
                .when(!selected, |d| d.hover(|s| s.bg(Theme::fill_hover())))
        };
        let title = |t: String| div().ml(px(9.)).flex_1().min_w_0().text_size(px(13.)).line_height(px(20.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(t));
        let page_icon = |e: &Entry| match &e.icon {
            Some(em) => div().size(px(16.)).flex_none().flex().items_center().justify_center().text_size(px(14.)).child(em.clone()).into_any_element(),
            None => icon("doc", 16., Theme::text_caption()).into_any_element(),
        };
        let mut list = div().id("move-tree").flex_1().min_w_0().px(px(8.)).py(px(12.)).overflow_y_scroll().track_scroll(&dlg.scroll);
        if q.is_empty() {
            list = list.child(
                row("move-root".into(), dlg.target == Some(None))
                    .pl(px(16.))
                    .child(icon("library", 16., Theme::text_caption()))
                    .child(title(crate::settings::text("我的文档").into()))
                    .on_click(cx.listener(|w, _, _, cx| {
                        if let Some(d) = w.move_dlg.as_mut() {
                            d.target = Some(None);
                        }
                        cx.notify();
                    })),
            );
            // The vault's folders, as they are on disk. A note cannot be
            // moved into the folder it already lives in.
            let tree = Tree::with_folders(&self.entries, &self.folders);
            let _ = &tree;
            for folder in &self.folders {
                let depth = folder.path.matches('/').count();
                let pick = folder.path.clone();
                let open = dlg.expanded.contains(&folder.path);
                let kids = self.folders.iter().any(|f| f.parent.as_deref() == Some(folder.path.as_str()));
                let toggle = folder.path.clone();
                let here = dlg.target.as_ref().is_some_and(|t| t.as_deref() == Some(folder.path.as_str()));
                list = list.child(
                    row(format!("move-folder-{}", folder.path), here)
                        .pl(px(16. + 20. * depth as f32))
                        .child(
                            div()
                                .id(SharedString::from(format!("move-arrow-{}", folder.path)))
                                .size(px(20.))
                                .p(px(5.))
                                .mr(px(8.))
                                .flex_none()
                                .rounded(px(5.))
                                .when(kids, |d| {
                                    d.hover(|s| s.bg(Theme::fill_hover_strong()))
                                        .child(icon("tree-triangle", 10., theme::c(craft::N500)).when(open, |s| s.with_transformation(Transformation::rotate(radians(std::f32::consts::FRAC_PI_2)))))
                                        .on_click(cx.listener(move |w, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(d) = w.move_dlg.as_mut()
                                                && !d.expanded.remove(&toggle) {
                                                    d.expanded.insert(toggle.clone());
                                                }
                                            cx.notify();
                                        }))
                                }),
                        )
                        .child(icon("folder", 16., Theme::text_caption()))
                        .child(title(folder.name.clone()))
                        .on_click(cx.listener(move |w, _, _, cx| {
                            if let Some(d) = w.move_dlg.as_mut() {
                                d.target = Some(Some(pick.clone()));
                            }
                            cx.notify();
                        })),
                );
            }
        } else {
            let hits: Vec<&Entry> = self.entries.iter().filter(|e| !hidden.contains(&e.id) && display_title(&e.title).to_lowercase().contains(&q)).collect();
            if hits.is_empty() {
                list = list.child(div().pt(px(80.)).flex().justify_center().text_size(px(13.)).text_color(Theme::text_caption()).child(crate::settings::text("无搜索结果")));
            }
            for e in hits {
                let pick = e.id.clone();
                list = list.child(
                    row(format!("move-hit-{}", e.id), dlg.target.as_ref().is_some_and(|t| t.as_deref() == Some(e.id.as_str())))
                        .pl(px(16.))
                        .child(page_icon(e))
                        .child(title(display_title(&e.title)))
                        .on_click(cx.listener(move |w, _, _, cx| {
                            if let Some(d) = w.move_dlg.as_mut() {
                                d.target = Some(Some(pick.clone()));
                            }
                            cx.notify();
                        })),
                );
            }
        }

        let ready = dlg.target.is_some();
        // 确定 waits for a target: the accent, faded, until one is picked.
        let ok = primary_button("move-ok", crate::settings::text("确定"));
        let ok = if ready {
            ok
        } else {
            let faded = theme::ca(craft::B500, 0.4);
            ok.bg(faded).cursor_default().hover(move |s| s.bg(faded)).active(move |s| s.bg(faded))
        };
        let dialog = dialog("move-dialog")
            .relative()
            .w(px(828.))
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().pt(px(24.)).pr(px(56.)).pl(px(24.)).text_size(px(15.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(crate::settings::text("从 我的文档 移动到")))
            .child(close_button("move-close").absolute().top(px(27.)).right(px(25.)).on_click(cx.listener(|w, _, window, cx| w.close_move(window, cx))))
            .child(
                div().px(px(24.)).mt(px(16.)).mb(px(24.)).child(
                    div()
                        .h(px(512.))
                        .flex()
                        .border_1()
                        .border_color(border)
                        .rounded(px(10.))
                        .child(left)
                        .child(div().w(px(1.)).h_full().flex_none().bg(border))
                        .child(list),
                ),
            )
            .child(
                div()
                    .px(px(24.))
                    .pb(px(24.))
                    .flex()
                    .justify_end()
                    .gap(px(12.))
                    .child(secondary_button("move-cancel", crate::settings::text("取消")).on_click(cx.listener(|w, _, window, cx| w.close_move(window, cx))))
                    .child(ok.on_click(cx.listener(|w, _, window, cx| {
                        let Some(d) = w.move_dlg.as_ref() else { return };
                        let Some(target) = d.target.clone() else { return };
                        let ids = d.ids.clone();
                        w.batch = None;
                        w.move_docs(&ids, target, cx);
                        w.close_move(window, cx);
                    }))),
            );
        Some(
            deferred(
                div()
                    .id("move-mask")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(scrim())
                    .on_mouse_down(MouseButton::Left, cx.listener(|w, _, window, cx| w.close_move(window, cx)))
                    .child(dialog),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}
