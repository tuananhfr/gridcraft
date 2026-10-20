//! Browser clipboard transport; clipboard markup never enters the page DOM.

const MAX_RICH_BYTES: usize = gridcraft_engine::clipboard_limits::MAX_HTML_BYTES;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_IMAGES: usize = 20;

#[derive(Default)]
pub struct PendingPaste {
    pub text: String,
    pub html: Option<String>,
    pub rtf: Option<String>,
    pub images: Vec<(String, Vec<u8>)>,
}

impl PendingPaste {
    fn bounded(mut self) -> (Self, bool) {
        let mut dropped = false;
        if self.html.as_ref().is_some_and(|s| !gridcraft_engine::clipboard_limits::accepts_html(s)) {
            self.html = None;
            dropped = true;
        }
        if self.rtf.as_ref().is_some_and(|s| s.len() > MAX_RICH_BYTES) {
            self.rtf = None;
            dropped = true;
        }
        let count = self.images.len();
        self.images.retain(|(mime, bytes)| mime.starts_with("image/") && bytes.len() <= MAX_IMAGE_BYTES);
        self.images.truncate(MAX_IMAGES);
        dropped |= count != self.images.len();
        if self.text.is_empty()
            && let Some(html) = &self.html
        {
            self.text = html_to_plain(html);
        }
        (self, dropped)
    }
}

fn paste_events(pending: &PendingPaste) -> Vec<egui::Event> {
    if pending.text.is_empty() { Vec::new() } else { vec![egui::Event::Paste(pending.text.clone())] }
}

#[derive(Default)]
struct PasteFrame {
    current: Option<PendingPaste>,
}

impl PasteFrame {
    fn begin(&mut self, pending: Option<PendingPaste>, raw: &mut egui::RawInput) {
        self.current = pending;
        if let Some(pending) = &self.current {
            raw.events.extend(paste_events(pending));
        }
    }
}

fn clipboard_key(event: &egui::Event) -> bool {
    matches!(event, egui::Event::Key { key: egui::Key::C | egui::Key::X | egui::Key::V, modifiers, .. }
        if (modifiers.ctrl || modifiers.command) && !modifiers.alt && !modifiers.shift)
}

