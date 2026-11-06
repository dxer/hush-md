//! On-disk library: a folder of Markdown notes, plus a small JSON index
//! (pinned flag, timestamps) in the app's data directory — never in the
//! note folder.

use crate::model::Document;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub created: u64,
    pub modified: u64,
    #[serde(default)]
    pub pinned: bool,
    /// Last time the document was opened, for 最近访问.
    #[serde(default)]
    pub opened: u64,
    /// 页宽设置, remembered per document.
    #[serde(default, skip_serializing_if = "PageWidth::is_standard")]
    pub width: PageWidth,
    /// Catalogue visibility is remembered independently of document headings.
    #[serde(default = "catalogue_closed_default")]
    pub catalogue_closed: bool,
    /// 父文档: the page this one hangs under in the 我的文档库 tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Position among its siblings (milliseconds of the last move); 0 falls
    /// back to `created`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub order: u64,
    /// 收藏时间 (0: not starred).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub starred: u64,
    /// 阅读模式, remembered per document like the page width.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reading: bool,
    /// The 文档信息 row under the title.
    #[serde(default, skip_serializing_if = "InfoRow::is_default")]
    pub info: InfoRow,
    /// 隐藏引用列表 (the "…" menu's 显示引用列表 switch off).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refs_hidden: bool,
    /// True when the note's first heading differs from its file name, so a
    /// rename should also rewrite that heading.
    #[serde(default)]
    pub rename_heading: bool,
}

fn catalogue_closed_default() -> bool { true }

impl Default for Meta {
    fn default() -> Self {
        Self {
            created: 0, modified: 0, pinned: false, opened: 0,
            width: PageWidth::default(), catalogue_closed: true,
            parent: None, order: 0, starred: 0, reading: false,
            info: InfoRow::default(), refs_hidden: false, rename_heading: true,
        }
    }
}

/// The 文档信息 row under the title: which items it shows (选择要展示的文档信息)
/// and whether it is hidden altogether (隐藏文档信息).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InfoRow {
    pub hidden: bool,
    pub created: bool,
    pub modified: bool,
}

impl Default for InfoRow {
    fn default() -> Self {
        InfoRow { hidden: false, created: false, modified: true }
    }
}

impl InfoRow {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Meta {
    /// Sort key among siblings: oldest first, moved pages last.
    pub fn sibling_key(&self) -> u64 {
        if self.order > 0 { self.order } else { self.created * 1000 }
    }
}

/// Three page widths: 默认 (820px column), 较宽 (1020px), 全宽.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageWidth {
    #[default]
    Standard,
    Wide,
    Full,
}

impl PageWidth {
    fn is_standard(&self) -> bool {
        *self == PageWidth::Standard
    }

    /// Max width of the content column; `None` fills the page.
    pub fn max(self) -> Option<f32> {
        match self {
            PageWidth::Standard => Some(820.),
            PageWidth::Wide => Some(1020.),
            PageWidth::Full => None,
        }
    }
}

/// Window-level UI state kept next to the library (`settings.json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_note_size: Option<[f32; 2]>,
    /// Dragged sidebar width; `None` is the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<f32>,
    /// 隐藏引用列表's 不再提示.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refs_hide_no_ask: bool,
    /// 主页: list instead of grid, and the sort order ("visited" …).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub home_list: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_sort: Option<String>,
    /// Main window size, remembered like the quick note window's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<[f32; 2]>,
    /// 图片存放位置 in 设置 → 文档库: empty means `assets/` beside the notes.
    /// Any other value is the absolute folder pasted images go into.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub assets_dir: String,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub meta: Meta,
    pub title: String,
    /// 文档图标 emoji.
    pub icon: Option<String>,
    /// Plain text body, used for search and the list snippet.
    pub body: String,
    /// Its 日期提醒, for the system notifications.
    pub reminders: Vec<crate::model::InlineReminder>,
    /// Its @文档 mentions, for the 反向引用 of the documents they point at.
    pub links: Vec<crate::model::DocLink>,
}

