#![recursion_limit = "512"]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod shared_store;
mod quick_note;
mod assets;
mod clipboard;
mod devtools;
mod docinfo;
mod docmenu;
mod editor;
mod highlight;
mod home;
mod html;
mod input;
mod modes;
#[cfg(target_os = "macos")]
mod native_header;
#[cfg(not(target_os = "macos"))]
#[path = "native_header_fallback.rs"]
mod native_header;
mod printing;
mod search;
mod sidebar;
mod settings;
mod themes;
mod shortcuts;
mod feishu_theme;
mod editor_theme;
mod editor_ui;
mod theme;
mod transfer;
mod dialogs;
mod tree;
mod ui;

use hush_core::{docxml, lang, md, model, palette, paste, source, store, time, vault};

use gpui::*;

#[cfg(target_os = "macos")]
fn set_app_icon() {
    use objc::{class, msg_send, sel, sel_impl};
    use objc::runtime::Object;
    let png = include_bytes!("../assets/branding/app-icon.png");
    unsafe {
        let data: *mut Object = msg_send![class!(NSData), dataWithBytes:png.as_ptr() length:png.len()];
        let image: *mut Object = msg_send![class!(NSImage), alloc];
        let image: *mut Object = msg_send![image, initWithData:data];
        if !image.is_null() {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, setApplicationIconImage:image];
            let _: () = msg_send![image, release];
        }
    }
}

/// The main window, centred at the remembered size.
fn open_main_window(scripted: bool, cx: &mut App) {
    // Scripted dev runs must not steal keyboard focus from whatever the
    // user is typing into.
    let bounds = Bounds::centered(None, quick_note::main_window_size(cx), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_background: WindowBackgroundAppearance::Blurred,
            titlebar: Some(TitlebarOptions {
                title: Some("hush.md".into()),
                appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
                traffic_light_position: Some(point(px(20.), px(app::HEADER_H / 2. - 7.))),
            }),
            window_min_size: Some(size(px(480.), px(480.))),
            focus: !scripted,
            ..Default::default()
        },
        |window, cx| {
            devtools::start(window, cx);
            cx.new(|cx| app::Workspace::new(window, cx))
        },
    )
    .expect("open window");
}

/// A window opened on one Markdown file, Typora style: that file, no library.
pub fn open_file_window(path: std::path::PathBuf, cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(900.), px(760.)), cx);
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_background: WindowBackgroundAppearance::Blurred,
            titlebar: Some(TitlebarOptions {
                title: Some(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "hush.md".into()).into()),
                appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
                traffic_light_position: Some(point(px(20.), px(app::HEADER_H / 2. - 7.))),
            }),
            window_min_size: Some(size(px(480.), px(480.))),
            ..Default::default()
        },
        move |window, cx| cx.new(|cx| app::Workspace::new_file_window(path, window, cx)),
    );
    if let Err(e) = result {
        eprintln!("无法打开文件：{e}");
    }
}

/// 打开文件…: pick a Markdown file and open it in a window of its own, the
/// way double-clicking it in Finder would.
pub fn open_file_dialog(cx: &mut App) {
    let exts: Vec<SharedString> = vault::EXTENSIONS.iter().map(|e| SharedString::from(*e)).collect();
    let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("打开文件".into()), extensions: Some(exts) });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = rx.await else { return };
        let Some(path) = paths.into_iter().next() else { return };
        let _ = cx.update(|cx| {
            // A file window owns the process's vault (`store::set_root`), so
            // it gets a process of its own rather than repointing the open
            // library.
            match std::env::current_exe() {
                Ok(exe) => { let _ = std::process::Command::new(exe).arg(&path).spawn(); }
                Err(_) => open_file_window(path, cx),
            }
        });
    })
    .detach();
}

/// The first `.md` / `.markdown` path on the command line, if any.
fn file_argument() -> Option<std::path::PathBuf> {
    std::env::args_os().skip(1)
        .map(std::path::PathBuf::from)
        .find(|p| vault::is_note_path(p) && p.is_file())
}

fn main() {
    let scripted = std::env::var_os("FDL_SCRIPT").is_some();
    // No single-instance guard: copies may share a library (builds tested
    // side by side). `Store` rereads an index another copy rewrote before
    // writing its own.
    let app = Application::new().with_assets(assets::Assets);
    // macOS keeps running after ⌘W closes the last window (quick notes stay
    // available); clicking the Dock icon then brings the main window back.
    app.on_reopen(|cx| if cx.windows().is_empty() { open_main_window(false, cx); });
    // 用 hush.md 打开 (Finder, Explorer, `open -a`): one window per file.
    //
    // A file window treats the file's own folder as its vault, which is a
    // process-wide choice (`store::set_root`). Opening one inside a running
    // library would repoint that library's image and link resolution, so a
    // file that arrives while we are running starts a new process instead;
    // the file gets its own window and the library is untouched.
    app.on_open_urls(|urls| {
        // Before `run` starts, nothing is open yet; after it, the main window is.
        let running = STARTED.with(|c| c.get());
        for url in urls {
            let path = url.strip_prefix("file://").map(transfer::percent_decode).unwrap_or(url.clone());
            let path = std::path::PathBuf::from(path);
            if !(vault::is_note_path(&path) && path.is_file()) {
                continue;
            }
            if running {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg(&path).spawn();
                }
            } else {
                // Launched by opening a file: the startup path below takes it.
                PENDING.with(|p| p.borrow_mut().push(path));
            }
        }
    });
    app.run(move |cx: &mut App| {
        STARTED.with(|c| c.set(true));
        #[cfg(target_os = "macos")]
        set_app_icon();
        editor::math::load_fonts(cx);
        input::bind_keys(cx);
        editor::source_view::bind_keys(cx);
        editor::keymap::bind_keys(cx);
        editor::image::bind_keys(cx);
        app::bind_keys(cx);
        settings::load();
        themes::load(&settings::prefs().theme_id(), false, || cx.text_system().all_font_names());
        shortcuts::init(cx);
        let quick_enabled = !scripted;
        // A file window picks its own vault, so the global store must be
        // created after `set_root` — `new_file_window` does it. Only a
        // library start opens the default one here.
        let mut files: Vec<std::path::PathBuf> = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
        files.extend(file_argument());
        if files.is_empty() {
            cx.set_global(shared_store::SharedStore::open().expect("open document library"));
        }
        if quick_enabled { quick_note::install(cx); }
        app::set_menus(cx);
        // `hush_md notes/a.md`, or the Finder handing us the file that
        // launched the app: a window for that file and nothing else.
        let single = !files.is_empty();
        for path in files {
            open_file_window(path, cx);
        }
        if !single {
            open_main_window(scripted, cx);
        }
        cx.on_window_closed(move |cx| {
            if (!quick_enabled || !cfg!(target_os = "macos")) && cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if !scripted {
            cx.activate(true);
        }
    });
}

thread_local! {
    /// `run` has started (so a library window may be open).
    static STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Files the system asked us to open before `run` started.
    static PENDING: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}