fn html_to_plain(html: &str) -> String {
    if html.len() > MAX_RICH_BYTES {
        return String::new();
    }
    let mut out = String::new();
    let mut rest = html;
    let mut hidden: Option<String> = None;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").and_then(|i| after.get(i + 3..)).unwrap_or_default();
            continue;
        }
        if rest.starts_with('<') {
            let mut quote = None;
            let end = rest.char_indices().find_map(|(i, ch)| {
                match (quote, ch) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, '\'' | '"') => quote = Some(ch),
                    (None, '>') => return Some(i),
                    _ => {}
                }
                None
            });
            let Some(end) = end else { break };
            let tag = rest.get(1..end).unwrap_or_default().trim();
            let closing = tag.starts_with('/');
            let name = tag.trim_start_matches('/').split([' ', '\t', '\r', '\n', '/']).next().unwrap_or_default().to_ascii_lowercase();
            rest = rest.get(end + 1..).unwrap_or_default();
            if let Some(raw) = &hidden {
                if closing && *raw == name {
                    hidden = None;
                }
                continue;
            }
            match (name.as_str(), closing) {
                ("script" | "style", false) => hidden = Some(name),
                ("td" | "th", true) => out.push('\t'),
                ("tr", true) => {
                    while out.ends_with('\t') {
                        out.pop();
                    }
                    out.push('\n');
                }
                ("br", _) | ("p" | "div", true) => out.push('\n'),
                _ => {}
            }
        } else if let Some(ch) = rest.chars().next() {
            if hidden.is_none() {
                if ch == '&'
                    && let Some(end) = rest.char_indices().take(33).find_map(|(i, ch)| (ch == ';').then_some(i))
                    && let Some(entity) = rest.get(1..end).and_then(decode_entity)
                {
                    out.push(entity);
                    rest = rest.get(end + 1..).unwrap_or_default();
                    continue;
                }
                out.push(ch);
            }
            rest = rest.get(ch.len_utf8()..).unwrap_or_default();
        }
    }
    out.trim_end_matches(['\t', '\r', '\n']).to_string()
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let number = if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                entity.strip_prefix('#')?.parse().ok()?
            };
            char::from_u32(number)
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::ClipboardBridge;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::RefCell;
    use std::rc::Rc;

    use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    use super::{MAX_IMAGE_BYTES, MAX_IMAGES, MAX_RICH_BYTES, PasteFrame, PendingPaste, clipboard_key};

    const READ_ERROR: &str = "The browser cannot read the clipboard — use Ctrl+V";
    const WRITE_ERROR: &str = "Could not write to the clipboard";
    const SIZE_NOTICE: &str = "Some clipboard content was too large and was skipped.";

    #[derive(Clone)]
    pub struct ClipboardBridge {
        inbox: Rc<RefCell<Option<PendingPaste>>>,
        frame: Rc<RefCell<PasteFrame>>,
        notice: Rc<RefCell<Option<&'static str>>>,
        ctx: egui::Context,
    }

    impl ClipboardBridge {
        pub fn install(ctx: egui::Context) -> Result<Self, String> {
            let bridge = Self {
                inbox: Rc::new(RefCell::new(None)),
                frame: Rc::new(RefCell::new(PasteFrame::default())),
                notice: Rc::new(RefCell::new(None)),
                ctx,
            };
            let listener_bridge = bridge.clone();
            let listener = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |event: web_sys::ClipboardEvent| {
                if html_editor_focused() {
                    return;
                }
                let Some(data) = event.clipboard_data() else { return };
                let mut pending = PendingPaste {
                    text: data.get_data("text/plain").unwrap_or_default(),
                    html: nonempty(data.get_data("text/html").ok()),
                    rtf: nonempty(data.get_data("text/rtf").ok()),
                    images: Vec::new(),
                };
                // DataTransfer expires after this handler; keep File handles before awaiting.
                let mut files = Vec::new();
                if let Some(list) = data.files() {
                    for i in 0..list.length() {
                        let Some(file) = list.item(i) else { continue };
                        if file.type_().starts_with("image/") {
                            if files.len() < MAX_IMAGES && file.size() <= MAX_IMAGE_BYTES as f64 {
                                files.push(file);
                            } else {
                                listener_bridge.report(SIZE_NOTICE);
                            }
                        }
                    }
                }
                event.prevent_default();
                event.stop_immediate_propagation();
                let (bounded, dropped) = pending.bounded();
                pending = bounded;
                if dropped {
                    listener_bridge.report(SIZE_NOTICE);
                }
                let bridge = listener_bridge.clone();
                if files.is_empty() {
                    bridge.deliver(pending);
                } else {
                    spawn_local(async move {
                        for file in files {
                            match blob_bytes(&file).await {
                                Ok(bytes) => pending.images.push((file.type_(), bytes)),
                                Err(()) => bridge.report(READ_ERROR),
                            }
                        }
                        bridge.deliver(pending);
                    });
                }
            });
            let window = web_sys::window().ok_or("no window")?;
            window.add_event_listener_with_callback_and_bool("paste", listener.as_ref().unchecked_ref(), true).map_err(|_| READ_ERROR)?;
            listener.forget();
            Ok(bridge)
        }

        pub fn connect(&self, services: &mut gridcraft_ui_egui::Services) {
            let bridge = self.clone();
            services.paste_html = Some(Box::new(move || bridge.peek_html()));
            let bridge = self.clone();
            services.paste_text = Some(Box::new(move || bridge.frame.borrow().current.as_ref().map(|p| p.text.clone())));
            let bridge = self.clone();
            services.set_clipboard_html = Some(Box::new(move |html, text| bridge.write(html, text)));
            let bridge = self.clone();
            services.request_clipboard = Some(Box::new(move || bridge.request_read()));
        }

        pub fn take(&self) -> Option<PendingPaste> {
            self.inbox.borrow_mut().take()
        }

        pub fn peek_html(&self) -> Option<String> {
            self.frame.borrow().current.as_ref().and_then(|p| p.html.clone())
        }

        pub fn raw_input(&self, raw: &mut egui::RawInput, app: &mut gridcraft_ui_egui::SheetApp) {
            raw.events.retain(|event| !clipboard_key(event));
            let pending = self.take();
            self.frame.borrow_mut().begin(pending, raw);
            let images =
                self.frame.borrow_mut().current.as_mut().filter(|p| p.text.is_empty()).map(|p| std::mem::take(&mut p.images)).unwrap_or_default();
            for (mime, bytes) in images {
                app.run_or_alert("insert.picture", serde_json::json!({"mime": mime, "base64": gridcraft_engine::io::base64_encode(&bytes)}));
            }
        }

        pub fn show_notice(&self, app: &mut gridcraft_ui_egui::SheetApp) {
            if let Some(message) = self.notice.borrow_mut().take() {
                app.toast = Some((message.into(), gridcraft_ui_egui::now_ms()));
            }
        }

        fn report(&self, message: &'static str) {
            *self.notice.borrow_mut() = Some(message);
            self.ctx.request_repaint();
        }

        fn deliver(&self, pending: PendingPaste) {
            let (pending, dropped) = pending.bounded();
            if dropped {
                self.report(SIZE_NOTICE);
            }
            *self.inbox.borrow_mut() = Some(pending);
            self.ctx.request_repaint();
        }

        pub fn write(&self, html: &str, text: &str) -> bool {
            let started = (|| {
                if html.len() > MAX_RICH_BYTES {
                    return None;
                }
                let window = web_sys::window()?;
                if !js_sys::Reflect::get(&window, &"ClipboardItem".into()).ok()?.is_function() {
                    return None;
                }
                let clipboard = window.navigator().clipboard();
                let data = js_sys::Object::new();
                for (mime, value) in [("text/html", html), ("text/plain", text)] {
                    let opts = web_sys::BlobPropertyBag::new();
                    opts.set_type(mime);
                    let parts = js_sys::Array::of1(&JsValue::from_str(value));
                    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts).ok()?;
                    js_sys::Reflect::set(&data, &mime.into(), &blob).ok()?;
                }
                let item = web_sys::ClipboardItem::new_with_record_from_str_to_blob_promise(&data).ok()?;
                call_promise(&clipboard, "write", Some(&js_sys::Array::of1(&item))).ok()
            })();
            let Some(promise) = started else { return false };
            let bridge = self.clone();
            spawn_local(async move {
                if JsFuture::from(promise).await.is_err() {
                    bridge.report(WRITE_ERROR);
                }
            });
            true
        }

        pub fn request_read(&self) {
            let promise = web_sys::window().and_then(|w| call_promise(&w.navigator().clipboard(), "read", None).ok());
            let Some(promise) = promise else {
                self.report(READ_ERROR);
                return;
            };
            let bridge = self.clone();
            spawn_local(async move {
                match read_items(promise, &bridge).await {
                    Ok(pending) => bridge.deliver(pending),
                    Err(()) => bridge.report(READ_ERROR),
                }
            });
        }
    }

    fn html_editor_focused() -> bool {
        let Some(active) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.active_element()) else { return false };
        if active.id().starts_with("egui_text_agent") {
            return false;
        }
        // eframe 0.36 gives its IME input no id and inserts it immediately after the canvas.
        if active.tag_name() == "INPUT"
            && active.get_attribute("autocapitalize").as_deref() == Some("off")
            && web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.get_element_by_id("gridcraft_canvas"))
                .and_then(|canvas| canvas.next_element_sibling())
                .is_some_and(|agent| agent == active)
        {
            return false;
        }
        matches!(active.tag_name().as_str(), "INPUT" | "TEXTAREA")
            || active.dyn_ref::<web_sys::HtmlElement>().is_some_and(|e| e.is_content_editable())
    }

    fn nonempty(text: Option<String>) -> Option<String> {
        text.filter(|s| !s.is_empty())
    }

    fn call_promise(target: &JsValue, name: &str, argument: Option<&JsValue>) -> Result<js_sys::Promise, ()> {
        let function = js_sys::Reflect::get(target, &name.into()).map_err(|_| ())?.dyn_into::<js_sys::Function>().map_err(|_| ())?;
        let result = match argument {
            Some(argument) => function.call1(target, argument),
            None => function.call0(target),
        };
        result.map_err(|_| ())?.dyn_into().map_err(|_| ())
    }

    async fn blob_bytes(blob: &web_sys::Blob) -> Result<Vec<u8>, ()> {
        if blob.size() > MAX_IMAGE_BYTES as f64 {
            return Err(());
        }
        let buffer = JsFuture::from(blob.array_buffer()).await.map_err(|_| ())?;
        let bytes = js_sys::Uint8Array::new(&buffer);
        if bytes.length() as usize > MAX_IMAGE_BYTES {
            return Err(());
        }
        Ok(bytes.to_vec())
    }

    async fn read_items(promise: js_sys::Promise, bridge: &ClipboardBridge) -> Result<PendingPaste, ()> {
        let value = JsFuture::from(promise).await.map_err(|_| ())?;
        let items = js_sys::Array::from(&value);
        let mut pending = PendingPaste::default();
        for value in items.iter() {
            let item = value.dyn_into::<web_sys::ClipboardItem>().map_err(|_| ())?;
            for mime in item.types().iter().filter_map(|t| t.as_string()) {
                let image = mime.starts_with("image/");
                if !image && !matches!(mime.as_str(), "text/plain" | "text/html" | "text/rtf") {
                    continue;
                }
                if image && pending.images.len() >= MAX_IMAGES {
                    bridge.report(SIZE_NOTICE);
                    continue;
                }
                let blob = JsFuture::from(item.get_type(&mime)).await.map_err(|_| ())?.dyn_into::<web_sys::Blob>().map_err(|_| ())?;
                let cap = if image {
                    MAX_IMAGE_BYTES
                } else if mime == "text/plain" {
                    usize::MAX
                } else {
                    MAX_RICH_BYTES
                };
                if blob.size() > cap as f64 {
                    bridge.report(SIZE_NOTICE);
                    continue;
                }
                if image {
                    pending.images.push((mime, blob_bytes(&blob).await?));
                } else {
                    let text = JsFuture::from(blob.text()).await.map_err(|_| ())?.as_string().ok_or(())?;
                    match mime.as_str() {
                        "text/plain" => pending.text = text,
                        "text/html" => pending.html = nonempty(Some(text)),
                        "text/rtf" => pending.rtf = nonempty(Some(text)),
                        _ => {}
                    }
                }
            }
        }
        Ok(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_only_paste_decodes_entities_and_table_boundaries() {
        let (paste, dropped) = PendingPaste {
            html: Some("<table><tr><td>A&amp;B</td><td>&#x1EA1;&#33;&nbsp;&lt;</td></tr><tr><td>2</td><td>3</td></tr></table>".into()),
            ..Default::default()
        }
        .bounded();
        assert!(!dropped);
        assert_eq!(paste.text, "A&B\tạ! <\n2\t3");
        assert!(matches!(paste_events(&paste).as_slice(), [egui::Event::Paste(text)] if text == &paste.text));
    }

    #[test]
    fn html_to_plain_ignores_markup_comments_and_raw_text() {
        assert_eq!(html_to_plain("<!--x--><style>bad</style><script>bad()</script><p title='>'>safe<br>text &unknown;</p>"), "safe\ntext &unknown;");
        assert_eq!(html_to_plain("<div>Tiếng Việt &#7879; &#xD800;</div>"), "Tiếng Việt ệ &#xD800;");
    }

    #[test]
    fn caps_drop_rich_parts_and_keep_plain_text() {
        let (paste, dropped) = PendingPaste {
            text: "fallback".into(),
            html: Some("x".repeat(MAX_RICH_BYTES + 1)),
            rtf: Some("x".repeat(MAX_RICH_BYTES + 1)),
            images: vec![("image/png".into(), vec![0; MAX_IMAGE_BYTES + 1])],
        }
        .bounded();
        assert!(dropped);
        assert_eq!(paste.text, "fallback");
        assert!(paste.html.is_none() && paste.rtf.is_none() && paste.images.is_empty());
        let (paste, dropped) = PendingPaste { images: vec![("image/png".into(), vec![]); MAX_IMAGES + 1], ..Default::default() }.bounded();
        assert!(dropped);
        assert_eq!(paste.images.len(), MAX_IMAGES);
    }

    #[test]
    fn rich_caps_are_inclusive() {
        let (paste, dropped) =
            PendingPaste { text: "text".into(), html: Some("x".repeat(MAX_RICH_BYTES)), rtf: Some("rtf".into()), ..Default::default() }.bounded();
        assert!(!dropped);
        assert_eq!(paste.html.as_ref().map(String::len), Some(MAX_RICH_BYTES));
        assert_eq!(paste.rtf.as_deref(), Some("rtf"));
    }

    #[test]
    fn table_allocation_limits_drop_html_and_keep_text() {
        for html in ["<table><tr><td colspan='16384'>x</td></tr><tr><td colspan='16384'>y</td></tr></table>".into(), "<div>".repeat(65)] {
            let (paste, dropped) = PendingPaste { text: "fallback".into(), html: Some(html), ..Default::default() }.bounded();
            assert!(dropped);
            assert!(paste.html.is_none());
            assert_eq!(paste.text, "fallback");
        }
    }

    #[test]
    fn image_only_paste_has_no_text_event() {
        let paste = PendingPaste { images: vec![("image/png".into(), vec![1])], ..Default::default() };
        assert!(paste_events(&paste).is_empty());
    }

    #[test]
    fn current_rich_parts_expire_on_the_next_frame() {
        let mut frame = PasteFrame::default();
        let mut raw = egui::RawInput::default();
        let paste = PendingPaste { text: "one".into(), html: Some("<b>one</b>".into()), ..Default::default() };
        frame.begin(Some(paste), &mut raw);
        assert_eq!(raw.events.len(), 1);
        assert_eq!(frame.current.as_ref().and_then(|p| p.html.as_deref()), Some("<b>one</b>"));
        raw.events.clear();
        frame.begin(None, &mut raw);
        assert!(frame.current.is_none());
        assert!(raw.events.is_empty());
    }

    #[test]
    fn browser_clipboard_keys_do_not_run_twice_or_change_other_shortcuts() {
        for key in [egui::Key::C, egui::Key::X, egui::Key::V] {
            let event = |modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            assert!(clipboard_key(&event(egui::Modifiers::CTRL)));
            assert!(clipboard_key(&event(egui::Modifiers::COMMAND)));
            assert!(!clipboard_key(&event(egui::Modifiers::CTRL | egui::Modifiers::SHIFT)));
            assert!(!clipboard_key(&event(egui::Modifiers::NONE)));
        }
        assert!(!clipboard_key(&egui::Event::Copy));
        assert!(!clipboard_key(&egui::Event::Cut));
    }
}