/// The vault-backed store: a folder of Markdown files (see `vault.rs`).
///
/// `Entry` keeps the shape the UI has always used; behind it, a note's
/// settings (page width, reading mode, pin) live in the app's data
/// directory, keyed by the vault's path, because a Markdown file has nowhere
/// of its own to keep them and the note folder stays the user's.
pub struct Store {
    vault: crate::vault::Vault,
    /// Lowercased file text, for search. Filled on first use and refreshed
    /// when a note is saved.
    search: HashMap<String, String>,
    /// Per-note settings, by vault-relative path.
    notes: HashMap<String, Meta>,
    /// Problems found while opening the vault, to tell the user once.
    pub warnings: Vec<String>,
    /// The vault settings file as last read or written.
    settings: crate::vault::Settings,
}

/// Version of the vault's own metadata layout, kept in `format.json` in the
/// vault's data folder.
pub const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
struct FormatMarker {
    format: u32,
}

#[derive(Serialize, Deserialize, Default)]
struct NotesFile {
    #[serde(default)]
    format: u32,
    #[serde(default)]
    notes: HashMap<String, Meta>,
}

/// The folder picked in 设置 → 文档库 for pasted images (`None`: beside the
/// notes). Read from the settings file directly: image storage decides its
/// destination before a `Store` is at hand.
pub fn assets_setting() -> Option<String> {
    let meta = crate::vault::Vault::meta_root(&Store::root_dir());
    let s: Settings = std::fs::read(meta.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())?;
    (!s.assets_dir.is_empty()).then_some(s.assets_dir)
}

/// A folder opened as the library for this process, when it is not the
/// default one (`hush_md notes/a.md` opens `notes/` as the vault: the file
/// and its images resolve beside it).
static ROOT_OVERRIDE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Point the library at `root` for the rest of the process. A single-file
/// window does this so the note's own folder is the vault.
pub fn set_root(root: PathBuf) {
    *ROOT_OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = Some(root);
}

impl Store {
    /// `HUSH_MD_HOME`, else `Hush.md` in the user's Documents; or whatever
    /// [`set_root`] picked for this process.
    pub fn root_dir() -> PathBuf {
        ROOT_OVERRIDE.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(crate::vault::Vault::default_root)
    }

    /// Where copied images and attachments live, relative to the note:
    /// `assets/` beside the note (what Typora does), so the folder can be
    /// moved or synced as a whole. The setting picks a fixed folder instead;
    /// see [`Self::ensure_assets_dir`].
    pub const ASSETS_DIR: &str = "assets";

