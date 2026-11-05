//! Workspace window: two panes — the document list on the left
//! (see `sidebar.rs`) and the editor on the right, with the
//! 64px top bar (breadcrumb title + save status) above the page.

use crate::docinfo::DocInfo;
use crate::docmenu::DocSub;
use crate::editor::view::{Editor, EditorEvent};
use crate::input::{TextField, TextFieldEvent};
use crate::settings::display_title;
use crate::store::Entry;
use crate::shared_store::SharedStore;
use crate::theme::{self, Theme, craft};
use crate::ui::{TipExt, TipSide, TitleBarExt, ToastKind, icon, ui_text};
use gpui::{prelude::FluentBuilder, *};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

actions!(workspace, [NewDoc, ImportDoc, ToggleSidebar, FocusSearch, FindInDoc, ReplaceInDoc, OpenSettings, OpenAbout, GoHome, CloseWindow, Quit,
    HideApp, HideOthers, ShowAll, Minimize, Zoom, ToggleFullScreen, ToggleSource]);

/// The menu bar, in the interface language.
pub fn set_menus(cx: &mut App) {
    use crate::settings::tr;
    cx.set_menus(vec![
        Menu {
            name: "hush.md".into(),
            items: vec![
                MenuItem::action(tr("关于 hush.md", "About hush.md"), OpenAbout),
                MenuItem::separator(),
                MenuItem::action(tr("设置…", "Settings…"), OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu(tr("服务", "Services"), SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(tr("隐藏 hush.md", "Hide hush.md"), HideApp),
                MenuItem::action(tr("隐藏其他", "Hide Others"), HideOthers),
                MenuItem::action(tr("全部显示", "Show All"), ShowAll),
                MenuItem::separator(),
                MenuItem::action(tr("退出 hush.md", "Quit hush.md"), Quit),
            ],
        },
        Menu {
            name: tr("文件", "File").into(),
            items: vec![
                MenuItem::action(tr("新建文档", "New Document"), NewDoc),
                MenuItem::action(tr("新建笔记窗口", "New Note Window"), crate::quick_note::QuickNew),
                MenuItem::action(tr("导入为文档…", "Import as Document…"), ImportDoc),
                MenuItem::separator(),
                MenuItem::action(tr("关闭窗口", "Close Window"), CloseWindow),
            ],
        },
        // The standard Edit menu: its items act on the focused editor, and
        // macOS adds 听写 and 表情与符号 to it.
        Menu {
            name: tr("编辑", "Edit").into(),
            items: {
                use crate::editor::keymap as k;
                vec![
                    MenuItem::os_action(tr("撤销", "Undo"), k::Undo, OsAction::Undo),
                    MenuItem::os_action(tr("重做", "Redo"), k::Redo, OsAction::Redo),
                    MenuItem::separator(),
                    MenuItem::os_action(tr("剪切", "Cut"), k::Cut, OsAction::Cut),
                    MenuItem::os_action(tr("拷贝", "Copy"), k::Copy, OsAction::Copy),
                    MenuItem::os_action(tr("粘贴", "Paste"), k::Paste, OsAction::Paste),
                    MenuItem::os_action(tr("全选", "Select All"), k::SelectAll, OsAction::SelectAll),
                ]
            },
        },
        Menu {
            name: tr("窗口", "Window").into(),
            items: vec![
                MenuItem::action(tr("最小化", "Minimize"), Minimize),
                MenuItem::action(tr("缩放", "Zoom"), Zoom),
                MenuItem::separator(),
                MenuItem::action(tr("进入 / 退出全屏幕", "Enter / Exit Full Screen"), ToggleFullScreen),
            ],
        },
    ]);
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-n", NewDoc, None),
        KeyBinding::new("cmd-shift-n", crate::quick_note::QuickNew, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-shift-i", ImportDoc, None),
        KeyBinding::new("cmd-alt-home", GoHome, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-]", ToggleSidebar, None),
        KeyBinding::new("cmd-\\", ToggleSidebar, None),
        KeyBinding::new("cmd-j", FocusSearch, None),
        KeyBinding::new("cmd-f", FindInDoc, None),
        KeyBinding::new("cmd-shift-h", ReplaceInDoc, None),
        // Source ⇄ rendered view (⌘/ is the shortcut panel).
        KeyBinding::new("cmd-shift-m", ToggleSource, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    // macOS app and window keys. ⌥⌘H (隐藏其他) stays with the editor's
    // 应用上次的颜色, the Feishu shortcut; 隐藏其他 is in the menu. On Windows
    // the system handles these (Win+↓, Alt+F4), and F11 is full screen.
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);
    // Windows: F11 for full screen, and Ctrl+H for 查找和替换 as in Office
    // and Notepad (written cmd-h: `shortcuts::windows_binding` maps cmd to
    // ctrl and drops a literal ctrl-h, Cocoa's backspace).
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([KeyBinding::new("f11", ToggleFullScreen, None), KeyBinding::new("cmd-h", ReplaceInDoc, None)]);
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(handle) = cx.active_window().and_then(|w| w.downcast::<Workspace>()) {
            // Key dispatch already borrows this window; close after it returns.
            cx.defer(move |cx| {
                if let Err(e) = handle.update(cx, |ws, window, cx| {
                    crate::quick_note::persist(cx);
                    if ws.save_before(cx, |_, window, _| window.remove_window()) { window.remove_window(); }
                }) { eprintln!("无法关闭笔记窗口：{e}"); }
            });
        }
    });
    cx.on_action(|_: &Quit, cx| {
        // The open document is saved first; when that fails its dialog asks
        // (重试 / 放弃修改) and quits afterwards.
        let ready = cx
            .windows()
            .into_iter()
            .filter_map(|w| w.downcast::<Workspace>())
            .all(|w| w.update(cx, |ws, _, cx| ws.save_before(cx, |_, _, cx| cx.quit())).unwrap_or(true));
        if ready {
            cx.quit();
        }
    });
    crate::modes::bind_keys(cx);
}

#[derive(Clone, Debug, PartialEq)]
pub enum SaveStatus {
    /// Freshly opened, nothing saved this session: show the modification time.
    Idle,
    Saved,
    Saving,
    Failed(String),
}

/// Right-click / "…" menu on a sidebar row.
#[derive(Clone, Debug)]
pub struct RowMenu {
    pub id: String,
    pub pos: Point<Pixels>,
    /// Which corner of the menu sits at `pos`.
    pub corner: Corner,
    /// 下载 submenu open.
    pub sub: bool,
    /// The 批量操作 menu for the checked rows.
    pub batch: bool,
    /// A folder row's menu (the id is its vault-relative path).
    #[doc(hidden)]
    pub folder: bool,
}

/// What the main pane shows: the open document, 主页 or 设置.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Doc,
    Home,
    Settings,
}

/// A confirmation dialog and the pages it is about.
#[derive(Clone, Debug)]
pub enum Confirm {
    /// 删除 (to the system trash); `batch` for the 批量操作 selection.
    Delete { ids: Vec<String>, batch: bool },
    /// Delete a whole folder and the notes inside it.
    DeleteFolder { path: String },
    /// The volume has no trash: a second 确认 before deleting for good.
    DeletePermanent { ids: Vec<String>, folder: Option<String> },
    /// 隐藏引用列表 (with 不再提示) / 显示引用列表.
    Refs { show: bool, no_ask: bool },
    /// The open document could not be written before it was to be replaced
    /// (another document, 新建, 退出): 重试 / 放弃修改, then carry on.
    SaveFailed,
}

/// What to do once the open document is safely on disk (or its changes given up).
pub type AfterSave = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// Inline 重命名 of a sidebar row.
pub struct Rename {
    pub id: String,
    pub field: Entity<TextField>,
    _subs: Vec<Subscription>,
}

/// The top bar's title while it is being edited: a field over the open note's
/// first heading.
pub struct TitleEdit {
    pub field: Entity<TextField>,
    _subs: Vec<Subscription>,
}

