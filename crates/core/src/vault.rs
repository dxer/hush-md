//! The vault: a folder of Markdown files, opened as is.
//!
//! There is no index and no database. The tree on disk *is* the tree in the
//! UI: every `.md` file under the opened folder is a note, every subfolder is
//! a branch, and a note's title is its first heading (falling back to its
//! file name). Saves write back to the same file, atomically; nothing else in
//! the folder is touched.
//!
//! Settings that belong to the vault rather than to a note (sidebar width,
//! theme, pinned order) live in `.hush/settings.json` inside it, which is the
//! only file this app creates on its own.

use crate::model::{display_title, Document};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// What the app calls a note with no heading and no usable file name.
pub const UNTITLED_FILE: &str = "未命名";
pub const EXT: &str = "md";

/// A folder in the vault that holds notes (directly or deeper).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Path relative to the vault root, `/`-separated, no trailing slash.
    pub path: String,
    pub name: String,
    pub parent: Option<String>,
}

/// A note found in the vault.
#[derive(Clone, Debug)]
pub struct Note {
    /// Path relative to the vault root, with `/` separators.
    pub path: String,
    /// The file's name without `.md`.
    pub name: String,
    /// First heading, or the file name.
    pub title: String,
    pub folder: Option<String>,
    pub modified: u64,
    pub created: u64,
    pub size: u64,
    /// Set while the note is open and has unsaved changes.
    pub dirty: bool,
}

impl Note {
    pub fn id(&self) -> &str {
        &self.path
    }
}

/// Vault-wide settings use the app's existing `store::Settings`, so every
/// preference the UI already saves keeps loading and saving as before.
pub type Settings = crate::store::Settings;

pub struct Vault {
    root: PathBuf,
    settings: Settings,
    /// Titles found on the last scan, so the sidebar does not reopen files.
    titles: HashMap<String, (u64, String)>,
    pub warnings: Vec<String>,
}