    /// The folder pasted images go into. `assets/` beside the notes by
    /// default (`None`), or the folder picked in 设置 → 文档库.
    pub fn assets_dir() -> PathBuf {
        match assets_setting() {
            Some(dir) => PathBuf::from(dir),
            None => Self::root_dir().join(Self::ASSETS_DIR),
        }
    }
    /// [`Self::assets_dir`], created first: a first paste into a fresh vault
    /// has no `assets/` yet.
    pub fn ensure_assets_dir() -> PathBuf {
        let dir = Self::assets_dir();
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// Where a pasted image was copied before the move out of `.hush/`.
    pub fn legacy_assets_dir() -> PathBuf {
        crate::vault::Vault::meta_root(&Self::root_dir()).join("assets")
    }

    /// Derived files that can be thrown away and rebuilt: thumbnails of
    /// attachments, downloaded copies of remote images. They live in the
    /// app's data directory, never in the note folder.
    pub fn cache_dir() -> PathBuf {
        crate::vault::Vault::data_root().join("cache")
    }

    /// [`Self::cache_dir`], created first.
    pub fn ensure_cache_dir() -> PathBuf {
        let dir = Self::cache_dir();
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    pub fn open() -> Result<Self> {
        Self::open_seeded(Self::root_dir(), true)
    }

    /// Open the default vault, optionally writing a welcome note into a folder
    /// that has none (`false` for a window opened on a single file).
    pub fn open_seeded(root: PathBuf, seed: bool) -> Result<Self> {
        let meta = crate::vault::Vault::meta_root(&root);
        Self::open_at_meta_seeded(root, meta, seed)
    }

    pub fn open_at(root: PathBuf) -> Result<Self> {
        Self::open_at_meta(root.clone(), crate::vault::Vault::meta_root(&root))
    }

    /// Open, with the metadata folder chosen by the caller (tests).
    pub fn open_at_meta(root: PathBuf, meta: PathBuf) -> Result<Self> {
        Self::open_at_meta_seeded(root, meta, true)
    }

    pub fn open_at_meta_seeded(root: PathBuf, meta: PathBuf, seed: bool) -> Result<Self> {
        let vault = crate::vault::Vault::open_at(root, meta)?;
        // A folder with no notes gets one to start from. An existing vault is
        // never touched.
        let empty = std::fs::read_dir(vault.root()).map(|rd| {
            rd.flatten().all(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                e.path().is_dir() || n.starts_with('.') || !crate::vault::is_note_path(&e.path())
            })
        }).unwrap_or(false);
        if seed && empty {
            let name = crate::welcome::file_name(false);
            let _ = vault.write_text(name, crate::welcome::TEXT);
        }
        let mut warnings = vec![];
        let meta_dir = vault.meta_dir().to_path_buf();
        let _ = std::fs::create_dir_all(&meta_dir);
        let marker = meta_dir.join("format.json");
        match std::fs::read(&marker).ok().and_then(|b| serde_json::from_slice::<FormatMarker>(&b).ok()) {
            Some(m) if m.format > FORMAT => warnings.push("这个文档库由更新版本的 hush.md 创建，部分内容可能无法正确显示，请升级后再编辑".into()),
            Some(_) => {}
            None => {
                let _ = write_atomic(&marker, &serde_json::to_vec_pretty(&FormatMarker { format: FORMAT })?);
            }
        }
        let notes = std::fs::read(meta_dir.join("notes.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<NotesFile>(&b).ok())
            .unwrap_or_default()
            .notes;
        let settings = vault.settings().clone();
        Ok(Store { vault, notes, warnings, settings, search: HashMap::new() })
    }

    fn save_notes(&self) -> Result<()> {
        let f = NotesFile { format: FORMAT, notes: self.notes.clone() };
        write_atomic(&self.vault.meta_dir().join("notes.json"), &serde_json::to_vec_pretty(&f)?)
    }

    /// One note by path, read straight from disk. A window opened on a single
    /// file uses this instead of [`Self::list`]: pointing the app at one `.md`
    /// in a folder of ten thousand must not scan, stat and read all of them
    /// before the first frame.
    pub fn entry_at(&mut self, id: &str) -> Option<Entry> {
        let note = self.vault.note_at(id)?;
        let mut e = self.entry(note);
        if let Ok(text) = self.vault.read_text(id) {
            e.body = self.search.entry(id.to_string()).or_insert_with(|| text.to_lowercase()).clone();
        }
        Some(e)
    }

    /// Every note in the vault, newest first? No: the sidebar sorts, so this
    /// is the folder order from `vault::list`.
    pub fn list(&mut self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self.vault.list().into_iter().map(|n| self.entry(n)).collect();
        // Drop cached text for notes that are gone.
        let live: std::collections::HashSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        self.search.retain(|id, _| live.contains(id.as_str()));
        // Fill the cache for whichever notes are missing: search reads it.
        let missing: Vec<String> = entries.iter().filter(|e| !self.search.contains_key(&e.id)).map(|e| e.id.clone()).collect();
        for id in missing {
            if let Ok(text) = self.vault.read_text(&id) {
                self.search.insert(id, text.to_lowercase());
            }
        }
        for e in &mut entries {
            e.body = self.search.get(&e.id).cloned().unwrap_or_default();
        }
        entries
    }

    /// Just the notes' text, for search, without rebuilding the list.
    pub fn body_of(&self, id: &str) -> String {
        self.search.get(id).cloned().unwrap_or_default()
    }

    /// A note file's modification time in unix seconds (0 when missing).
    pub fn modified(&self, id: &str) -> u64 {
        self.vault.resolve(id).ok().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
    }

    /// A vault-relative path as an absolute one, refusing escapes.
    pub fn resolve(&self, rel: &str) -> Result<std::path::PathBuf> {
        self.vault.resolve(rel)
    }

    /// Folders that hold notes, as vault-relative paths.
    pub fn folders(&self) -> Vec<crate::vault::Folder> {
        self.vault.folders()
    }

    /// The notes directly inside `folder`, by name: one `read_dir`, nothing
    /// read. A file window's 文件 page lists the note's own folder with this.
    pub fn siblings(&self, folder: Option<&str>) -> Vec<(String, String)> {
        self.vault.siblings(folder)
    }

    fn entry(&self, n: crate::vault::Note) -> Entry {
        let meta = self.notes.get(&n.path).cloned().unwrap_or_else(|| Meta {
            created: n.created,
            modified: n.modified,
            ..Default::default()
        });
        Entry { id: n.path, meta, title: n.title, icon: None, body: String::new(), reminders: vec![], links: vec![] }
    }

    pub fn load(&self, id: &str) -> Result<Document> {
        self.vault.read(id)
    }

    pub fn load_for_edit(&self, id: &str, _shared: bool) -> Result<Document> {
        self.vault.read(id)
    }

    pub fn save(&mut self, id: &str, doc: &Document) -> Result<Meta> {
        self.vault.write(id, doc)?;
        self.search.insert(id.to_string(), doc.plain_text().to_lowercase());
        let modified = crate::time::now();
        let meta = self.notes.entry(id.to_string()).or_default();
        meta.modified = modified;
        if meta.created == 0 {
            meta.created = modified;
        }
        let meta = meta.clone();
        self.save_notes()?;
        Ok(meta)
    }

    /// Create a note in the vault root from a document.
    pub fn create_from(&mut self, doc: &Document) -> Result<String> {
        let title = if doc.title.text.trim().is_empty() { "未命名" } else { doc.title.text.trim() };
        let rel = self.vault.new_note("", title)?;
        self.vault.write(&rel, doc)?;
        let now = crate::time::now();
        self.notes.insert(rel.clone(), Meta { created: now, modified: now, ..Default::default() });
        self.save_notes()?;
        Ok(rel)
    }

    pub fn create(&mut self) -> Result<(String, Document)> {
        self.create_in("")
    }

    /// A new empty note inside `folder` (`""` = the vault root).
    pub fn create_in(&mut self, folder: &str) -> Result<(String, Document)> {
        let doc = crate::md::empty();
        let rel = self.vault.new_note(folder, "未命名")?;
        self.vault.write(&rel, &doc)?;
        let now = crate::time::now();
        self.notes.insert(rel.clone(), Meta { created: now, modified: now, ..Default::default() });
        self.save_notes()?;
        Ok((rel, doc))
    }

    pub fn size_of(&self, id: &str, _doc: &Document) -> u64 {
        self.vault.resolve(id).ok().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len()).unwrap_or(0)
    }

    /// Move a note to the system trash. Fails with
    /// [`crate::vault::NoTrash`] when the volume has none; the note and its
    /// settings are left alone then.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        self.vault.delete(id)?;
        self.forget(id)
    }

    /// Delete a note's file for good, after the user agreed no copy is kept.
    pub fn delete_permanently(&mut self, id: &str) -> Result<()> {
        self.vault.delete_permanently(id)?;
        self.forget(id)
    }

    fn forget(&mut self, id: &str) -> Result<()> {
        self.notes.remove(id);
        self.search.remove(id);
        self.save_notes()
    }

    /// Write a note's file from exact text, without re-serializing a model.
    pub fn write_text(&mut self, id: &str, text: &str) -> Result<Option<Meta>> {
        self.vault.write_text(id, text)?;
        self.search.insert(id.to_string(), text.to_lowercase());
        let modified = crate::time::now();
        let meta = self.notes.entry(id.to_string()).or_default();
        meta.modified = modified;
        if meta.created == 0 {
            meta.created = modified;
        }
        let meta = meta.clone();
        self.save_notes()?;
        Ok(Some(meta))
    }

    /// A note's saved metadata, for a caller that wrote the file itself.
    pub fn note_meta(&mut self, id: &str) -> Result<Meta> {
        let modified = crate::time::now();
        let meta = self.notes.entry(id.to_string()).or_default();
        meta.modified = modified;
        let meta = meta.clone();
        self.save_notes()?;
        Ok(meta)
    }

    /// Copy a Markdown file into the vault under its own name. Anything else
    /// that reads as text is brought in as Markdown too, keeping its bytes.
    pub fn import(&mut self, path: &std::path::Path) -> Result<String> {
        let text = std::fs::read_to_string(path)?;
        let title = crate::vault::first_heading(path)
            .or_else(|| text.lines().find(|l| !l.trim().is_empty()).map(|l| l.trim().trim_start_matches('#').trim().to_string()))
            .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().into_owned());
        let rel = self.vault.new_note("", &title)?;
        self.vault.write_text(&rel, &text)?;
        let now = crate::time::now();
        self.notes.insert(rel.clone(), Meta { created: now, modified: now, ..Default::default() });
        self.save_notes()?;
        Ok(rel)
    }