pub struct Workspace {
    pub quick_note: bool,
    pub shortcut_recording: Option<String>,
    pub shortcut_strokes: Vec<String>,
    /// The failed change: (action, message); an empty action is page-wide.
    pub shortcut_error: Option<(String, String)>,
    pub settings_page: crate::settings::Page,
    /// 外观's theme dropdown while open: the theme its preview shows.
    pub theme_menu: Option<String>,
    /// Where the theme dropdown's button was drawn last frame.
    pub theme_anchor: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// 快捷键's filter field.
    pub settings_filter: Entity<TextField>,
    native_sidebar: crate::native_header::NativeSidebar,
    pub store: SharedStore,
    pub entries: Vec<Entry>,
    pub current: Option<String>,
    pub editor: Option<Entity<Editor>>,
    pub search: Entity<TextField>,
    /// Keyboard highlight inside search results.
    pub search_sel: Option<usize>,
    pub sidebar_open: bool,
    /// Dragged width (300 by default, 268 to min(800, window − 540)).
    pub sidebar_width: f32,
    /// The resize handle is being dragged.
    pub sidebar_drag: bool,
    /// The search dialog (command-center style) is open.
    pub search_open: bool,
    pub status: SaveStatus,
    pub row_menu: Option<RowMenu>,
    pub rename: Option<Rename>,
    pub title_edit: Option<TitleEdit>,
    /// A folder being renamed: its path and the field holding the new name.
    pub folder_rename: Option<(String, Entity<TextField>, Vec<Subscription>)>,
    /// Last-frame bounds of each row's "…" button, to hang its menu from.
    pub more_bounds: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    /// Header "…" menu and its open submenu.
    pub doc_menu: bool,
    pub doc_sub: Option<DocSub>,
    /// Last-frame bounds of the open submenu (clicks there keep the menu open).
    pub doc_sub_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub doc_info: Option<DocInfo>,
    pub list_scroll: ScrollHandle,
    pub search_scroll: ScrollHandle,
    /// Last-frame bounds of the row menu's 下载 submenu.
    pub row_sub_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// 批量操作 mode: the checked docs of 我的文档, in check order.
    pub batch: Option<Vec<String>>,
    /// The open confirmation dialog.
    pub confirm: Option<Confirm>,
    pub view: View,
    pub home: crate::home::Home,
    pub settings_scroll: ScrollHandle,
    /// The toast and when it appeared.
    pub toast: Option<(ToastKind, SharedString, Instant)>,
    toast_task: Option<Task<()>>,
    /// Folded sidebar sections: 置顶, 我的文档.
    pub folded: [bool; 2],
    /// Expanded pages of the 我的文档 tree.
    pub expanded: HashSet<String>,
    /// The vault's folders (vault-relative paths), refreshed with the notes.
    pub folders: Vec<crate::vault::Folder>,
    pub move_dlg: Option<crate::tree::MoveDialog>,
    /// 导出 Markdown 设置.
    pub export_md: Option<crate::transfer::ExportMd>,
    /// 编辑 / 阅读 dropdown, and the mode button's last-frame bounds.
    pub mode_menu: bool,
    pub mode_btn_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The header "…" button, which the doc menu hangs from.
    pub more_btn_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The 快捷键 drawer, and the help button's menu (open while hovered).
    pub shortcuts: bool,
    pub(crate) focus: FocusHandle,
    save_task: Option<Task<()>>,
    /// The open document has changes not yet on disk (cleared only by a
    /// successful save or by 放弃修改).
    unsaved: bool,
    /// Held back by a failed save: runs after 重试 succeeds or 放弃修改.
    pub(crate) after_save: Option<AfterSave>,
    editor_sub: Option<Subscription>,
    _subs: Vec<Subscription>,
}


