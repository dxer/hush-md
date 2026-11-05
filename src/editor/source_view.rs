//! The source view: the whole Markdown file in one text surface.
//!
//! Unlike the rendered page, there is no block tree here — the text is the
//! document. Typing, Enter, selection and undo all work on the raw file,
//! which is what makes the source view feel like an editor rather than a
//! form. The model is re-parsed from this text whenever it changes, and the
//! text itself is what gets saved.

use crate::editor::text::{LayoutCache, RichTextElement, TextSpec};
use crate::model::{Align, BlockId, RichText};
use crate::theme::Theme;
use gpui::*;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    code,
    [Backspace, Delete, Left, Right, SelectLeft, SelectRight, Home, End, SelectAll, Copy, Cut, Paste, Up, Down, Tab, ShiftTab, Save, Escape, Undo, Redo]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some("SourceView");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("cmd-left", Home, c),
        KeyBinding::new("cmd-right", End, c),
        KeyBinding::new("cmd-a", SelectAll, c),
        KeyBinding::new("cmd-c", Copy, c),
        KeyBinding::new("cmd-x", Cut, c),
        KeyBinding::new("cmd-v", Paste, c),
        KeyBinding::new("tab", Tab, c),
        KeyBinding::new("shift-tab", ShiftTab, c),
        KeyBinding::new("cmd-s", Save, c),
        KeyBinding::new("cmd-z", Undo, c),
        KeyBinding::new("cmd-shift-z", Redo, c),
        KeyBinding::new("escape", Escape, c),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceEvent {
    /// The text changed; the editor should re-parse and schedule a save.
    Changed,
    Save,
    Escape,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(u64::MAX - 2_000_000);

// The source surface undoes its own text: an undo stack of (text, caret)
// snapshots. The rendered view keeps its undo in `Core`; the two never mix
// because a source edit replaces the model wholesale.
#[derive(Default)]
struct History {
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    /// The last edit, for coalescing runs of typing.
    last: Option<(std::time::Instant, usize)>,
}

pub struct SourceView {
    focus: FocusHandle,
    pub text: String,
    /// Caret as a byte offset into `text`.
    caret: usize,
    /// Selection anchor; equals `caret` when there is no selection.
    anchor: usize,
    cache: LayoutCache,
    id: BlockId,
    pub size: f32,
    pub line_height: f32,
    pub mono: bool,
    history: History,
    selecting: bool,
    /// ⌘F in the source view: the query and the hits' byte ranges.
    pub find: Option<String>,
    pub find_hits: Vec<Range<usize>>,
    pub find_current: usize,
}

impl EventEmitter<SourceEvent> for SourceView {}

impl Focusable for SourceView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SourceView {
    pub fn new(text: &str, cx: &mut Context<Self>) -> Self {
        SourceView {
            focus: cx.focus_handle(),
            text: text.to_string(),
            caret: 0,
            anchor: 0,
            cache: Default::default(),
            id: BlockId(NEXT_ID.fetch_add(1, Ordering::Relaxed)),
            size: 14.,
            line_height: 24.,
            mono: true,
            history: History::default(),
            selecting: false,
            find: None,
            find_hits: vec![],
            find_current: 0,
        }
    }

    /// Put the caret at `offset` (clamped to a character boundary).
    #[allow(dead_code)]
    pub fn set_caret(&mut self, offset: usize, cx: &mut Context<Self>) {
        let o = self.text.ceil_char_boundary(offset.min(self.text.len()));
        self.caret = o;
        self.anchor = o;
        cx.notify();
    }

    /// Search the file's text (case-insensitive) and select the first hit.
    pub fn find(&mut self, q: &str, cx: &mut Context<Self>) {
        self.find = (!q.is_empty()).then(|| q.to_string());
        self.find_hits.clear();
        self.find_current = 0;
        if let Some(q) = self.find.clone() {
            let hay = self.text.to_lowercase();
            let needle = q.to_lowercase();
            let mut at = 0;
            while let Some(i) = hay[at..].find(&needle) {
                let s = at + i;
                self.find_hits.push(s..s + needle.len());
                at = s + needle.len().max(1);
            }
        }
        if let Some(first) = self.find_hits.first().cloned() {
            self.caret = first.end;
            self.anchor = first.start;
        }
        cx.notify();
    }

    /// Next (`+1`) or previous (`-1`) hit, wrapping.
    pub fn find_step(&mut self, dir: isize, cx: &mut Context<Self>) {
        if self.find_hits.is_empty() {
            return;
        }
        let n = self.find_hits.len() as isize;
        self.find_current = (self.find_current as isize + dir).rem_euclid(n) as usize;
        if let Some(r) = self.find_hits.get(self.find_current) {
            self.caret = r.end;
            self.anchor = r.start;
        }
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }

    fn replace(&mut self, range: Range<usize>, s: &str, cx: &mut Context<Self>) {
        let range = self.text.floor_char_boundary(range.start)..self.text.ceil_char_boundary(range.end);
        self.record(range.clone(), s.len());
        self.text.replace_range(range.clone(), s);
        self.caret = range.start + s.len();
        self.anchor = self.caret;
        self.cache.borrow_mut().clear();
        cx.emit(SourceEvent::Changed);
        cx.notify();
    }

    /// Remember the state before an edit, coalescing consecutive typing.
    fn record(&mut self, range: Range<usize>, inserted: usize) {
        let now = std::time::Instant::now();
        let coalesce = self.history.last.is_some_and(|(t, at)| {
            at == range.start && now.duration_since(t) < std::time::Duration::from_millis(700) && inserted <= 2
        });
        if !coalesce {
            self.history.undo.push((self.text.clone(), self.caret));
            if self.history.undo.len() > 500 {
                self.history.undo.remove(0);
            }
        }
        self.history.redo.clear();
        self.history.last = Some((now, range.start + inserted));
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some((text, caret)) = self.history.undo.pop() else { return };
        self.history.redo.push((std::mem::replace(&mut self.text, text), self.caret));
        self.caret = caret.min(self.text.len());
        self.anchor = self.caret;
        self.history.last = None;
        self.cache.borrow_mut().clear();
        cx.emit(SourceEvent::Changed);
        cx.notify();
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        let Some((text, caret)) = self.history.redo.pop() else { return };
        self.history.undo.push((std::mem::replace(&mut self.text, text), self.caret));
        self.caret = caret.min(self.text.len());
        self.anchor = self.caret;
        self.history.last = None;
        self.cache.borrow_mut().clear();
        cx.emit(SourceEvent::Changed);
        cx.notify();
    }

    fn move_to(&mut self, o: usize, extend: bool, cx: &mut Context<Self>) {
        let o = self.text.ceil_char_boundary(o.min(self.text.len()));
        self.caret = o;
        if !extend {
            self.anchor = o;
        }
        self.history.last = None;
        cx.notify();
    }

    fn move_h(&mut self, forward: bool, extend: bool, cx: &mut Context<Self>) {
        let sel = self.selection();
        // A selection collapses to its edge on a plain arrow.
        if !extend && !sel.is_empty() {
            let o = if forward { sel.end } else { sel.start };
            return self.move_to(o, false, cx);
        }
        let at = self.caret;
        let o = if forward {
            self.text[at..].graphemes(true).next().map(|g| at + g.len()).unwrap_or(at)
        } else {
            self.text[..at].graphemes(true).next_back().map(|g| at - g.len()).unwrap_or(at)
        };
        self.move_to(o, extend, cx);
    }

    fn line_bounds(&self, at: usize) -> (usize, usize) {
        let start = self.text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let end = self.text[at..].find('\n').map(|i| at + i).unwrap_or(self.text.len());
        (start, end)
    }

    fn move_v(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let (start, end) = self.line_bounds(self.caret);
        if down {
            if end == self.text.len() {
                return self.move_to(end, extend, cx);
            }
            let col = self.caret - start;
            let next_start = end + 1;
            let next_end = self.text[next_start..].find('\n').map(|i| next_start + i).unwrap_or(self.text.len());
            let o = (next_start + col).min(next_end);
            let o = self.text.floor_char_boundary(o);
            self.move_to(o, extend, cx);
        } else {
            if start == 0 {
                return self.move_to(0, extend, cx);
            }
            let col = self.caret - start;
            let prev_start = self.text[..start - 1].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let o = self.text.floor_char_boundary((prev_start + col).min(start - 1));
            self.move_to(o, extend, cx);
        }
    }

    fn delete_selection_or(&mut self, back: bool, cx: &mut Context<Self>) {
        let sel = self.selection();
        if !sel.is_empty() {
            return self.replace(sel, "", cx);
        }
        let at = self.caret;
        if back {
            if at == 0 {
                return;
            }
            let from = self.text[..at].graphemes(true).next_back().map(|g| at - g.len()).unwrap_or(at);
            self.replace(from..at, "", cx);
        } else {
            if at == self.text.len() {
                return;
            }
            let to = self.text[at..].graphemes(true).next().map(|g| at + g.len()).unwrap_or(at);
            self.replace(at..to, "", cx);
        }
    }

    /// The character under a window-space point. `TextLayout::index_for`
    /// takes window coordinates: rows are laid out from the element's origin
    /// and it removes that itself.
    fn index_for(&self, p: Point<Pixels>) -> usize {
        self.cache.borrow().get(&self.id).map(|l| l.index_for(p)).unwrap_or(0).min(self.text.len())
    }

    fn on_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        let i = self.index_for(ev.position);
        let shift = ev.modifiers.shift;
        self.selecting = true;
        self.move_to(i, shift, cx);
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let i = self.index_for(ev.position);
        let i = self.text.ceil_char_boundary(i.min(self.text.len()));
        self.caret = i;
        cx.notify();
    }

    fn on_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        cx.notify();
    }
}