impl Vault {
    /// `HUSH_MD_HOME`, else `HUSH.md` in the user's Documents.
    pub fn default_root() -> PathBuf {
        std::env::var_os("HUSH_MD_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::document_dir().or_else(|| dirs::home_dir().map(|h| h.join("Documents"))).unwrap_or_else(|| PathBuf::from(".")).join("Hush.md"))
    }

    pub fn open(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root).with_context(|| format!("无法创建文件夹 {}", root.display()))?;
        let settings = Self::load_settings(&root);
        Ok(Vault { root, settings, titles: HashMap::new(), warnings: vec![] })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn meta_dir(&self) -> PathBuf {
        self.root.join(".hush")
    }

    fn settings_path(&self) -> PathBuf {
        self.meta_dir().join("settings.json")
    }

    fn load_settings(root: &Path) -> Settings {
        std::fs::read(root.join(".hush/settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn save_settings(&mut self, s: Settings) -> Result<()> {
        self.settings = s;
        std::fs::create_dir_all(self.meta_dir())?;
        write_atomic(&self.settings_path(), &serde_json::to_vec_pretty(&self.settings)?)
    }

    /// Every note in the vault, sorted by folder then title.
    pub fn list(&mut self) -> Vec<Note> {
        let mut out = vec![];
        let root = self.root.clone();
        self.walk(&root, &mut out);
        for n in &mut out {
            n.dirty = false;
        }
        out.sort_by(|a, b| (a.folder.as_deref().unwrap_or(""), a.title.to_lowercase()).cmp(&(b.folder.as_deref().unwrap_or(""), b.title.to_lowercase())));
        out
    }

    /// Every folder that holds a note, directly or below it. A folder exists
    /// in the vault because it has notes; empty ones are not listed.
    pub fn folders(&self) -> Vec<Folder> {
        let mut dirs: Vec<String> = vec![];
        let mut holding: std::collections::HashSet<String> = std::collections::HashSet::new();
        self.scan_dirs(&self.root, &mut dirs, &mut holding);
        let mut out: Vec<Folder> = dirs
            .into_iter()
            .filter(|d| holding.contains(d))
            .map(|path| {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                let parent = path.rsplit_once('/').map(|(d, _)| d.to_string());
                Folder { path, name, parent }
            })
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Collects every folder, and the folders that contain a note somewhere
    /// under them.
    fn scan_dirs(&self, dir: &Path, dirs: &mut Vec<String>, holding: &mut std::collections::HashSet<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                if let Ok(rel) = path.strip_prefix(&self.root) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    dirs.push(rel.clone());
                    self.scan_dirs(&path, dirs, holding);
                    if self.has_note_below(&path) {
                        holding.insert(rel);
                    }
                }
            } else if path.extension().is_some_and(|x| x == EXT)
                && let Some(parent) = path.parent().and_then(|p| p.strip_prefix(&self.root).ok())
            {
                let mut cur = parent.to_string_lossy().replace('\\', "/");
                loop {
                    if cur.is_empty() || !holding.insert(cur.clone()) {
                        break;
                    }
                    match cur.rsplit_once('/') {
                        Some((d, _)) => cur = d.to_string(),
                        None => break,
                    }
                }
            }
        }
    }

    /// True when a note lives in `dir` or below it.
    fn has_note_below(&self, dir: &Path) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else { return false };
        for e in rd.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                if self.has_note_below(&path) {
                    return true;
                }
            } else if path.extension().is_some_and(|x| x == EXT) {
                return true;
            }
        }
        false
    }

    /// Notes directly inside `folder` (`None` = the vault root).
    pub fn notes_in(&self, folder: Option<&str>) -> Vec<PathBuf> {
        let dir = match folder {
            None => self.root.clone(),
            Some(f) => self.root.join(f),
        };
        let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
        let mut out: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == EXT))
            .collect();
        out.sort();
        out
    }

    fn walk(&mut self, dir: &Path, out: &mut Vec<Note>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            // Hidden folders hold our own settings; skip them and junk.
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                self.walk(&path, out);
                continue;
            }
            if path.extension().is_none_or(|x| x != EXT) {
                continue;
            }
            let Ok(rel) = path.strip_prefix(&self.root) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let folder = rel.rsplit_once('/').map(|(d, _)| d.to_string());
            let (created, modified, size) = stat(&path);
            let title = self.title_of(&rel, &path, modified);
            out.push(Note { path: rel, name: stem, title, folder, modified, created, size, dirty: false });
        }
    }

    /// A note's display title: its first ATX heading, else its file name.
    /// Cached against the file's mtime, so a big vault only reads what moved.
    fn title_of(&mut self, rel: &str, path: &Path, modified: u64) -> String {
        if let Some((m, t)) = self.titles.get(rel)
            && *m == modified
        {
            return t.clone();
        }
        let title = first_heading(path).unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().trim().to_string());
        let t = display_title(&title);
        self.titles.insert(rel.to_string(), (modified, t.clone()));
        t
    }

    pub fn read(&self, rel: &str) -> Result<Document> {
        let path = self.resolve(rel)?;
        let text = std::fs::read_to_string(&path).with_context(|| format!("无法读取 {}", path.display()))?;
        Ok(crate::md::read(&text))
    }

    /// Read the raw file, for the source view.
    pub fn read_text(&self, rel: &str) -> Result<String> {
        let path = self.resolve(rel)?;
        std::fs::read_to_string(&path).with_context(|| format!("无法读取 {}", path.display()))
    }

    pub fn write_text(&self, rel: &str, text: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        write_atomic(&path, text.as_bytes())
    }

    pub fn write(&self, rel: &str, doc: &Document) -> Result<()> {
        self.write_text(rel, &crate::md::write(doc))
    }

    /// Resolve a vault-relative path, refusing anything that escapes it.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        if rel.split('/').any(|s| s == ".." || s.is_empty()) || Path::new(rel).is_absolute() {
            anyhow::bail!("非法的路径：{rel}");
        }
        Ok(self.root.join(rel))
    }

    /// A fresh, unused `名称.md` beside `folder` ("" for the root).
    pub fn new_note(&self, folder: &str, title: &str) -> Result<String> {
        let base = sanitize(title);
        let base = if base.is_empty() { UNTITLED_FILE.to_string() } else { base };
        let dir = self.root.join(folder);
        std::fs::create_dir_all(&dir)?;
        let mut n = 0;
        loop {
            let name = if n == 0 { format!("{base}.{EXT}") } else { format!("{base} {n}.{EXT}") };
            let rel = if folder.is_empty() { name.clone() } else { format!("{folder}/{name}") };
            let path = dir.join(&name);
            if !path.exists() {
                write_atomic(&path, format!("# {}\n\n", title.trim()).as_bytes())?;
                return Ok(rel);
            }
            n += 1;
            if n > 9999 { anyhow::bail!("无法创建文件"); }
        }
    }

    /// Rename a folder (its notes move with it).
    pub fn rename_folder(&self, rel: &str, name: &str) -> Result<String> {
        let path = self.resolve(rel)?;
        let parent = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let stem = sanitize(name);
        if stem.is_empty() {
            anyhow::bail!("目录名不能为空");
        }
        let target = self.root.join(parent).join(&stem);
        let new_rel = if parent.is_empty() { stem } else { format!("{parent}/{stem}") };
        if new_rel == rel {
            return Ok(new_rel);
        }
        if target.exists() {
            anyhow::bail!("同名文件夹已存在");
        }
        std::fs::rename(&path, &target)?;
        Ok(new_rel)
    }

    /// Delete a folder and everything in it.
    pub fn delete_folder(&self, rel: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        std::fs::remove_dir_all(&path).with_context(|| format!("无法删除 {}", path.display()))
    }

    /// Rename a note's file, keeping it in place.
    pub fn rename(&self, rel: &str, title: &str) -> Result<String> {
        let path = self.resolve(rel)?;
        let dir = path.parent().unwrap_or(&self.root);
        let folder = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let stem = sanitize(title);
        let stem = if stem.is_empty() { UNTITLED_FILE.to_string() } else { stem };
        let mut n = 0;
        loop {
            let name = if n == 0 { format!("{stem}.{EXT}") } else { format!("{stem} {n}.{EXT}") };
            let target = dir.join(&name);
            if target == path { return Ok(rel.to_string()); }
            if !target.exists() {
                std::fs::rename(&path, &target)?;
                return Ok(if folder.is_empty() { name } else { format!("{folder}/{name}") });
            }
            n += 1;
            if n > 9999 { anyhow::bail!("无法重命名"); }
        }
    }

    pub fn delete(&self, rel: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        // The file goes away; the caller warns before calling this.
        std::fs::remove_file(&path).with_context(|| format!("无法删除 {}", path.display()))
    }

    /// Move the note's file to a new folder (`""` = vault root).
    pub fn move_note(&self, rel: &str, folder: &str) -> Result<String> {
        let path = self.resolve(rel)?;
        let dir = self.root.join(folder);
        std::fs::create_dir_all(&dir)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let target = dir.join(&name);
        let new_rel = if folder.is_empty() { name.clone() } else { format!("{folder}/{name}") };
        if new_rel == rel { return Ok(new_rel); }
        std::fs::rename(&path, &target)?;
        Ok(new_rel)
    }
}