/// A macOS notification (only logged when scripted).
fn notify_system(title: &str, body: &str) {
    if std::env::var_os("FDL_SCRIPT").is_some() {
        return eprintln!("notification: {title} / {body}");
    }
    let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"{}\"", q(body), q(title));
    std::thread::spawn(move || std::process::Command::new("osascript").arg("-e").arg(script).status());
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_window(false, window, cx)
    }

    pub fn new_window(quick: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build_window(quick, None, window, cx)
    }

    pub fn new_document_window(document: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build_window(true, document, window, cx)
    }

    fn build_window(quick: bool, document: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::settings::load();
        if !cx.has_global::<SharedStore>() { cx.set_global(SharedStore::open().expect("open document library")); }
        let mut store = cx.global::<SharedStore>().clone();
        let entries = store.list();
        let folders = store.folders();
        let search = cx.new(|cx| {
            let mut f = TextField::new("搜索", cx);
            f.placeholder_color = Theme::text_disabled();
            f
        });
        let settings_filter = cx.new(|cx| {
            let mut f = TextField::new(crate::settings::tr("搜索快捷键", "Search shortcuts"), cx);
            f.placeholder_color = Theme::text_disabled();
            f
        });
        let subs = vec![
            cx.subscribe(&settings_filter, |_, _, ev: &TextFieldEvent, cx| if matches!(ev, TextFieldEvent::Changed) { cx.notify(); }),
            cx.observe_window_bounds(window, |this, window, cx| crate::quick_note::remember(window, this.quick_note, cx)),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    if let Some(id) = &this.current { crate::quick_note::used(id, cx); }
                    this.entries = this.store.list();
                    this.folders = this.store.folders();
                    this.sort();
                    // The files are the source of truth: if another program
                    // changed the open note, show what is on disk now.
                    this.reload_if_changed_on_disk(window, cx);
                    cx.notify();
                }
                else { this.cancel_shortcut_recording(cx); this.flush(cx); }
            }),
            cx.subscribe_in(&search, window, |this, _, ev: &TextFieldEvent, window, cx| {
                match ev {
                    TextFieldEvent::Changed => {
                        this.search_sel = this.default_search_sel(cx);
                        this.search_scroll.set_offset(point(px(0.), px(0.)));
                    }
                    TextFieldEvent::Up => this.move_search_sel(-1, cx),
                    TextFieldEvent::Down => this.move_search_sel(1, cx),
                    TextFieldEvent::Submit => {
                        if let Some(ix) = this.search_sel {
                            this.pick_search(ix, window, cx)
                        }
                    }
                    TextFieldEvent::Cancel => this.close_search(window, cx),
                    _ => {}
                }
                cx.notify();
            }),
            // Last resort (the system logging out): ⌘Q and the close button
            // go through `save_before` and ask when the save fails.
            cx.on_app_quit(|this, cx| {
                crate::quick_note::persist(cx);
                this.flush(cx);
                async {}
            }),
        ];
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            crate::quick_note::persist(cx);
            this.update(cx, |w, cx| { w.cancel_shortcut_recording(cx); w.save_before(cx, |_, window, _| window.remove_window()) }).unwrap_or(true)
        });
        if !quick { crate::settings::watch_themes(cx); }
        // 日期提醒: post what fell due since the last look (not what was
        // missed while the app was closed).
        cx.spawn(async |this, cx| {
            let mut last = crate::time::now_ms() as i64;
            loop {
                cx.background_executor().timer(Duration::from_secs(15)).await;
                let now = crate::time::now_ms() as i64;
                if this.update(cx, |w, cx| w.check_reminders(last, now, cx)).is_err() {
                    break;
                }
                last = now;
            }
        })
        .detach();
        let sidebar_width = store.settings().sidebar_width.unwrap_or(theme::SIDEBAR_WIDTH);
        let mut ws = Workspace {
            quick_note: quick,
            shortcut_recording: None,
            shortcut_strokes: vec![],
            shortcut_error: None,
            settings_page: Default::default(),
            theme_menu: None,
            theme_anchor: Default::default(),
            settings_filter,
            store,
            entries,
            folders,
            current: None,
            editor: None,
            search,
            search_sel: None,
            sidebar_open: !quick,
            sidebar_width,
            sidebar_drag: false,
            search_open: false,
            status: SaveStatus::Idle,
            row_menu: None,
            rename: None,
            title_edit: None,
            folder_rename: None,
            more_bounds: Default::default(),
            doc_menu: false,
            doc_sub: None,
            doc_sub_bounds: Rc::new(Cell::new(None)),
            doc_info: None,
            list_scroll: ScrollHandle::new(),
            search_scroll: ScrollHandle::new(),
            row_sub_bounds: Rc::new(Cell::new(None)),
            batch: None,
            confirm: None,
            view: View::Doc,
            home: Default::default(),
            settings_scroll: ScrollHandle::new(),
            toast: None,
            toast_task: None,
            mode_menu: false,
            mode_btn_bounds: Rc::new(Cell::new(None)),
            more_btn_bounds: Rc::new(Cell::new(None)),
            native_sidebar: Default::default(),
            shortcuts: false,
            folded: [false; 2],
            expanded: HashSet::new(),
            move_dlg: None,
            export_md: None,
            focus: cx.focus_handle(),
            save_task: None,
            unsaved: false,
            after_save: None,
            editor_sub: None,
            _subs: subs,
        };
        ws.sort();
        ws.load_home();
        match document.or_else(|| (!quick).then(|| ws.entries.first().map(|e| e.id.clone())).flatten()) {
            Some(id) => ws.open(&id, window, cx),
            None => ws.new_doc(window, cx),
        }
        // Kept in the top bar (not just a toast): the user should notice.
        let warnings = ws.store.take_warnings();
        if !warnings.is_empty() {
            let msg = warnings.iter().map(|s| crate::settings::diagnostic(s)).collect::<Vec<_>>().join(crate::settings::tr("；", "; "));
            ws.show_toast_kind(ToastKind::Error, &msg, cx);
            ws.status = SaveStatus::Failed(msg);
        }
        ws
    }

    /// Most recently modified first.
    pub(crate) fn sort(&mut self) {
        self.entries.sort_by_key(|e| std::cmp::Reverse(e.meta.modified));
    }

    pub fn query(&self, cx: &App) -> String {
        self.search.read(cx).text.trim().to_lowercase()
    }

    /// Reload the open note when its file changed underneath us (an external
    /// editor, a sync client, `git checkout`). Unsaved edits stay ahead: they
    /// are written back on the next save, so the disk never wins over them.
    pub fn reload_if_changed_on_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.unsaved || self.editor.is_none() {
            return;
        }
        let Some(id) = self.current.clone() else { return };
        let Some(ed) = self.editor.clone() else { return };
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else { return };
        let disk = self.store.modified(&id);
        let shown = ed.read(cx).modified;
        if disk == 0 || disk <= shown {
            return;
        }
        let Ok(doc) = self.store.load(&id) else { return };
        let _ = entry;
        ed.update(cx, |e, cx| {
            e.receive_document(&hush_core::editor::core::Core::new(doc), cx);
            e.modified = disk;
            cx.notify();
        });
        self.show_toast_kind(ToastKind::Info, crate::settings::text("文件已在磁盘上更新，已重新载入"), cx);
        let _ = window;
        cx.notify();
    }

    pub fn open(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.try_open(id, window, cx);
    }

    /// Show `id`; false when it could not be opened (or the open document
    /// could not be saved first, see [`Self::save_before`]).
    pub fn try_open(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.view != View::Doc {
            self.view = View::Doc;
            cx.notify();
        }
        if self.current.as_deref() == Some(id) {
            return true;
        }
        let target = id.to_string();
        if !self.save_before(cx, move |w, window, cx| {
            w.open(&target, window, cx);
        }) {
            return false;
        }
        let shared = !self.document_peers(id, window, cx).is_empty();
        let doc = match self.store.load_for_edit(id, shared) {
            Ok(d) => d,
            Err(e) => {
                self.show_toast_kind(ToastKind::Error, &crate::localized_format!("无法打开文档：{}", "Could not open document: {}", crate::settings::diagnostic(&e.to_string())), cx);
                cx.notify();
                return false;
            }
        };
        let meta = self.entries.iter().find(|e| e.id == id).map(|e| e.meta.clone());
        let modified = meta.as_ref().map_or_else(crate::time::now, |m| m.modified);
        let width = meta.as_ref().map(|m| m.width).unwrap_or_default();
        let pinned = meta.as_ref().is_some_and(|m| !m.catalogue_closed);
        let note_path = id.to_string();
        let editor = cx.new(|cx| {
            let mut e = Editor::new(doc, modified, width, window, cx);
            e.set_note_path(&note_path, cx);
            e.cat_pinned = pinned;
            if let Some(m) = &meta {
                e.created = m.created;
                e.info = m.info;
                e.set_read_only(m.reading, cx);
            }
            e
        });
        // Reuse the live state, including unsaved edits and undo history.
        if let Some(peer) = self.document_peers(id, window, cx).first() {
            let core = peer.read(cx).core.clone();
            editor.update(cx, |e, cx| e.receive_document(&core, cx));
        }
        self.editor_sub = Some(cx.subscribe_in(&editor, window, |this, _, ev: &EditorEvent, window, cx| match ev {
            EditorEvent::Changed => {
                // 阅读模式 put the document back: nothing to save.
                let rejected = this.editor.clone().is_some_and(|ed| ed.update(cx, |e, cx| e.reject_read_only_edit(cx)));
                if !rejected {
                    this.sync_document(window, cx);
                    this.schedule_save(window, cx);
                }
            }
            EditorEvent::Mode(reading) => this.set_mode(*reading, window, cx),
            EditorEvent::Info(info) => this.set_info_row(*info),
            EditorEvent::Toast => {
                this.toast = None;
                cx.notify();
            }
            EditorEvent::Save => {
                this.flush(cx);
            }
            EditorEvent::Search(q) => {
                this.open_search(window, cx);
                this.search.update(cx, |s, cx| s.set_text(q, cx));
                this.search_sel = this.default_search_sel(cx);
            }
            EditorEvent::OpenDoc(id) => this.open(id, window, cx),
            EditorEvent::OpenDocAt(id, ordinal) => this.open_at(id, *ordinal, window, cx),
            EditorEvent::HideRefs => this.ask_refs(false, cx),
            EditorEvent::Duplicate => {
                if let Some(id) = this.current.clone() {
                    this.duplicate_doc(&id, window, cx);
                }
            }
            EditorEvent::Catalogue(pinned) => {
                if let Some(id) = this.current.clone() {
                    if let Some(e) = this.entries.iter_mut().find(|e| e.id == id) {
                        e.meta.catalogue_closed = !pinned;
                    }
                    let _ = this.store.set_catalogue(&id, *pinned);
                }
            }
        }));
        // Opening a document puts the caret in it, unless the user is typing a search.
        if !self.search_open {
            editor.read(cx).focus(window);
        }
        if let Some(m) = self.store.touch(id)
            && let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
                e.meta.opened = m.opened;
            }
        self.editor = Some(editor);
        self.current = Some(id.to_string());
        crate::quick_note::used(id, cx);
        self.reveal_in_tree(id);
        self.sync_sub_pages(cx);
        self.status = SaveStatus::Idle;
        self.row_menu = None;
        self.doc_menu = false;
        self.doc_sub = None;
        self.mode_menu = false;
        cx.notify();
        true
    }

    pub fn new_doc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_doc_in(None, window, cx);
    }

    /// 新建笔记 in the folder the open note lives in, or at the root.
    pub fn new_doc_in(&mut self, folder: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.save_before(cx, |w, window, cx| w.new_doc(window, cx)) {
            return;
        }
        let folder = folder.or_else(|| self.current.as_deref().and_then(crate::tree::folder_of)).unwrap_or_default();
        match self.store.create_in(&folder) {
            Ok((id, _)) => {
                if !folder.is_empty() {
                    self.expanded.insert(folder.clone());
                }
                self.search_open = false;
                self.doc_info = None;
                self.home.sort_menu = false;
                self.entries = self.store.list();
                self.folders = self.store.folders();
                self.sort();
                self.search.update(cx, |s, cx| s.set_text("", cx));
                self.open(&id, window, cx);
                if let Some(ed) = &self.editor {
                    ed.read(cx).focus(window);
                }
                // A new note has no name yet: put the caret in its row so the
                // name can be typed straight away, as in a file manager.
                self.begin_title_edit(window, cx);
                cx.notify();
                return;
            }
            Err(e) => self.status = SaveStatus::Failed(crate::localized_format!("新建失败：{}", "Could not create document: {}", crate::settings::diagnostic(&e.to_string()))),
        }
        cx.notify();
    }

    /// 删除 a single page without asking: it goes straight to the system
    /// trash, and can be restored from Finder / Explorer.
    pub fn ask_delete(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.row_menu = None;
        self.doc_menu = false;
        self.doc_sub = None;
        self.delete_docs(&[id.to_string()], window, cx);
        cx.notify();
    }

    /// Move documents to the system trash; if the open one goes, open the next.
    pub fn delete_docs(&mut self, ids: &[String], window: &mut Window, cx: &mut Context<Self>) {
        // A page goes with its child pages: the subtree goes as one batch.
        let tree = crate::tree::Tree::new(&self.entries);
        let mut all: Vec<(String, Option<String>)> = vec![];
        for id in ids {
            let covered = self.ancestors(id).iter().any(|a| ids.contains(&a.id));
            if covered {
                continue;
            }
            for d in tree.subtree(id) {
                if !all.iter().any(|(x, _)| *x == d) {
                    let root = (d != *id).then(|| id.clone());
                    all.push((d, root));
                }
            }
        }
        let groups = all;
        let all: Vec<String> = groups.iter().map(|(d, _)| d.clone()).collect();
        for handle in cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id()) {
            if let Some(handle) = handle.downcast::<Workspace>()
                && handle.update(cx, |ws, _, _| ws.current.as_ref().is_some_and(|id| all.contains(id))).unwrap_or(false)
            {
                self.show_toast_kind(ToastKind::Error, crate::settings::text("请先关闭正在编辑这些文档的其他窗口，再删除"), cx);
                return;
            }
        }
        if self.current.as_ref().is_some_and(|c| all.contains(c)) {
            let retry = ids.to_vec();
            if !self.save_before(cx, move |w, window, cx| w.delete_docs(&retry, window, cx)) {
                return;
            }
        }
        let mut deleted = vec![];
        let mut errors = vec![];
        // A volume without a trash: ask once more, then delete for good.
        let mut no_trash = vec![];
        for (id, root) in &groups {
            // Do not split a subtree when its root could not be deleted.
            if root.as_ref().is_some_and(|r| !deleted.contains(r)) { continue; }
            match self.store.delete(id) {
                Ok(()) => deleted.push(id.clone()),
                Err(e) if e.downcast_ref::<crate::vault::NoTrash>().is_some() => no_trash.push(id.clone()),
                Err(e) => errors.push(crate::localized_format!("删除失败：{}", "Could not delete document: {}", crate::settings::diagnostic(&e.to_string()))),
            }
        }
        if self.current.as_ref().is_some_and(|c| deleted.contains(c)) {
            self.save_task = None;
            self.unsaved = false;
            self.current = None;
            self.editor = None;
            self.editor_sub = None;
        }
        self.entries.retain(|e| !deleted.contains(&e.id));
        if let Some(b) = self.batch.as_mut() {
            b.retain(|id| !deleted.contains(id));
        }
        self.row_menu = None;
        if self.current.is_none() {
            // Stay on 主页 if that's where the delete came from.
            let view = self.view;
            match self.entries.first().map(|e| e.id.clone()) {
                Some(next) => self.open(&next, window, cx),
                None => self.new_doc(window, cx),
            }
            self.view = view;
        }
        self.sync_sub_pages(cx);
        errors.extend(self.store.take_warnings().iter().map(|s| crate::settings::diagnostic(s)));
        if errors.is_empty() && no_trash.is_empty() {
            self.show_toast(crate::settings::text("已移到回收站"), cx);
        } else if !errors.is_empty() {
            let msg = errors.join("；");
            self.show_toast_kind(ToastKind::Error, &msg, cx);
            self.status = SaveStatus::Failed(msg);
        }
        if !no_trash.is_empty() {
            // Sub-pages of a note that stayed behind would become top-level
            // notes, so they go with it in the permanent delete too.
            let carried: Vec<String> = groups.iter().filter(|(_, root)| root.as_ref().is_some_and(|r| no_trash.contains(r))).map(|(d, _)| d.clone()).collect();
            for id in carried {
                if !no_trash.contains(&id) {
                    no_trash.push(id);
                }
            }
            self.confirm = Some(Confirm::DeletePermanent { ids: no_trash, folder: None });
        }
        cx.notify();
    }

    /// Delete documents for good, after the 无回收站 confirmation.
    pub fn delete_permanently(&mut self, ids: &[String], folder: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let mut errors = vec![];
        if let Some(rel) = folder {
            match self.store.delete_folder_permanently(rel) {
                Ok(()) => {
                    self.expanded.retain(|p| p != rel && !p.starts_with(&format!("{rel}/")));
                }
                Err(e) => errors.push(crate::localized_format!("删除失败：{}", "Could not delete folder: {}", crate::settings::diagnostic(&e.to_string()))),
            }
        }
        let mut deleted = vec![];
        for id in ids {
            match self.store.delete_permanently(id) {
                Ok(()) => deleted.push(id.clone()),
                Err(e) => errors.push(crate::localized_format!("删除失败：{}", "Could not delete document: {}", crate::settings::diagnostic(&e.to_string()))),
            }
        }
        let gone = |id: &str| folder.is_some_and(|rel| id.starts_with(&format!("{rel}/"))) || deleted.iter().any(|x| x == id);
        if self.current.as_deref().is_some_and(gone) {
            self.save_task = None;
            self.unsaved = false;
            self.current = None;
            self.editor = None;
            self.editor_sub = None;
        }
        self.entries = self.store.list();
        self.folders = self.store.folders();
        self.sort();
        if self.current.is_none() {
            let view = self.view;
            match self.entries.first().map(|e| e.id.clone()) {
                Some(next) => self.open(&next, window, cx),
                None => self.new_doc(window, cx),
            }
            self.view = view;
        }
        self.sync_sub_pages(cx);
        if errors.is_empty() {
            self.show_toast(crate::settings::text("已彻底删除"), cx);
        } else {
            let msg = errors.join("；");
            self.show_toast_kind(ToastKind::Error, &msg, cx);
            self.status = SaveStatus::Failed(msg);
        }
        cx.notify();
    }

    pub fn toggle_pin(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut on = false;
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.meta.pinned = !e.meta.pinned;
            on = e.meta.pinned;
            let _ = self.store.set_pinned(id, e.meta.pinned);
        }
        self.sort();
        self.row_menu = None;
        self.show_toast(if on { crate::settings::text("已添加到“置顶”") } else { crate::settings::text("已从“置顶”移除") }, cx);
        cx.notify();
    }

    /// 收藏 / 取消收藏 (listed under 主页 → 收藏).
    pub fn toggle_star(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(e) = self.entries.iter_mut().find(|e| e.id == id) else { return };
        let on = e.meta.starred == 0;
        if let Ok(t) = self.store.set_starred(id, on) {
            e.meta.starred = t;
        }
        self.row_menu = None;
        self.doc_menu = false;
        self.doc_sub = None;
        self.show_toast(if on { crate::settings::text("已添加到“收藏”") } else { crate::settings::text("已从“收藏”中移除") }, cx);
        cx.notify();
    }

    /// Switch the main pane to 主页 or back to the document.
    pub fn show_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.cancel_shortcut_recording(cx);
        self.view = view;
        self.row_menu = None;
        self.theme_menu = None;
        self.home.sort_menu = false;
        cx.notify();
    }

    /// A success toast (删除成功, 移动成功 …).
    pub fn show_toast(&mut self, msg: &str, cx: &mut Context<Self>) {
        self.show_toast_kind(ToastKind::Success, msg, cx);
    }

    pub fn show_toast_kind(&mut self, kind: ToastKind, msg: &str, cx: &mut Context<Self>) {
        self.toast = Some((kind, SharedString::from(msg.to_string()), Instant::now()));
        if let Some(ed) = &self.editor {
            ed.update(cx, |e, cx| e.clear_toast(cx));
        }
        self.toast_task = Some(cx.spawn(async |this, cx| {
            cx.background_executor().timer(Duration::from_millis(2700)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
        cx.notify();
    }

    fn render_toast(&self, window: &mut Window) -> Option<AnyElement> {
        let (kind, msg, t0) = self.toast.as_ref()?;
        crate::ui::toast_el(*kind, msg, *t0, window)
    }

    /// 重命名: the row's title turns into an input; Enter or clicking away saves, Esc cancels.
    pub fn start_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.entries.iter().find(|e| e.id == id).map(|e| e.title.trim().to_string()) else { return };
        let field = cx.new(|cx| {
            let mut f = TextField::new(crate::model::UNTITLED, cx);
            f.set_text(&title, cx);
            f.select_all_text(cx);
            f
        });
        let focus = field.focus_handle(cx);
        window.focus(&focus);
        let subs = vec![
            cx.subscribe_in(&field, window, |w, _, ev: &TextFieldEvent, window, cx| match ev {
                TextFieldEvent::Submit => w.finish_rename(true, window, cx),
                TextFieldEvent::Cancel => w.finish_rename(false, window, cx),
                _ => {}
            }),
            cx.on_blur(&focus, window, |w, window, cx| w.finish_rename(true, window, cx)),
        ];
        self.rename = Some(Rename { id: id.to_string(), field, _subs: subs });
        // From 主页 the row may be hidden: show it in the sidebar.
        self.sidebar_open = true;
        self.reveal_in_tree(id);
        self.row_menu = None;
        cx.notify();
    }

    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.rename.take() else { return };
        let refocus = r.field.read(cx).is_focused(window);
        let title = r.field.read(cx).text.trim().to_string();
        let old = self.entries.iter().find(|e| e.id == r.id).map(|e| e.title.trim().to_string());
        if commit && old.is_some_and(|o| o != title) {
            if self.current.as_deref() == Some(r.id.as_str()) {
                // The open editor owns the title; its Changed event saves it.
                if let Some(ed) = &self.editor {
                    ed.update(cx, |e, cx| e.set_title(&title, cx));
                }
            } else if let Some(peer) = self.document_peers(&r.id, window, cx).first() {
                peer.update(cx, |e, cx| e.set_title(&title, cx));
            } else {
                match self.store.rename(&r.id, &title) {
                    Ok((new_id, meta)) => {
                        // The note's file carries its title, so the id moves
                        // with it; anything pointing at the old id follows.
                        if new_id != r.id {
                            self.retarget(&r.id, &new_id);
                        }
                        if let Some(e) = self.entries.iter_mut().find(|e| e.id == new_id) {
                            e.title = title;
                            e.meta = meta;
                        }
                        self.sort();
                    }
                    Err(e) => self.status = SaveStatus::Failed(crate::localized_format!("重命名失败：{}", "Could not rename document: {}", crate::settings::diagnostic(&e.to_string()))),
                }
            }
        }
        if refocus
            && let Some(ed) = &self.editor {
                ed.read(cx).focus(window);
            }
        self.sync_sub_pages(cx);
        cx.notify();
    }

    /// A note's file was renamed: keep the open document, its history and the
    /// selection pointing at the same note under its new path.
    pub(crate) fn retarget(&mut self, old: &str, new: &str) {
        if self.current.as_deref() == Some(old) {
            self.current = Some(new.to_string());
        }
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == old) {
            e.id = new.to_string();
        }
    }

    fn document_peers(&self, id: &str, window: &Window, cx: &mut Context<Self>) -> Vec<Entity<Editor>> {
        cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id())
            .filter_map(|w| w.downcast::<Workspace>())
            .filter_map(|w| w.update(cx, |ws, _, _| {
                (ws.current.as_deref() == Some(id)).then(|| ws.editor.clone()).flatten()
            }).ok().flatten()).collect()
    }

    fn sync_document(&mut self, window: &Window, cx: &mut Context<Self>) {
        let (Some(id), Some(editor)) = (self.current.clone(), self.editor.clone()) else { return };
        let peers = self.document_peers(&id, window, cx);
        if peers.is_empty() { return; }
        let core = editor.read(cx).core.clone();
        for peer in peers { peer.update(cx, |e, cx| e.receive_document(&core, cx)); }
        for handle in cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id()).filter_map(|w| w.downcast::<Workspace>()) {
            let _ = handle.update(cx, |ws, _, cx| {
                if let Some(entry) = ws.entries.iter_mut().find(|e| e.id == id) {
                    entry.title = core.doc.title.text.clone();
                    entry.icon = core.doc.icon.clone();
                }
                // Every remaining view can save the latest state if its sibling closes.
                if ws.current.as_deref() == Some(&id) { ws.unsaved = true; }
                cx.notify();
            });
        }
    }

    fn schedule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.status = SaveStatus::Saving;
        self.unsaved = true;
        // The note's title is its first heading, which is part of the text
        // being edited: take it from the document so the sidebar follows.
        if let (Some(id), Some(ed)) = (&self.current, &self.editor) {
            let doc = &ed.read(cx).core.doc;
            let title = display_title(&crate::md::title_of(doc));
            if let Some(e) = self.entries.iter_mut().find(|e| &e.id == id) {
                e.title = title;
            }
        }
        self.save_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let _ = this.update(cx, |w, cx| w.flush(cx));
        }));
        cx.notify();
    }

    fn check_reminders(&mut self, from: i64, to: i64, cx: &mut Context<Self>) {
        // The open document's reminders are read live (unsaved edits too).
        let open = self.editor.as_ref().map(|e| e.read(cx).core.doc.reminders());
        for e in &self.entries {
            let rs = match &open {
                Some(rs) if self.current.as_deref() == Some(e.id.as_str()) => rs,
                _ => &e.reminders,
            };
            for r in rs.iter().filter(|r| r.should_notify && from < r.notify && r.notify <= to) {
                notify_system(&display_title(&e.title), &crate::editor::reminder::notification(r));
            }
        }
    }

    /// Write the open document to disk if it has unsaved changes. False when
    /// that failed: the changes stay pending (a later save retries them) and
    /// the top bar shows the error.
    pub fn flush(&mut self, cx: &mut Context<Self>) -> bool {
        self.save_task = None;
        if !self.unsaved {
            return true;
        }
        let (Some(id), Some(ed)) = (self.current.clone(), self.editor.clone()) else {
            self.unsaved = false;
            return true;
        };
        let doc = &ed.read(cx).core.doc;
        let (title, icon, body, reminders, links) =
            (display_title(&crate::md::title_of(doc)), doc.icon.clone(), doc.plain_text(), doc.reminders(), doc.doc_links());
        // An edit in the source view knows the file's exact text; writing it
        // directly keeps every byte the edit did not touch.
        let saved = match ed.read(cx).docs.clone() {
            Some(text) => self.store.write_text(&id, &text).and_then(|()| self.store.note_meta(&id)),
            None => self.store.save(&id, doc),
        };
        match saved {
            Ok(meta) => {
                if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
                    e.title = title;
                    e.icon = icon;
                    e.body = body;
                    e.reminders = reminders;
                    e.links = links;
                    e.meta = meta.clone();
                }
                self.sync_links(cx);
                ed.update(cx, |e, cx| {
                    e.modified = meta.modified;
                    cx.notify();
                });
                self.status = SaveStatus::Saved;
                self.unsaved = false;
                self.sort();
            }
            Err(e) => self.status = SaveStatus::Failed(crate::localized_format!("保存失败：{}", "Could not save: {}", crate::settings::diagnostic(&e.to_string()))),
        }
        cx.notify();
        !self.unsaved
    }

    /// Save the open document before something replaces it. When the save
    /// fails nothing is replaced: the 保存失败 dialog offers 重试 / 放弃修改
    /// and runs `then` after either.
    pub fn save_before(&mut self, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> bool {
        if self.flush(cx) {
            return true;
        }
        self.after_save = Some(Box::new(then));
        self.confirm = Some(Confirm::SaveFailed);
        cx.notify();
        false
    }

    /// The 保存失败 dialog's choices: `retry` saves again, otherwise the
    /// changes are given up. Either way the held-back action runs once the
    /// document no longer needs saving.
    pub(crate) fn resolve_save_failed(&mut self, retry: bool, window: &mut Window, cx: &mut Context<Self>) {
        if retry {
            if !self.flush(cx) {
                self.confirm = Some(Confirm::SaveFailed);
                return;
            }
        } else {
            self.unsaved = false;
            self.status = SaveStatus::Idle;
        }
        if let Some(then) = self.after_save.take() {
            then(self, window, cx);
        }
        cx.notify();
    }

    /// The error of the last failed save, for the 保存失败 dialog.
    pub(crate) fn save_error(&self) -> String {
        match &self.status {
            SaveStatus::Failed(e) => e.clone(),
            _ => crate::settings::text("保存失败").to_string(),
        }
    }

    /// Hand the open document the library's titles and the documents
    /// linking to it (双向链接).
    pub(crate) fn sync_links(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(ed)) = (self.current.clone(), self.editor.clone()) else { return };
        let docs = self
            .entries
            .iter()
            .map(|e| crate::editor::links::DocRef { id: e.id.clone(), title: e.title.clone(), icon: e.icon.clone(), modified: e.meta.modified, opened: e.meta.opened })
            .collect();
        let backlinks = self
            .entries
            .iter()
            .filter(|e| e.id != id)
            .filter_map(|e| {
                let lines: Vec<_> = e.links.iter().filter(|l| l.target == id).cloned().collect();
                (!lines.is_empty()).then(|| crate::editor::links::Backlink { id: e.id.clone(), lines })
            })
            .collect();
        let hidden = self.entries.iter().find(|e| e.id == id).is_some_and(|e| e.meta.refs_hidden);
        ed.update(cx, |e, cx| e.set_links(id, docs, backlinks, hidden, cx));
    }

    /// A 反向引用 line: open that document at its `ordinal`th block.
    fn open_at(&mut self, id: &str, ordinal: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.try_open(id, window, cx) {
            return;
        }
        let Some(ed) = self.editor.clone() else { return };
        ed.update(cx, |e, cx| {
            let mut n = 0;
            let mut target = None;
            e.core.doc.for_each_block(|b| {
                if n == ordinal {
                    target = Some(b.id);
                }
                n += 1;
            });
            if let Some(b) = target {
                e.scroll_to_block(b, cx);
            }
        });
    }

    /// 隐藏引用列表 / 显示引用列表: confirm first (hiding can skip it with 不再提示).
    pub(crate) fn ask_refs(&mut self, show: bool, cx: &mut Context<Self>) {
        if !show && self.store.settings().refs_hide_no_ask {
            return self.set_refs_hidden(true, cx);
        }
        self.confirm = Some(Confirm::Refs { show, no_ask: false });
        cx.notify();
    }

    pub(crate) fn set_refs_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else { return };
        if self.store.update_meta(&id, |m| m.refs_hidden = hidden).is_ok()
            && let Some(e) = self.entries.iter_mut().find(|e| e.id == id)
        {
            e.meta.refs_hidden = hidden;
        }
        self.sync_links(cx);
        cx.notify();
    }

    /// 主页's search box, centred on the pane but never over what sits on
    /// either side of it: the title (and, with the sidebar hidden, the
    /// traffic lights and 展开侧栏) on the left, + on the right. Below a
    /// usable width it is None and the caller shows a search icon instead.
    pub(crate) fn header_search(&self, window: &Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let main = f32::from(window.viewport_size().width) - if self.sidebar_open { self.sidebar_w(window) } else { 0. };
        let title = crate::settings::label(crate::settings::text("主页"));
        let title_w = f32::from(window.text_system().shape_line(title.clone().into(), px(17.), &[window.text_style().highlight(HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() }).to_run(title.len())], None).width);
        // page_header's left inset (26 with the sidebar, else the traffic
        // lights plus the 4 + 28 + 12 of 展开侧栏), then 16 clear of the title.
        let left = if self.sidebar_open { 26. } else { theme::TRAFFIC_LIGHTS_W + 44. + if cfg!(target_os = "windows") { 32. } else { 0. } } + title_w + 16.;
        // + is 32 wide with an 8pt inset; 16 clear of it.
        let right = HEADER_ACTIONS_H + crate::ui::header_right_inset() + 16.;
        let half = (main / 2. - left).min(main / 2. - right);
        let width = (main - 320.).min(2. * half).min(360.);
        if width < 120. { return None }
        Some(div().id("header-search").absolute().left(px((main - width) / 2.)).top(px(8.))
            .w(px(width)).h(px(32.)).no_drag().flex().items_center().gap(px(8.)).px(px(10.))
            .rounded(px(9.)).bg(Theme::fill_hover())
            .cursor_pointer().hover(|s| s.bg(Theme::fill_hover_strong()))
            .child(icon("search", 15., Theme::text_caption()))
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(Theme::text_caption()).child(crate::settings::tr("搜索文档…", "Search documents…")))
            .when(width >= 200., |d| d.child(div().flex_none().text_size(px(11.)).text_color(Theme::text_placeholder()).child(crate::shortcuts::hint("workspace::FocusSearch", "⌘ + J").replace(" + ", " "))))
            .tip(crate::settings::tr("搜索文档", "Search documents"), Some(crate::shortcuts::hint("workspace::FocusSearch", "⌘ + J")))
            .on_click(cx.listener(|w, _, window, cx| w.open_search(window, cx))))
    }

    /// The note's title in the top bar: text that becomes an editable field
    /// with one click. In Markdown the title *is* the first heading, so
    /// editing here edits that line.
    fn title_el(&self, title: String, cx: &mut Context<Self>) -> AnyElement {
        if let Some(t) = &self.title_edit {
            return div()
                .min_w(px(120.))
                .max_w(px(420.))
                .h(px(24.))
                .px(px(6.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .bg(Theme::bg_body())
                .border_1()
                .border_color(Theme::primary_500())
                .text_size(px(14.))
                .child(t.field.clone())
                .into_any_element();
        }
        div()
            .id("doc-title")
            .no_drag()
            .min_w(px(28.))
            .max_w(px(420.))
            .px(px(4.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .cursor_pointer()
            .text_size(px(14.))
            .line_height(px(20.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(Theme::text())
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .hover(|s| s.bg(Theme::fill_hover()))
            .child(ui_text(title))
            .on_click(cx.listener(|w, _, window, cx| w.begin_title_edit(window, cx)))
            .into_any_element()
    }

    /// Rename a folder from the sidebar. The notes inside keep their place.
    pub fn rename_folder(&mut self, rel: &str, name: &str, cx: &mut Context<Self>) {
        match self.store.rename_folder(rel, name) {
            Ok(new_rel) => {
                // Anything the UI holds by path follows the folder.
                self.entries = self.store.list();
                self.folders = self.store.folders();
                if let Some(cur) = self.current.clone()
                    && cur.starts_with(&format!("{rel}/"))
                {
                    self.current = Some(format!("{new_rel}/{}", &cur[rel.len() + 1..]));
                }
                self.rebuild_expanded(rel, &new_rel);
                self.sort();
            }
            Err(e) => self.status = SaveStatus::Failed(crate::localized_format!("重命名失败：{}", "Could not rename folder: {}", crate::settings::diagnostic(&e.to_string()))),
        }
        cx.notify();
    }

    /// Keep the sidebar's open folders open across a folder rename.
    fn rebuild_expanded(&mut self, old: &str, new: &str) {
        let moved: Vec<String> = self.expanded.iter().filter(|p| p.starts_with(&format!("{old}/"))).cloned().collect();
        for p in moved {
            self.expanded.remove(&p);
            self.expanded.insert(format!("{new}/{}", &p[old.len() + 1..]));
        }
        if self.expanded.remove(old) {
            self.expanded.insert(new.to_string());
        }
    }

    /// Delete a folder: its notes go with it. The caller has warned.
    pub fn delete_folder(&mut self, rel: &str, cx: &mut Context<Self>) {
        match self.store.delete_folder(rel) {
            Ok(()) => {
                if self.current.as_deref().is_some_and(|c| c.starts_with(&format!("{rel}/"))) {
                    self.current = None;
                    self.editor = None;
                    self.editor_sub = None;
                }
                self.expanded.remove(rel);
                self.entries = self.store.list();
                self.folders = self.store.folders();
                self.sort();
                self.show_toast(crate::settings::text("已移到回收站"), cx);
            }
            Err(e) if e.downcast_ref::<crate::vault::NoTrash>().is_some() => {
                self.confirm = Some(Confirm::DeletePermanent { ids: vec![], folder: Some(rel.to_string()) });
            }
            Err(e) => self.status = SaveStatus::Failed(crate::localized_format!("删除失败：{}", "Could not delete folder: {}", crate::settings::diagnostic(&e.to_string()))),
        }
        cx.notify();
    }

    /// 新建文件夹 in the open note's folder (or beside the one just picked).
    pub fn new_folder(&mut self, parent: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let parent = parent.or_else(|| self.current.as_deref().and_then(crate::tree::folder_of)).unwrap_or_default();
        let p = parent.clone();
        match self.store.create_folder(&p, crate::settings::text("新建文件夹")) {
            Ok(rel) => {
                self.expanded.insert(parent);
                self.folders = self.store.folders();
                self.show_toast_kind(ToastKind::Info, &crate::localized_format!("已创建文件夹 {}", "Created folder {}", rel), cx);
            }
            Err(e) => self.status = SaveStatus::Failed(crate::localized_format!("新建文件夹失败：{}", "Could not create folder: {}", crate::settings::diagnostic(&e.to_string()))),
        }
        let _ = window;
        cx.notify();
    }

    /// Click the title to rename the note's heading in place.
    pub fn begin_title_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current.is_none() || self.reading() {
            return;
        }
        let title = self
            .current
            .as_ref()
            .and_then(|id| self.entries.iter().find(|e| &e.id == id))
            .map(|e| e.title.clone())
            .unwrap_or_default();
        let field = cx.new(|cx| TextField::new(crate::settings::text("笔记标题"), cx));
        field.update(cx, |f, cx| {
            f.size = 14.;
            f.line_height = 20.;
            f.set_text(&title, cx);
            f.select_all_text(cx);
        });
        window.focus(&field.focus_handle(cx));
        let subs = vec![cx.subscribe_in(&field, window, |w, _, ev: &TextFieldEvent, window, cx| match ev {
            TextFieldEvent::Submit => {
                let title = w.title_edit.as_ref().map(|t| t.field.read(cx).text.clone()).unwrap_or_default();
                w.finish_title_edit(Some(title), window, cx);
            }
            TextFieldEvent::Cancel => w.finish_title_edit(None, window, cx),
            _ => cx.notify(),
        })];
        self.title_edit = Some(TitleEdit { field, _subs: subs });
        cx.notify();
    }

    /// Commit (`Some`) or abandon (`None`) a title edit.
    pub fn finish_title_edit(&mut self, title: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.title_edit.take() else { return };
        let _ = edit;
        if let Some(title) = title {
            let title = title.trim().to_string();
            let old = self.current.as_ref().and_then(|id| self.entries.iter().find(|e| &e.id == id)).map(|e| e.title.clone()).unwrap_or_default();
            if !title.is_empty() && title != old {
                if let Some(ed) = &self.editor {
                    let ed = ed.clone();
                    ed.update(cx, |e, cx| e.set_title(&title, cx));
                }
                if let Some(e) = self.current.as_ref().and_then(|id| self.entries.iter_mut().find(|e| &e.id == id)) {
                    e.title = title;
                }
                self.unsaved = true;
                self.schedule_save(window, cx);
            }
        }
        if let Some(ed) = &self.editor {
            ed.read(cx).focus(window);
        }
        cx.notify();
    }

    fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // A Markdown note has no title of its own: its name is the sidebar
        // entry's title, which comes from the first heading.
        let title = self
            .current
            .as_ref()
            .and_then(|id| self.entries.iter().find(|e| &e.id == id))
            .map(|e| display_title(&e.title))
            .unwrap_or_default();
        let save_error = match &self.status {
            SaveStatus::Failed(error) => Some(error.clone()),
            _ => None,
        };
        // Independent icon controls, with equal top and right insets.
        let seg = crate::ui::header_icon;
        let divider = || div().flex_none().w(px(1.)).h(px(18.)).bg(Theme::line_divider());
        // Breadcrumbs: the folders the note lives in, then the note itself.
        // Long chains keep the first and last and fold the middle into "…".
        let chain: Vec<(String, String, Option<String>)> = self
            .current
            .as_deref()
            .map(|id| {
                let mut out = vec![];
                let mut parts: Vec<&str> = id.split('/').collect();
                parts.pop();
                let mut prefix = String::new();
                for part in parts {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(part);
                    // A folder named "a/b" is one row, keyed by its path.
                    out.push((format!("dir:{prefix}"), part.to_string(), None));
                }
                out
            })
            .unwrap_or_default();
        let mut crumbs: Vec<AnyElement> = vec![];
        let sep = || div().mx(px(3.)).flex_none().child(icon("chevron-right", 12., theme::c(craft::N500))).into_any_element();
        for (i, (id, name, em)) in chain.iter().enumerate() {
            if chain.len() > 2 && i > 0 && i < chain.len() - 1 {
                if i == 1 {
                    let hidden: Vec<&str> = chain[1..chain.len() - 1].iter().map(|c| c.1.as_str()).collect();
                    crumbs.push(
                        div()
                            .id("crumb-more")
                            .no_drag()
                            .flex_none()
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .text_color(Theme::text_caption())
                            .tip(&hidden.join(" / "), None)
                            .child("…")
                            .into_any_element(),
                    );
                    crumbs.push(sep());
                }
                continue;
            }
            let id = id.clone();
            crumbs.push(
                div()
                    .id(SharedString::from(format!("crumb-{id}")))
                    .no_drag()
                    .min_w(px(28.))
                    .max_w(px(200.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(Theme::text_caption())
                    .hover(|s| s.text_color(Theme::primary_500()))
                    .when_some(em.clone(), |d, em| {
                        d.child(div().size(px(20.)).mr(px(4.)).flex_none().flex().items_center().justify_center().text_size(px(16.)).child(em))
                    })
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(name.clone())))
                    .on_click(cx.listener(move |w, _, window, cx| {
                        if let Some(dir) = id.strip_prefix("dir:") {
                            // A folder crumb opens the folder: its first note,
                            // with the folder expanded in the sidebar.
                            w.expanded.insert(dir.to_string());
                            let dir = dir.to_string();
                            if let Some(first) = w.entries.iter().find(|e| crate::tree::folder_of(&e.id).as_deref() == Some(dir.as_str())).map(|e| e.id.clone()) {
                                w.open(&first, window, cx);
                            } else {
                                cx.notify();
                            }
                        } else {
                            w.open(&id, window, cx);
                        }
                    }))
                    .into_any_element(),
            );
            crumbs.push(sep());
        }
        let record_more = self.more_btn_bounds.clone();
        div()
            .id("top-bar").relative()
            .h(px(HEADER_H))
            .flex_none()
            .flex()
            .items_center()
            .pr(px(crate::ui::header_right_inset()))
            // Clear of the traffic lights, which sit over the note when the sidebar is closed.
            .pl(if self.sidebar_open { px(18.) } else { px(theme::TRAFFIC_LIGHTS_W) })
            .title_bar()
            .when(!self.sidebar_open, |d| {
                d.child(
                    div()
                        .id("open-sidebar")
                        .no_drag()
                        .flex_none()
                        .ml(px(4.))
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
                .child(
                    div().id("header-home").no_drag().flex_none().ml(px(4.)).size(px(28.))
                        .flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
                        .hover(|s| s.bg(Theme::fill_hover_strong()))
                        .active(|s| s.bg(Theme::fill_pressed()))
                        .tip(crate::settings::text("返回主页"), None)
                        .child(icon("home", 18., Theme::icon()))
                        .on_click(cx.listener(|w, _, _, cx| w.show_view(View::Home, cx))),
                )
                .child(div().mx(px(12.)).child(divider()))
            })
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .h(px(20.))
                            .min_w_0()
                            .flex()
                            .items_center()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .children(crumbs)
                            .child(self.title_el(title, cx)),

                    )
                    .when_some(save_error, |d, error| d.child(
                        div().min_w_0().max_w(px(220.)).text_size(px(12.)).line_height(px(18.))
                            .text_color(theme::c(craft::R400)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(error),
                    )),
            )
            .child(div().flex_1().min_w(px(16.)))
            .child(
                div().gap(px(4.))
                    .h(px(HEADER_ACTIONS_H)).flex_none()
                    .flex()
                    .items_center()
                    .when(self.current.is_some(), |d| {
                        d.child(
                            // No tooltip while its dropdown is open; the menu hangs
                            // from this button's right edge.
                            seg("doc-more", "ellipsis")
                                .relative()
                                .when(self.doc_menu, |d| d.bg(Theme::fill_hover()))
                                .when(!self.doc_menu, |d| d.tip(crate::settings::tr("更多", "More"), None))
                                .child(canvas(move |b, _, _| record_more.set(Some(b)), |_, _, _, _| {}).absolute().inset_0())
                                .on_click(cx.listener(|w, _, _, cx| {
                                    w.doc_menu = !w.doc_menu;
                                    w.doc_sub = None;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        seg("doc-search", "search")
                            .tip_label(crate::settings::text("搜索(⌘ + J)").into(), TipSide::Below)
                            .on_click(cx.listener(|_, _, window, cx| window.dispatch_action(Box::new(FocusSearch), cx))),
                    )
                    .child(
                        // Straight to a new document: no 新建 menu, as 导入 is rare.
                        seg("doc-new", "plus").tip(crate::settings::tr("新建文档", "New document"), None).on_click(cx.listener(|w, _, window, cx| w.new_doc(window, cx))),
                    ).into_any_element()
            )
    }
}

/// Header and action group share an 8px top/right inset.
pub const HEADER_H: f32 = 48.;
pub(crate) const HEADER_ACTIONS_H: f32 = 32.;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.native_sidebar.sync(if self.sidebar_open { self.sidebar_w(window) } else { 0. }, window);
        let sidebar = if self.sidebar_open { Some(if matches!(self.view, View::Settings) { self.render_settings_sidebar(window, cx) } else { self.render_sidebar(window, cx) }) } else { None };
        let row_menu = self.render_row_menu(window, cx);
        let resize = self.sidebar_open.then(|| self.render_resize_handle(window, cx));
        let resize_overlay = self.render_resize_overlay(cx);
        let search = self.render_search(window, cx);
        let doc_menu = self.render_doc_menu(window, cx);
        let doc_info = self.render_doc_info(cx);
        let confirm = self.render_confirm(cx);
        let home_menus = self.render_home_menus(cx);
        let theme_menu = self.render_theme_menu(cx);
        let toast = self.render_toast(window);
        let mode_menu = self.render_mode_menu(cx);
        let main = match self.view {
            View::Doc => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .child(self.top_bar(cx))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .children(self.editor.clone()),
                        )
                        .children(self.render_shortcuts(cx)),
                )
                .into_any_element(),
            View::Home => self.render_home(window, cx),
            View::Settings => self.render_settings(cx),
        };
        let move_dlg = self.render_move_dialog(cx);
        let export_md = self.render_export_md(cx);
        // Only the note stands out: a white plane flush with the window's top,
        // right and bottom edges. The sidebar sits sunken on the window
        // surface beside it; the change of value is the only line between
        // them (no gutter, radius, border or shadow).
        // A document sits on its theme's page colour.
        let page = if self.view == View::Doc { crate::editor_theme::Theme::bg_body() } else { Theme::bg_body() };
        let note = div().id("note").relative().flex_1().min_w_0().h_full().flex().bg(page).child(main);
        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .bg(Theme::window())
            .font_family(theme::UI_FONT)
            .text_color(Theme::text())
            // Window keys act on the window they were pressed in, not on
            // whichever window macOS considers active.
            .on_action(cx.listener(|w, _: &CloseWindow, window, cx| {
                crate::quick_note::persist(cx);
                if w.save_before(cx, |_, window, _| window.remove_window()) { window.remove_window(); }
            }))
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Zoom, window, _| window.zoom_window())
            .on_action(|_: &ToggleFullScreen, window, _| window.toggle_fullscreen())
            .on_action(cx.listener(|w, _: &OpenSettings, _, cx| w.show_view(View::Settings, cx)))
            .on_action(cx.listener(|w, _: &OpenAbout, _, cx| w.open_settings(crate::settings::Page::About, cx)))
            .on_action(cx.listener(|w, _: &GoHome, _, cx| w.show_view(View::Home, cx)))
            .on_action(cx.listener(|w, _: &NewDoc, window, cx| w.new_doc(window, cx)))
            .on_action(cx.listener(|w, _: &ImportDoc, window, cx| w.open_import(window, cx)))
            .on_action(cx.listener(|w, _: &ToggleSidebar, _, cx| {
                w.sidebar_open = !w.sidebar_open;
                cx.notify();
            }))
            .on_action(cx.listener(|w, _: &FindInDoc, window, cx| w.find_in_doc(false, window, cx)))
            .on_action(cx.listener(|w, _: &ReplaceInDoc, window, cx| {
                let replace = !w.reading();
                w.find_in_doc(replace, window, cx)
            }))
            .on_action(cx.listener(|w, _: &ToggleSource, window, cx| w.toggle_source(window, cx)))
            .on_action(cx.listener(|w, _: &crate::modes::EditMode, window, cx| w.set_mode(false, window, cx)))
            .on_action(cx.listener(|w, _: &crate::modes::ReadMode, window, cx| w.set_mode(true, window, cx)))
            .on_action(cx.listener(|w, _: &crate::modes::ToggleShortcuts, _, cx| {
                w.shortcuts = !w.shortcuts;
                cx.notify();
            }))
            .on_key_down(cx.listener(|w, ev: &KeyDownEvent, window, cx| {
                // The theme dropdown: ↑ ↓ preview, ↩ applies, Esc closes it (not 设置).
                if w.theme_menu.is_some() && w.theme_menu_key(&ev.keystroke.key, cx) {
                    cx.stop_propagation();
                    return;
                }
                // Esc leaves 设置 (unless its filter field has the caret).
                if ev.keystroke.key == "escape" && w.view == View::Settings && !w.settings_filter.read(cx).is_focused(window) {
                    w.close_settings(cx);
                    cx.stop_propagation();
                    return;
                }
                if ev.keystroke.key == "escape" && (w.doc_info.is_some() || w.doc_menu || w.mode_menu) {
                    w.doc_info = None;
                    w.mode_menu = false;
                    w.doc_menu = false;
                    w.doc_sub = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            // Esc leaves 批量操作 (closing its menu or confirmation first), even from the editor.
            .capture_key_down(cx.listener(|w, ev: &KeyDownEvent, _, cx| {
                if w.record_shortcut(ev, cx) { return; }
                // Esc cancels a confirmation dialog.
                if ev.keystroke.key == "escape" && (w.confirm.is_some() || w.export_md.is_some()) {
                    w.confirm = None;
                    w.after_save = None;
                    w.export_md = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if ev.keystroke.key != "escape" || w.batch.is_none() || w.search_open || w.rename.is_some() {
                    return;
                }
                if w.row_menu.is_some() {
                    w.row_menu = None;
                } else {
                    w.batch = None;
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|w, _: &FocusSearch, window, cx| {
                if w.search_open { w.close_search(window, cx); } else { w.open_search(window, cx); }
                cx.stop_propagation();
            }))
            .children(sidebar)
            .child(note)
            .children(resize)
            .children(crate::ui::window_controls(window))
            .children(row_menu)
            .children(doc_menu)
            .children(mode_menu)
            .children(doc_info)
            .children(home_menus)
            .children(theme_menu)
            .children(confirm)
            .children(move_dlg)
            .children(export_md)
            .children(search)
            .children(resize_overlay)
            .children(toast)
    }
}

impl Workspace {
    /// ⌘/ : show the open note as Markdown source, or go back to the page.
    pub fn toggle_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = self.editor.clone() else { return };
        ed.update(cx, |e, cx| e.toggle_source(window, cx));
        ed.read(cx).focus(window);
        if self.unsaved || ed.read(cx).core.dirty {
            self.unsaved = true;
            self.schedule_save(window, cx);
        }
        cx.notify();
    }
}
