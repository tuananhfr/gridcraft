//! System clipboard routing shared by desktop and web.

use serde_json::{Value as Json, json};

use crate::SheetApp;

pub(crate) enum PendingPaste {
    Special(&'static str),
    Dialog,
}

pub(crate) const EXTERNAL_CHOICES: &[(&str, &str)] = &[
    ("keepSource", "Keep Source Formatting"),
    ("matchDestination", "Match Destination Formatting"),
    ("values", "Values Only"),
];

impl SheetApp {
    /// Copy or cut the selected cells to the system clipboard. Engine-only commands keep
    /// their existing internal clipboard behavior and do not write to the host clipboard.
    pub fn copy_to_clipboard(&mut self, ctx: &egui::Context, command: &str) {
        // Only ask the engine for HTML when the host can publish it.
        let rich = self.services.set_clipboard_html.is_some() || self.services.copy_html.is_some();
        let result = match self.run_command(command, json!({"html": rich})) {
            Ok(result) => result,
            Err(e) => return self.alert(e),
        };
        let Some(text) = result.get("text").and_then(Json::as_str) else { return };
        // The system clipboard gets this copy, so its text decides what Paste pastes.
        self.clipboard_text_at_copy = None;
        self.cut_text = (command == "edit.cut").then(|| text.replace("\r\n", "\n"));
        if let Some(html) = result.get("html").and_then(Json::as_str)
            && let Some(write) = &self.services.set_clipboard_html
            && write(html, text)
        {
            return;
        }
        if let Some(html) = result.get("html").and_then(Json::as_str)
            && let Some(copy_html) = &mut self.services.copy_html
        {
            match copy_html(html, text) {
                Ok(()) => return,
                Err(error) => log::warn!("HTML clipboard unavailable; copying plain text: {error}"),
            }
        }
        ctx.copy_text(text.to_string());
    }

    /// Paste text that came from the system clipboard (Cmd/Ctrl+V, or the ribbon and menu Paste),
    /// with the clipboard's HTML flavour when the host can read it.
    ///
    /// Excel pastes a cut once: after GridCraft's cut has been pasted, the system clipboard text
    /// that came with it is not pasted again over the moved block; GridCraft's own (empty)
    /// clipboard is pasted instead, which reports "Nothing to paste.". Other text means the
    /// clipboard has moved on, so the cut's text is forgotten and pastes normally later.
    pub fn paste_external(&mut self, text: String) {
        if let Some(pending) = self.pending_paste.take() {
            let params = self.clipboard_special_params(Some(text));
            return self.finish_clipboard_special(pending, params);
        }
        if Self::same_clipboard_text(Some(&text), self.cut_text.as_deref()) {
            if self.session.clipboard.is_none() {
                return self.run_or_alert("edit.paste", json!({}));
            }
        } else {
            self.cut_text = None;
        }
        let html = self.services.paste_html.as_mut().and_then(|read| read());
        let params = match html {
            Some(html) => json!({"text": text, "html": html}),
            None => json!({"text": text}),
        };
        self.run_or_alert("edit.paste", params);
    }

    /// Ribbon and menu Paste: paste the text on the system clipboard, as Cmd/Ctrl+V does. With no
    /// text there, no host clipboard (web), or the text that was there when GridCraft last copied
    /// without publishing, GridCraft's own clipboard is pasted.
    pub fn paste_from_clipboard(&mut self) {
        self.pending_paste = None;
        if let Some(request) = &self.services.request_clipboard {
            request();
            return;
        }
        match self.system_clipboard_text() {
            Some(text) if self.session.clipboard.is_none() || self.clipboard_text_at_copy.as_ref() != Some(&text) => self.paste_external(text),
            _ => self.run_or_alert("edit.paste", json!({})),
        }
    }

    /// The system clipboard's plain text, with the line endings of egui's `Event::Paste` text.
    /// `None` when it holds no text or the host can't read it.
    pub(crate) fn system_clipboard_text(&mut self) -> Option<String> {
        self.services.paste_text.as_mut().and_then(|read| read()).filter(|t| !t.is_empty()).map(|t| t.replace("\r\n", "\n"))
    }

    /// Whether two clipboard texts are the same copy: trailing line breaks don't count, as in the
    /// engine's `edit.paste`, and CRLF matches LF (egui's `Event::Paste` text may keep the host's).
    pub(crate) fn same_clipboard_text(a: Option<&str>, b: Option<&str>) -> bool {
        fn chars(t: &str) -> impl Iterator<Item = char> + '_ {
            t.trim_end_matches(['\r', '\n']).chars().filter(|c| *c != '\r')
        }
        match (a, b) {
            (Some(a), Some(b)) => chars(a).eq(chars(b)),
            _ => false,
        }
    }

    pub fn paste_special_from_clipboard(&mut self, what: &'static str) {
        self.request_clipboard_special(PendingPaste::Special(what));
    }

    pub(crate) fn open_clipboard_special(&mut self) {
        self.request_clipboard_special(PendingPaste::Dialog);
    }

    fn request_clipboard_special(&mut self, pending: PendingPaste) {
        if let Some(request) = &self.services.request_clipboard {
            self.pending_paste = Some(pending);
            request();
            return;
        }
        let text = self.system_clipboard_text();
        let params = self.clipboard_special_params(text);
        self.finish_clipboard_special(pending, params);
    }

    fn clipboard_special_params(&mut self, text: Option<String>) -> Json {
        if Self::same_clipboard_text(text.as_deref(), self.cut_text.as_deref()) && self.session.clipboard.is_none() {
            return json!({"source": "internal"});
        }
        let html = self.services.paste_html.as_mut().and_then(|read| read());
        let internal = self.session.clipboard.as_ref().is_some_and(|clip| {
            (text.is_none() && html.is_none())
                || Self::same_clipboard_text(text.as_deref(), Some(&clip.text))
                || text.as_ref().is_some_and(|text| self.clipboard_text_at_copy.as_ref() == Some(text))
        });
        if internal {
            return json!({"source": "internal"});
        }
        let mut params = json!({"source": "external"});
        if let Some(text) = text {
            params["text"] = json!(text);
        }
        if let Some(html) = html {
            params["html"] = json!(html);
        }
        params
    }

    fn finish_clipboard_special(&mut self, pending: PendingPaste, mut params: Json) {
        match pending {
            PendingPaste::Dialog => self.dialog = crate::dialogs::Dialog::open(self, "pasteSpecial", params),
            PendingPaste::Special(what) => {
                let internal = params["source"] == "internal";
                params["what"] = json!(if internal && what == "keepSource" { "all" } else if internal { "values" } else { what });
                self.run_or_alert("edit.pasteSpecial", params);
            }
        }
    }
}