/// Rewrite the first ATX heading of a file's text; used when a note's title
/// changes. Returns the text unchanged when there is no heading.
pub fn set_title(text: &str, title: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut done = false;
    for line in text.split_inclusive('\n') {
        if !done && line.trim_start().starts_with("# ") {
            let (body, nl) = match line.strip_suffix('\n') {
                Some(b) => (b, "\n"),
                None => (line, ""),
            };
            let _ = body;
            out.push_str(&format!("# {title}{nl}"));
            done = true;
        } else {
            out.push_str(line);
        }
    }
    if done {
        out
    } else if text.is_empty() {
        format!("# {title}\n")
    } else {
        format!("# {title}\n\n{text}")
    }
}

/// The first ATX heading in a file, if any.
pub fn first_heading(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    use std::io::{BufRead, BufReader};
    let mut in_fence = false;
    for line in BufReader::new(f).lines().take(400).map_while(Result::ok) {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(rest) = t.strip_prefix("# ") {
            let h = rest.trim().trim_end_matches('#').trim();
            if !h.is_empty() {
                return Some(h.to_string());
            }
        }
    }
    None
}

/// A title as a file name: no path separators or reserved characters.
pub fn sanitize(title: &str) -> String {
    let mut out = String::new();
    for c in title.trim().chars() {
        match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => out.push(' '),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    let t = out.trim().trim_matches('.').trim();
    // Keep names usable on every platform.
    let t = if t.chars().count() > 80 { t.chars().take(80).collect::<String>() } else { t.to_string() };
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "COM1", "LPT1"];
    if RESERVED.contains(&t.to_ascii_uppercase().as_str()) { format!("{t}_") } else { t }
}

