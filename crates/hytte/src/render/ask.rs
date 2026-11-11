//! The Ask pane (one question for your own Ollama, answered in a box that scrolls) and the card an
//! Explain / Summarise / Shorten answer shows in.

use super::*;
use crate::ollama::Status;
use crate::ui_state::Warm;
use crate::ui_state::{
    AiCard, Field, Reply, ASK_FIELD_H, ASK_FOOT_H, ASK_NOTICE_H, REPLY_PAD, REPLY_W,
};

/// What a model's download weighs, for the models Hytte suggests.
fn size_of(model: &str) -> Option<&'static str> {
    match model {
        "qwen3:1.7b" => Some("1.4 GB"),
        "qwen3:0.6b" => Some("0.5 GB"),
        "gemma3:1b" => Some("0.8 GB"),
        _ => None,
    }
}

impl Renderer {
    /// Height of `text` word-wrapped to `w` px in the answer font.
    pub fn wrapped_h(&self, text: &str, w: f32) -> f32 {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let Ok(l) = (unsafe { self.dw.CreateTextLayout(&wide, &self.f.wrap, w, 10_000.0) }) else {
            // No DirectWrite: a line per 52 characters, 16 px a line.
            return (text.chars().count() as f32 / 52.0).ceil().max(1.0) * 16.0;
        };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = l.GetMetrics(&mut m);
        }
        m.height
    }

    /// Three dots that pulse one after another: something is on its way.
    fn dots(&self, x: f32, cy: f32) {
        let t = self.t.get();
        for i in 0..3 {
            let k = ((t * 4.0 - i as f32 * 0.7).sin() * 0.5 + 0.5).powi(2);
            self.circle(
                x + i as f32 * 9.0,
                cy,
                2.6,
                self.cc(self.fg(), 0.25 + 0.6 * k),
            );
        }
    }

    /// `r`'s text in a box at (x, y) of height `h`, scrolled; a thin bar shows past the end.
    fn reply_box(&self, r: &Reply, x: f32, y: f32, h: f32) {
        unsafe {
            self.rt.PushAxisAlignedClip(
                &rect(x, y, REPLY_W + 6.0, h),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
        }
        self.text(
            &r.shown,
            &self.f.wrap,
            x,
            y - r.scroll,
            REPLY_W,
            r.text_h + 24.0,
            self.cc(self.fg(), 0.95),
        );
        unsafe { self.rt.PopAxisAlignedClip() };
        let max = r.max_scroll(h);
        if max > 0.5 && r.text_h > 1.0 {
            let (bx, bh) = (x + REPLY_W + 2.0, (h * h / r.text_h).clamp(14.0, h));
            let by = y + (h - bh) * (r.scroll / max);
            self.fill_rr(bx, by, 3.0, bh, 1.5, self.cc(self.fg(), 0.3));
        }
    }

    pub(super) fn exp_ask(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        let fg = self.fg();
        let accent = self.accent.get();
        let ready = m.ask.status == Status::Ready;
        let r = &m.ask.reply;
        // The field, with Ask / Stop at its right.
        self.text_field(
            fr,
            Field::Ask,
            (12.0, 12.0, w - 24.0, ASK_FIELD_H),
            "Ask something",
            66.0,
        );
        let (bx, by) = (w - 12.0 - 60.0, 12.0 + 5.0);
        if r.busy {
            self.button("Stop", bx, by, 54.0, 24.0, accent, Action::AskStop);
        } else if ready && !m.ask_edit.is_empty() {
            self.button("Ask", bx, by, 54.0, 24.0, accent, Action::AskSend);
        } else {
            self.text("Ask", &self.f.btn, bx, by, 54.0, 24.0, self.cc(GRAY, 0.6));
        }
        let top = 12.0 + ASK_FIELD_H + 8.0;
        let box_h = m.ask_box_h();
        let tx = REPLY_PAD;
        if !ready {
            self.ask_notice(fr, w, top);
        } else if r.shown.is_empty() && !r.busy && r.error.is_none() {
            let (line, sub): (String, Option<String>) = match &m.ask.warm {
                Warm::Loading => (format!("Loading {}…", m.ask_model()), None),
                Warm::Loaded => (
                    format!("{} is loaded.", m.ask_model()),
                    Some(m.warm_stays()),
                ),
                Warm::Unloading => (format!("Freeing {}…", m.ask_model()), None),
                Warm::Failed(e) => ("Couldn't change that.".to_string(), Some(e.clone())),
                Warm::Idle => (format!("{} answers here, on this PC.", m.ask_model()), None),
            };
            self.text(
                &line,
                &self.f.body,
                tx,
                top,
                w - 32.0,
                20.0,
                self.cc(GRAY, 0.85),
            );
            if let Some(s) = sub {
                self.text(
                    &s,
                    &self.f.small,
                    tx,
                    top + 22.0,
                    w - 32.0,
                    16.0,
                    self.cc(GRAY, 0.7),
                );
            }
            if m.ask.warm == Warm::Loaded {
                self.button(
                    "Unload now",
                    w - 12.0 - 120.0,
                    top - 3.0,
                    120.0,
                    24.0,
                    self.fg(),
                    Action::AskUnload,
                );
            }
            if matches!(m.ask.warm, Warm::Idle | Warm::Failed(_)) {
                self.button(
                    "Load model now",
                    w - 12.0 - 120.0,
                    top - 3.0,
                    120.0,
                    24.0,
                    accent,
                    Action::AskWarm,
                );
            }
        } else if r.shown.is_empty() && r.busy {
            self.dots(tx + 4.0, top + 14.0);
        } else {
            self.reply_box(r, tx, top, box_h);
        }
        // Under the answer: the model, or what went wrong; Copy and Clear.
        let fy = top + box_h + 6.0;
        let note = match (&r.error, ready) {
            (Some(e), _) => (e.as_str().to_string(), AMBER),
            (None, true) => (format!("{} · local, can be wrong", m.ask_model()), GRAY),
            (None, false) => ("Runs on your own Ollama".to_string(), GRAY),
        };
        let has_text = !r.shown.is_empty();
        let right = if has_text || !m.ask_edit.is_empty() {
            120.0
        } else {
            0.0
        };
        self.text(
            &note.0,
            &self.f.small,
            tx,
            fy,
            w - 32.0 - right,
            ASK_FOOT_H,
            self.cc(note.1, 0.9),
        );
        if has_text {
            self.button(
                "Copy",
                w - 12.0 - 54.0 - 6.0 - 54.0,
                fy,
                54.0,
                ASK_FOOT_H,
                accent,
                Action::CopyText(r.shown.clone()),
            );
        }
        if has_text || !m.ask_edit.is_empty() {
            self.button(
                "Clear",
                w - 12.0 - 54.0,
                fy,
                54.0,
                ASK_FOOT_H,
                fg,
                Action::AskClear,
            );
        }
    }

    /// What the pane says while Ollama can't answer yet, with the one thing to do about it.
    fn ask_notice(&self, fr: &Frame, w: f32, top: f32) {
        let m = fr.model;
        let accent = self.accent.get();
        let say = |a: &str, b: &str| {
            self.text(
                a,
                &self.f.title,
                REPLY_PAD,
                top,
                w - 32.0,
                20.0,
                self.cc(self.fg(), 0.97),
            );
            self.text(
                b,
                &self.f.small,
                REPLY_PAD,
                top + 20.0,
                w - 32.0,
                16.0,
                self.cc(GRAY, 1.0),
            );
        };
        let button_y = top + 42.0;
        match &m.ask.status {
            Status::Unknown | Status::Checking => {
                say(
                    "Looking for Ollama…",
                    "On this PC, at the address in [ai] host.",
                );
            }
            Status::NotRunning => {
                say(
                    "Ollama isn't running.",
                    "Start it, or get it free at ollama.com.",
                );
                self.button(
                    "Try again",
                    REPLY_PAD,
                    button_y,
                    84.0,
                    24.0,
                    accent,
                    Action::AskRetry,
                );
            }
            Status::Problem(e) => {
                say("Can't use Ollama.", e);
                self.button(
                    "Try again",
                    REPLY_PAD,
                    button_y,
                    84.0,
                    24.0,
                    accent,
                    Action::AskRetry,
                );
            }
            Status::NoModel => {
                if let Some(b) = &m.ask.pull {
                    let pct = b
                        .fraction()
                        .map_or(String::new(), |f| format!("  {}%", (f * 100.0) as u32));
                    say(
                        &format!("Downloading {}{pct}", m.ask_model()),
                        "Ollama is fetching it. This can take a few minutes.",
                    );
                    let bw = w - 32.0 - 70.0 - 8.0;
                    let by = button_y + 10.0;
                    self.fill_rr(REPLY_PAD, by, bw, 4.0, 2.0, self.cc(self.fg(), 0.12));
                    let f = b.fraction().unwrap_or(0.0);
                    if f > 0.0 {
                        self.fill_rr(
                            REPLY_PAD,
                            by,
                            (bw * f).max(4.0),
                            4.0,
                            2.0,
                            self.cc(accent, 0.95),
                        );
                    } else {
                        // Not started: a little light runs along the bar.
                        let k = (self.t.get() * 0.8).fract();
                        self.fill_rr(
                            REPLY_PAD + (bw - 40.0) * k,
                            by,
                            40.0,
                            4.0,
                            2.0,
                            self.cc(accent, 0.6),
                        );
                    }
                    self.button(
                        "Stop",
                        REPLY_PAD + bw + 8.0,
                        button_y,
                        62.0,
                        24.0,
                        self.fg(),
                        Action::AskPullStop,
                    );
                } else {
                    let sub = match &m.ask.pull_err {
                        Some(e) => e.clone(),
                        None => "Ollama is running but doesn't have it yet.".to_string(),
                    };
                    say(&format!("{} isn't downloaded.", m.ask_model()), &sub);
                    let label = match size_of(m.ask_model()) {
                        Some(s) => format!("Get it ({s})"),
                        None => "Get it".to_string(),
                    };
                    self.button(
                        &label,
                        REPLY_PAD,
                        button_y,
                        110.0,
                        24.0,
                        accent,
                        Action::AskPull,
                    );
                }
            }
            Status::Ready => {}
        }
        let _ = ASK_NOTICE_H;
    }

    pub(super) fn exp_ai(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        let Some(c) = m.ai_card.as_ref() else { return };
        self.ai_card(fr, c, w);
    }

    fn ai_card(&self, fr: &Frame, c: &AiCard, w: f32) {
        let m = fr.model;
        let accent = self.accent.get();
        let fg = self.fg();
        let r = &c.reply;
        self.circle(26.0, 24.0, 9.0, self.cc(BLUE, 0.2));
        self.glyph('\u{E8BD}', 26.0, 24.0, 14.0, self.cc(BLUE, 1.0));
        let (title, col) = match &r.error {
            Some(e) => (e.as_str(), AMBER),
            None => (c.title.as_str(), fg),
        };
        self.text(
            title,
            &self.f.title,
            46.0,
            14.0,
            w - 46.0 - 20.0,
            20.0,
            self.cc(col, 0.97),
        );
        let box_h = m.ai_box_h();
        let by = 44.0;
        if r.shown.is_empty() && r.busy {
            self.dots(REPLY_PAD + 4.0, by + 10.0);
        } else {
            self.reply_box(r, REPLY_PAD, by, box_h);
        }
        let y = by + box_h + 10.0;
        if r.busy {
            self.button("Stop", 16.0, y, 62.0, 24.0, accent, Action::AiDismiss);
        } else {
            if !r.shown.is_empty() {
                self.button(
                    "Copy",
                    16.0,
                    y,
                    62.0,
                    24.0,
                    accent,
                    Action::CopyText(r.shown.clone()),
                );
                self.button("Dismiss", 86.0, y, 70.0, 24.0, fg, Action::AiDismiss);
            } else {
                self.button("Dismiss", 16.0, y, 70.0, 24.0, fg, Action::AiDismiss);
            }
        }
        if !r.busy {
            self.text(
                "Local · can be wrong",
                &self.f.small,
                w - 16.0 - 130.0,
                y,
                130.0,
                24.0,
                self.cc(GRAY, 0.8),
            );
        }
    }
}