    pub fn set_width(&mut self, id: &str, width: PageWidth) -> Result<()> {
        self.notes.entry(id.to_string()).or_default().width = width;
        self.save_notes()
    }

    pub fn update_meta(&mut self, id: &str, f: impl FnOnce(&mut Meta)) -> Result<()> {
        f(self.notes.entry(id.to_string()).or_default());
        self.save_notes()
    }

    pub fn set_catalogue(&mut self, id: &str, closed: bool) -> Result<()> {
        self.update_meta(id, |m| m.catalogue_closed = closed)
    }

    pub fn settings(&self) -> crate::vault::Settings {
        self.settings.clone()
    }

    pub fn save_settings(&mut self, s: &crate::vault::Settings) -> Result<()> {
        self.settings = s.clone();
        self.vault.save_settings(s.clone())
    }

    /// Rename a note's file to its title. Returns the new id and its meta.
    pub fn rename(&mut self, id: &str, title: &str) -> Result<(String, Meta)> {
        let new_rel = self.vault.rename(id, title)?;
        if new_rel != id && let Some(m) = self.notes.remove(id) {
            self.notes.insert(new_rel.clone(), m);
            self.save_notes()?;
        }
        let meta = self.notes.get(&new_rel).cloned().unwrap_or_default();
        Ok((new_rel, meta))
    }