fn stat(path: &Path) -> (u64, u64, u64) {
    let Ok(m) = std::fs::metadata(path) else { return (0, 0, 0) };
    let secs = |t: std::io::Result<std::time::SystemTime>| t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let c = secs(m.created().or_else(|_| m.modified()));
    let mo = secs(m.modified());
    (c, mo, m.len())
}

/// Write through a temporary file and rename, so a crash never leaves half a
/// note behind.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct V(PathBuf);
    impl V {
        fn new() -> Self {
            let d = std::env::temp_dir().join(format!("hush-vault-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&d).unwrap();
            V(d)
        }
        fn open(&self) -> Vault {
            Vault::open(self.0.clone()).unwrap()
        }
    }
    impl Drop for V {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_folder_of_markdown_is_a_vault() {
        let v = V::new();
        std::fs::write(v.0.join("a.md"), "# Alpha\n\nhello\n").unwrap();
        std::fs::create_dir_all(v.0.join("ideas")).unwrap();
        std::fs::write(v.0.join("ideas/b.md"), "# Beta\n").unwrap();
        std::fs::write(v.0.join(".hidden.md"), "# Nope\n").unwrap();
        std::fs::write(v.0.join("note.txt"), "x").unwrap();
        let mut vault = v.open();
        let list = vault.list();
        let titles: Vec<_> = list.iter().map(|n| n.title.as_str()).collect();
        assert!(titles.contains(&"Alpha"));
        assert!(titles.contains(&"Beta"));
        assert_eq!(list.len(), 2, "{titles:?}");
        assert!(vault.read("ideas/b.md").unwrap().blocks.iter().any(|b| matches!(b.kind, crate::model::BlockKind::Heading { level: 1, .. })));
    }

    #[test]
    fn reading_and_writing_preserves_the_file() {
        let v = V::new();
        let src = "---\ntags: [x]\n---\n\n# T\n\n**bold** text\n";
        std::fs::write(v.0.join("n.md"), src).unwrap();
        let vault = v.open();
        let doc = vault.read("n.md").unwrap();
        vault.write("n.md", &doc).unwrap();
        assert_eq!(std::fs::read_to_string(v.0.join("n.md")).unwrap(), src);
    }

    #[test]
    fn new_notes_get_a_free_name() {
        let v = V::new();
        let vault = v.open();
        let a = vault.new_note("", "Idea").unwrap();
        let b = vault.new_note("", "Idea").unwrap();
        assert_eq!(a, "Idea.md");
        assert_eq!(b, "Idea 1.md");
        // Reserved characters never reach the file system.
        let c = vault.new_note("", "a/b:c*d").unwrap();
        assert!(!c.contains('/') || c.starts_with("a b"), "{c}");
    }

    #[test]
    fn titles_follow_the_first_heading() {
        let v = V::new();
        std::fs::write(v.0.join("x.md"), "no heading\n").unwrap();
        assert_eq!(first_heading(&v.0.join("x.md")), None);
        std::fs::write(v.0.join("x.md"), "# Real\n").unwrap();
        assert_eq!(first_heading(&v.0.join("x.md")).as_deref(), Some("Real"));
        // A heading inside a fenced block is not the title.
        std::fs::write(v.0.join("x.md"), "```\n# not a title\n```\n# Yes\n").unwrap();
        assert_eq!(first_heading(&v.0.join("x.md")).as_deref(), Some("Yes"));
    }

    #[test]
    fn set_title_rewrites_the_heading() {
        assert_eq!(set_title("# Old\n\nbody\n", "New"), "# New\n\nbody\n");
        assert_eq!(set_title("body\n", "New"), "# New\n\nbody\n");
        assert_eq!(set_title("", "New"), "# New\n");
        assert_eq!(set_title("## Sub\nbody\n", "New"), "# New\n\n## Sub\nbody\n");
    }

    #[test]
    fn names_are_safe_for_every_platform() {
        assert_eq!(sanitize("a/b"), "a b");
        assert_eq!(sanitize("  spaced  "), "spaced");
        assert_eq!(sanitize("..."), "");
        assert_eq!(sanitize("CON"), "CON_");
        assert!(sanitize(&"x".repeat(200)).chars().count() <= 80);
    }
}