impl Render for SourceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let mut spec = TextSpec::body();
        spec.size = self.size;
        spec.line_height = self.line_height;
        spec.mono = self.mono;
        spec.color = Theme::text();
        spec.wrap = true;
        let sel = self.selection();
        let el = RichTextElement {
            id: self.id,
            text: RichText::plain(self.text.clone()),
            spec,
            align: Align::Left,
            placeholder: Some("空的 Markdown 文件".into()),
            placeholder_color: Some(Theme::text_placeholder()),
            placeholder_dx: 0.,
            selection: (focused && !sel.is_empty()).then_some(sel),
            caret: Some(self.caret),
            marked: None,
            cache: self.cache.clone(),
            show_caret: focused && self.selection().is_empty(),
            highlight: Some(Box::new(|t: &str| crate::highlight::spans("Markdown", t))),
            found: self.find_hits.iter().enumerate().map(|(i, r)| (r.clone(), i == self.find_current)).collect(),
            eq_counter: 0,
            button_edit: None,
        };
        let entity = cx.entity();
        let focus = self.focus.clone();
        div()
            .id(("source-view", self.id.0))
            .track_focus(&self.focus)
            .key_context("SourceView")
            .size_full()
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_action(cx.listener(|this, _: &Backspace, _, cx| this.delete_selection_or(true, cx)))
            .on_action(cx.listener(|this, _: &Delete, _, cx| this.delete_selection_or(false, cx)))
            .on_action(cx.listener(|this, _: &Left, _, cx| this.move_h(false, false, cx)))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.move_h(true, false, cx)))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.move_h(false, true, cx)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.move_h(true, true, cx)))
            .on_action(cx.listener(|this, _: &Up, _, cx| this.move_v(false, false, cx)))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.move_v(true, false, cx)))
            .on_action(cx.listener(|this, _: &Home, _, cx| {
                let (s, _) = this.line_bounds(this.caret);
                this.move_to(s, false, cx);
            }))
            .on_action(cx.listener(|this, _: &End, _, cx| {
                let (_, e) = this.line_bounds(this.caret);
                this.move_to(e, false, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.anchor = 0;
                this.caret = this.text.len();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Tab, _, cx| this.replace(this.selection(), "  ", cx)))
            .on_action(cx.listener(|this, _: &ShiftTab, _, cx| {
                let (s, _) = this.line_bounds(this.caret);
                if this.text[s..].starts_with("  ") {
                    this.replace(s..s + 2, "", cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.redo(cx)))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                let sel = this.selection();
                if !sel.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(this.text[sel].to_string()));
                }
            }))
            .on_action(cx.listener(|this, _: &Cut, _, cx| {
                let sel = this.selection();
                if !sel.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(this.text[sel.clone()].to_string()));
                    this.replace(sel, "", cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if let Some(s) = cx
                    .read_from_clipboard()
                    .and_then(|item| item.entries().iter().find_map(|e| match e {
                        ClipboardEntry::String(s) => Some(s.text().clone()),
                        _ => None,
                    }))
                {
                    this.replace(this.selection(), &s.replace("\r\n", "\n"), cx);
                }
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                // Enter is a newline in the file itself; keep it away from the
                // keymap so it can never be read as "submit".
                if ev.keystroke.key == "enter" && !ev.keystroke.modifiers.modified() {
                    this.replace(this.selection(), "\n", cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|_, _: &Save, _, cx| cx.emit(SourceEvent::Save)))
            .on_action(cx.listener(|_, _: &Escape, _, cx| cx.emit(SourceEvent::Escape)))
            .child(el)
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

impl SourceView {
    fn to16(&self, o: usize) -> usize {
        self.text[..o.min(self.text.len())].encode_utf16().count()
    }

    fn from16(&self, o16: usize) -> usize {
        let mut n = 0;
        for (i, ch) in self.text.char_indices() {
            if n >= o16 {
                return i;
            }
            n += ch.len_utf16();
        }
        self.text.len()
    }

    fn range16(&self, r: &Range<usize>) -> Range<usize> {
        self.to16(r.start)..self.to16(r.end)
    }

    fn range8(&self, r: &Range<usize>) -> Range<usize> {
        self.from16(r.start)..self.from16(r.end)
    }
}

impl EntityInputHandler for SourceView {
    fn text_for_range(&mut self, range: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let r = self.range8(&range);
        actual.replace(self.range16(&r));
        Some(self.text[r].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range16(&self.selection()), reversed: self.caret < self.anchor })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let r = range.map(|r| self.range8(&r)).unwrap_or_else(|| self.selection());
        self.replace(r, &text.replace("\r\n", "\n").replace('\r', "\n"), cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _new_selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range.map(|r| self.range8(&r)).unwrap_or_else(|| self.selection());
        self.replace(r, text, cx);
    }

    fn bounds_for_range(&mut self, range: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let r = self.range8(&range);
        let cache = self.cache.borrow();
        let l = cache.get(&self.id)?;
        let a = l.position_for(r.start);
        let b = l.position_for(r.end);
        Some(Bounds::from_corners(point(bounds.left() + a.x, bounds.top() + a.y), point(bounds.left() + b.x, bounds.top() + b.y + l.line_height)))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.to16(self.index_for(p)))
    }
}