    pub fn touch(&mut self, id: &str) -> Option<Meta> {
        let m = self.notes.entry(id.to_string()).or_default();
        m.opened = crate::time::now();
        let out = m.clone();
        let _ = self.save_notes();
        Some(out)
    }

    pub fn set_parent(&mut self, id: &str, parent: Option<&str>) -> Result<Meta> {
        self.notes.entry(id.to_string()).or_default().parent = parent.map(str::to_string);
        self.save_notes()?;
        Ok(self.notes.get(id).cloned().unwrap_or_default())
    }

    /// Move a note's file into `folder` (`""` = the vault root). A folder
    /// path in `parent` is used as the destination; the note's id changes
    /// with its file, so the caller updates anything holding it.
    pub fn move_to_folder(&mut self, id: &str, folder: &str) -> Result<String> {
        let new_id = self.vault.move_note(id, folder)?;
        if new_id != id {
            let mut meta = self.notes.remove(id).unwrap_or_default();
            meta.parent = None;
            self.notes.insert(new_id.clone(), meta);
            if let Some(body) = self.search.remove(id) {
                self.search.insert(new_id.clone(), body);
            }
            self.save_notes()?;
        }
        Ok(new_id)
    }

    /// Rename a folder; every note under it gets a new id.
    pub fn rename_folder(&mut self, rel: &str, name: &str) -> Result<String> {
        let new_rel = self.vault.rename_folder(rel, name)?;
        if new_rel == rel {
            return Ok(new_rel);
        }
        // Notes (and their settings) follow their folder.
        let prefix = format!("{rel}/");
        let moved: Vec<(String, String)> = self
            .notes
            .keys()
            .filter(|id| id.starts_with(&prefix))
            .map(|id| (id.clone(), format!("{new_rel}/{}", &id[prefix.len()..])))
            .collect();
        for (old, new) in &moved {
            if let Some(m) = self.notes.remove(old) {
                self.notes.insert(new.clone(), m);
            }
            if let Some(b) = self.search.remove(old) {
                self.search.insert(new.clone(), b);
            }
        }
        self.save_notes()?;
        Ok(new_rel)
    }

    /// Delete a folder and the notes in it, through the system trash.
    pub fn delete_folder(&mut self, rel: &str) -> Result<()> {
        self.vault.delete_folder(rel)?;
        self.forget_folder(rel)
    }

    /// Delete a folder and the notes in it for good, after the user agreed
    /// no copy is kept.
    pub fn delete_folder_permanently(&mut self, rel: &str) -> Result<()> {
        self.vault.delete_folder_permanently(rel)?;
        self.forget_folder(rel)
    }

    fn forget_folder(&mut self, rel: &str) -> Result<()> {
        let prefix = format!("{rel}/");
        self.notes.retain(|id, _| !id.starts_with(&prefix));
        self.search.retain(|id, _| !id.starts_with(&prefix));
        self.save_notes()
    }

    /// Create a folder in the vault (`parent` relative; `""` = root).
    pub fn create_folder(&self, parent: &str, name: &str) -> Result<String> {
        let name = crate::vault::sanitize(name);
        if name.is_empty() {
            anyhow::bail!("目录名不能为空");
        }
        let rel = if parent.is_empty() { name } else { format!("{parent}/{name}") };
        let dir = self.resolve(&rel)?;
        if dir.exists() {
            anyhow::bail!("同名文件夹已存在");
        }
        std::fs::create_dir_all(&dir)?;
        Ok(rel)
    }

    pub fn set_starred(&mut self, id: &str, on: bool) -> Result<u64> {
        let m = self.notes.entry(id.to_string()).or_default();
        m.starred = if on { crate::time::now() } else { 0 };
        let t = m.starred;
        self.save_notes()?;
        Ok(t)
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Result<()> {
        self.notes.entry(id.to_string()).or_default().pinned = pinned;
        self.save_notes()
    }
}

/// Write `bytes` to `path` through a temporary file, so a crash leaves the
/// old file intact.
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("{}.tmp", path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// An image source from a document, as a path on disk.
///
/// A relative path is tried next to the note first (Markdown files usually
/// keep their images beside them), then from the vault root (where the
/// editor copies pasted images).
pub fn image_path(src: &str) -> Option<PathBuf> {
    image_path_in(None, src)
}

/// Like [`image_path`], resolving `src` relative to the note at `note`.
///
/// Pasted images are written as `assets/<id>.png`, relative to the note's
/// own folder — which for a note in `notes/` is `notes/assets/`. Older
/// builds wrote the same text from the vault root, so a source that is not
/// beside the note is tried from there as well, and finally in the copy an
/// older version kept under `.hush/assets/`.
pub fn image_path_in(note: Option<&str>, src: &str) -> Option<PathBuf> {
    image_path_in_root(&Store::root_dir(), note, src)
}

/// [`image_path_in`] with the vault root given, for callers that hold one
/// (and for tests, which cannot set the process-wide default).
pub fn image_path_in_root(root: &Path, note: Option<&str>, src: &str) -> Option<PathBuf> {
    if src.is_empty() || src.contains("://") {
        return None;
    }
    let p = Path::new(src);
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    if let Some(note) = note
        && let Some(dir) = note.rsplit_once('/').map(|(d, _)| d)
    {
        let beside = root.join(dir).join(p);
        if beside.exists() {
            return Some(beside);
        }
    }
    let full = root.join(p);
    if full.exists() {
        return Some(full);
    }
    src.strip_prefix("assets/")
        .map(|rest| crate::vault::Vault::meta_root(root).join("assets").join(rest))
        .filter(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lib(PathBuf);

    impl Lib {
        fn new(name: &str) -> Self {
            let d = std::env::temp_dir().join(format!("hush-test-{name}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&d).unwrap();
            Lib(d)
        }
        /// Metadata in a folder of its own: the note folder stays the user's.
        fn meta(&self) -> PathBuf {
            self.0.with_extension("meta")
        }
        fn open(&self) -> Store {
            Store::open_at_meta(self.0.clone(), self.meta()).unwrap()
        }
    }

    impl Drop for Lib {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
            let _ = std::fs::remove_dir_all(self.meta());
        }
    }

    fn doc(title: &str) -> Document {
        crate::md::read(&format!("# {title}\n\nbody\n"))
    }

    #[test]
    fn an_empty_folder_starts_with_a_welcome_note() {
        let lib = Lib::new("welcome");
        let mut s = lib.open();
        let list = s.list();
        assert_eq!(list.len(), 1, "{list:?}");
        assert!(list[0].title.contains("hush.md"));
        // Opening it again does not add a second one, even after deleting it.
        // (Permanent: the system trash is the user's, not the test's.)
        s.delete_permanently(&list[0].id).unwrap();
        assert!(!lib.0.join(&list[0].id).exists());
        let mut again = Store::open_at_meta(lib.0.clone(), lib.meta()).unwrap();
        assert_eq!(again.list().len(), 1, "a vault that had notes keeps its state");
    }

    #[test]
    fn an_existing_vault_is_never_seeded() {
        let lib = Lib::new("no-seed");
        std::fs::write(lib.0.join("mine.md"), "# Mine\n").unwrap();
        let mut s = lib.open();
        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Mine");
    }

    #[test]
    fn markdown_files_are_the_library() {
        let lib = Lib::new("basic");
        std::fs::write(lib.0.join("notes.md"), "# First\n\nhello\n").unwrap();
        std::fs::create_dir(lib.0.join("sub")).unwrap();
        std::fs::write(lib.0.join("sub/deep.md"), "# Deep\n").unwrap();
        std::fs::write(lib.0.join("long.markdown"), "# Long\n").unwrap();
        std::fs::write(lib.0.join("ignore.txt"), "not a note\n").unwrap();
        let mut s = lib.open();
        let list = s.list();
        assert_eq!(list.len(), 3, "{list:?}");
        assert_eq!(list[0].title, "First");
        assert!(list.iter().any(|e| e.id == "sub/deep.md"));
        assert!(list.iter().any(|e| e.id == "long.markdown"));
        assert!(!list.iter().any(|e| e.id == "ignore.txt"));
        assert!(s.load("long.markdown").is_ok());
    }

    #[test]
    fn entry_at_reads_one_note_without_scanning_the_folder() {
        let lib = Lib::new("entry-at");
        std::fs::write(lib.0.join("keeper.md"), "# Keeper\n\nbody\n").unwrap();
        for i in 0..50 {
            std::fs::write(lib.0.join(format!("other-{i}.md")), "# Other\n").unwrap();
        }
        std::fs::create_dir(lib.0.join("sub")).unwrap();
        std::fs::write(lib.0.join("sub/deep.md"), "# Deep\n\nbody\n").unwrap();
        let mut s = lib.open();
        // A file window lists only the note it was opened on; the folder's
        // other notes stay out of `entries` but still resolve.
        let e = s.entry_at("keeper.md").unwrap();
        assert_eq!(e.title, "Keeper");
        assert!(e.body.contains("body"), "search text is filled in");
        let deep = s.entry_at("sub/deep.md").unwrap();
        assert_eq!((deep.title.as_str(), deep.meta.created > 0), ("Deep", true));
        // Not a note, not there: nothing to list.
        assert!(s.entry_at("ignore.txt").is_none());
        assert!(s.entry_at("missing.md").is_none());
        // Paths that try to leave the vault are refused like everywhere else.
        assert!(s.entry_at("../escape.md").is_none());
        // The full scan still finds everything when a library window asks.
        assert_eq!(s.list().len(), 52);
    }

    #[test]
    fn siblings_lists_one_folder_without_recursing() {
        let lib = Lib::new("siblings");
        std::fs::write(lib.0.join("keeper.md"), "# Keeper\n\nbody\n").unwrap();
        std::fs::write(lib.0.join("Other.markdown"), "# Other\n").unwrap();
        std::fs::write(lib.0.join("notes.txt"), "x").unwrap();
        std::fs::write(lib.0.join(".hidden.md"), "# Hidden\n").unwrap();
        std::fs::create_dir(lib.0.join("sub")).unwrap();
        std::fs::write(lib.0.join("sub/deep.md"), "# Deep\n").unwrap();
        let s = lib.open();
        // The vault root: two notes by name, no folders, no other files.
        assert_eq!(
            s.siblings(None),
            vec![("keeper.md".to_string(), "keeper".to_string()), ("Other.markdown".to_string(), "Other".to_string())]
        );
        // One folder in, one folder's worth: nothing from the root.
        assert_eq!(s.siblings(Some("sub")), vec![("sub/deep.md".to_string(), "deep".to_string())]);
        // A folder that is not there is an empty list, not an error.
        assert!(s.siblings(Some("nope")).is_empty());
    }

    #[test]
    fn save_writes_the_markdown_byte_for_byte() {
        let lib = Lib::new("save");
        let src = "---\ntags: [a]\n---\n\n# T\n\ntext\n";
        std::fs::write(lib.0.join("a.md"), src).unwrap();
        let mut s = lib.open();
        let id = "a.md".to_string();
        let mut d = s.load(&id).unwrap();
        s.save(&id, &d).unwrap();
        assert_eq!(std::fs::read_to_string(lib.0.join("a.md")).unwrap(), src);
        // An edit changes only its own block.
        // Block 0 is the front matter, 1 the heading, 2 the paragraph.
        d.blocks[2].text = crate::model::RichText::plain("changed");
        s.save(&id, &d).unwrap();
        assert_eq!(std::fs::read_to_string(lib.0.join("a.md")).unwrap(), src.replace("text", "changed"));
    }

    #[test]
    fn create_and_rename_follow_the_heading() {
        let lib = Lib::new("rename");
        let mut s = lib.open();
        let (id, _) = s.create().unwrap();
        assert!(lib.0.join(&id).is_file());
        let (new_id, _) = s.rename(&id, "我的笔记").unwrap();
        assert_eq!(new_id, "我的笔记.md");
        assert!(lib.0.join("我的笔记.md").is_file());
        let mut s = lib.open();
        assert_eq!(s.list()[0].title, "我的笔记");
    }

    #[test]
    fn titles_come_from_the_first_heading() {
        let lib = Lib::new("titles");
        std::fs::write(lib.0.join("file-name.md"), "no heading here\n").unwrap();
        std::fs::write(lib.0.join("headed.md"), "# Real Title\n\nbody\n").unwrap();
        let mut s = lib.open();
        let list = s.list();
        assert!(list.iter().any(|e| e.title == "file-name"));
        assert!(list.iter().any(|e| e.title == "Real Title"));
    }

    #[test]
    fn settings_survive_a_restart() {
        let lib = Lib::new("settings");
        std::fs::write(lib.0.join("a.md"), "# A\n").unwrap();
        let mut s = lib.open();
        s.set_width("a.md", PageWidth::Wide).unwrap();
        s.set_pinned("a.md", true).unwrap();
        let mut s = lib.open();
        let e = s.list().into_iter().find(|e| e.id == "a.md").unwrap();
        assert_eq!(e.meta.width, PageWidth::Wide);
        assert!(e.meta.pinned);
    }

    #[test]
    fn paths_cannot_escape_the_vault() {
        let lib = Lib::new("escape");
        let s = lib.open();
        assert!(s.load("../outside.md").is_err());
        assert!(s.load("/etc/passwd").is_err());
    }

    #[test]
    fn failed_save_leaves_the_old_file() {
        let lib = Lib::new("savefail");
        std::fs::write(lib.0.join("a.md"), "# Old\n").unwrap();
        let mut s = lib.open();
        // Make the temporary file un-writable by turning it into a folder.
        std::fs::create_dir_all(lib.0.join(".a.md.tmp")).unwrap();
        assert!(s.save("a.md", &doc("New")).is_err());
        assert_eq!(std::fs::read_to_string(lib.0.join("a.md")).unwrap(), "# Old\n");
    }

    #[test]
    fn settings_never_land_in_the_note_folder() {
        let lib = Lib::new("clean");
        std::fs::write(lib.0.join("a.md"), "# A\n").unwrap();
        let mut s = lib.open();
        s.set_pinned("a.md", true).unwrap();
        s.save_settings(&Settings { sidebar_width: Some(280.), ..Default::default() }).unwrap();
        let left: Vec<_> = std::fs::read_dir(&lib.0).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["a.md".to_string()], "the note folder holds only notes");
        assert!(lib.meta().join("notes.json").is_file());
        assert!(lib.meta().join("settings.json").is_file());
    }

    #[test]
    fn an_old_dot_hush_folder_is_moved_out_and_removed() {
        let lib = Lib::new("migrate");
        std::fs::write(lib.0.join("a.md"), "# A\n").unwrap();
        std::fs::create_dir_all(lib.0.join(".hush/themes")).unwrap();
        std::fs::write(lib.0.join(".hush/settings.json"), r#"{"sidebar_width": 311.0}"#).unwrap();
        std::fs::write(lib.0.join(".hush/themes/mine.css"), "body{}").unwrap();
        let s = lib.open();
        assert_eq!(s.settings().sidebar_width, Some(311.));
        assert!(!lib.0.join(".hush").exists(), "the old folder is gone");
        assert_eq!(std::fs::read_to_string(lib.meta().join("themes/mine.css")).unwrap(), "body{}");
    }

    #[test]
    fn images_resolve_beside_the_note_and_from_older_locations() {
        let lib = Lib::new("images");
        std::fs::create_dir_all(lib.0.join("notes/assets")).unwrap();
        std::fs::write(lib.0.join("notes/assets/pic.png"), b"x").unwrap();
        // Written by this version: relative to the note's own folder.
        assert!(image_path_in_root(&lib.0, Some("notes/a.md"), "assets/pic.png").is_some());
        // Written from the vault root by an older version.
        std::fs::create_dir_all(lib.0.join("assets")).unwrap();
        std::fs::write(lib.0.join("assets/old.png"), b"x").unwrap();
        assert!(image_path_in_root(&lib.0, None, "assets/old.png").is_some());
    }
}
