//! The Ask pane and the AI card on the UI thread: asking, stopping, looking at Ollama, getting a
//! model, and the actions on a clip, a shelf file or a failed command. The network work runs on
//! worker threads (`ollama.rs`); what they find comes back as `UiEvent`s.

use super::*;
use crate::ollama::Status;
use crate::ui_state::{AiJob, AiKind, AiSource, Field, REPLY_W};
use std::sync::atomic::{AtomicBool, Ordering};

impl Ui {
    /// Look at Ollama when the Ask pane is showing and nothing is known (or the last look found
    /// it missing: opening the pane again looks again). Cheap to call: usually does nothing.
    pub(super) fn ask_refresh(&mut self) {
        if !self.model.ai.enabled {
            return;
        }
        if self.anim.scene != Scene::ExpAsk {
            // A pane that was shut with Ollama missing looks again when it is next opened.
            if matches!(
                self.model.ask.status,
                Status::NotRunning | Status::NoModel | Status::Problem(_)
            ) && self.model.ask.pull.is_none()
            {
                self.model.ask.status = Status::Unknown;
            }
            // Closed: Ollama may have freed the model since; say nothing until it is loaded again.
            self.model.ask_closed();
            return;
        }
        if self.model.ask.status != Status::Unknown {
            return;
        }
        self.model.ask.status = Status::Checking;
        let (host, model, sh) = (
            self.model.ai.host.clone(),
            self.model.ai.model.clone(),
            self.shared.clone(),
        );
        std::thread::spawn(move || {
            crate::proc::eco_thread();
            let (status, size) = crate::ollama::probe(&host, &model);
            push_event(&sh, UiEvent::ModelSize(size));
            push_event(&sh, UiEvent::Ollama(status));
        });
    }

    /// Enter or the Ask button: ask what is in the field.
    pub(super) fn ask_send(&mut self) {
        if self.model.ask_edit.is_empty() || !self.fits_ram(true) {
            return;
        }
        if let Some(job) = self.model.begin_ask() {
            self.run_ai(job);
        }
    }

    /// Start `job` on a worker thread. Anything still being answered is stopped first.
    pub(super) fn run_ai(&mut self, job: AiJob) {
        self.ai_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.ai_cancel = cancel.clone();
        let sh = self.shared.clone();
        std::thread::spawn(move || {
            crate::proc::eco_thread();
            let AiJob {
                to_card,
                seq,
                host,
                question,
            } = job;
            let r = crate::ollama::ask(&host, &question, &cancel, |t| {
                push_event(&sh, UiEvent::AiText(seq, to_card, t.to_string()));
            });
            push_event(&sh, UiEvent::AiDone(seq, to_card, r));
        });
        self.layout();
        self.kick();
    }

    /// Stop the answer that is coming.
    pub(super) fn ask_stop(&mut self) {
        self.ai_cancel.store(true, Ordering::Relaxed);
        self.model.stop_ai(Instant::now());
        self.layout();
        self.kick();
    }

    /// Forget the question and answer.
    pub(super) fn ask_clear(&mut self) {
        self.ai_cancel.store(true, Ordering::Relaxed);
        self.model.clear_ask();
        self.layout();
        self.kick();
    }

    /// Close the card (and stop its answer).
    pub(super) fn ai_dismiss(&mut self) {
        self.ai_cancel.store(true, Ordering::Relaxed);
        self.model.dismiss_ai();
        self.layout();
        self.kick();
    }

    /// An action on a clip, a shelf file or a failed command: its answer comes in the card.
    pub(super) fn ai_action(&mut self, kind: AiKind, src: AiSource) {
        if !self.model.ai.enabled {
            return;
        }
        if !self.model.fits(crate::proc::free_ram(), false) {
            // The warning lives in the Ask pane; here a line says where to find it.
            self.model.ask.tight = None;
            let mut c = Chip::new(
                "Short on memory for the model. Open Ask to run it anyway.",
                Instant::now(),
            );
            c.tone = crate::ui_state::Tone::Info;
            self.model.chip = Some(c);
            return;
        }
        let job = match src {
            AiSource::Clip(i) => {
                let Some(t) = self.model.clip_text(i) else {
                    return;
                };
                self.model.begin_ai(kind, "this clip", &t)
            }
            AiSource::File(id) => {
                let Some(p) = self
                    .model
                    .shelf
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.path.clone())
                else {
                    return;
                };
                match read_text_start(&p) {
                    Some(t) => self.model.begin_ai(kind, "this file", &t),
                    None => {
                        let mut c = Chip::new("That file isn't plain text.", Instant::now());
                        c.tone = crate::ui_state::Tone::Info;
                        self.model.chip = Some(c);
                        return;
                    }
                }
            }
            AiSource::Failed(id) => {
                let Some((cmd, code, out)) = self.model.failed_text(&id) else {
                    return;
                };
                self.model.begin_ai_failed(&cmd, code, &out)
            }
        };
        // The pill folds shut with the pointer away; the card is what the user asked for, so show it.
        self.hover = true;
        self.run_ai(job);
    }

    /// The memory check before a question or a load: false (and the warning showing) when the
    /// model is bigger than the free RAM and the user hasn't said "Run anyway".
    fn fits_ram(&mut self, then_ask: bool) -> bool {
        let ok = self.model.fits(crate::proc::free_ram(), then_ask);
        if !ok {
            self.layout();
            self.kick();
        }
        ok
    }

    /// "Run anyway" on the memory warning: do what was waiting.
    pub(super) fn ask_run_anyway(&mut self) {
        match self.model.run_anyway() {
            Some(true) => self.ask_send(),
            Some(false) => self.ask_warm(),
            None => {}
        }
        self.layout();
        self.kick();
    }

    /// "Cancel" on the memory warning.
    pub(super) fn ask_cancel_tight(&mut self) {
        self.model.cancel_tight();
        self.layout();
        self.kick();
    }

    /// Have Ollama download the model.
    pub(super) fn ask_pull(&mut self) {
        if self.model.ask.pull.is_some() {
            return;
        }
        self.model.begin_pull();
        let cancel = Arc::new(AtomicBool::new(false));
        self.pull_cancel = cancel.clone();
        let (host, model, sh) = (
            self.model.ai.host.clone(),
            self.model.ai.model.clone(),
            self.shared.clone(),
        );
        std::thread::spawn(move || {
            crate::proc::eco_thread();
            let mut last = Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now);
            let r = crate::ollama::pull(&host, &model, &cancel, |p| {
                // Many lines a second: the bar needs a few.
                if last.elapsed() >= Duration::from_millis(120) {
                    last = Instant::now();
                    push_event(&sh, UiEvent::Pull(p.clone()));
                }
            });
            push_event(&sh, UiEvent::PullDone(r));
        });
        self.layout();
        self.kick();
    }

    /// Load the model now, so the first question doesn't wait for it.
    pub(super) fn ask_warm(&mut self) {
        if !self.fits_ram(false) || !self.model.begin_warm() {
            return;
        }
        let (host, model, keep, sh) = (
            self.model.ai.host.clone(),
            self.model.ai.model.clone(),
            self.model.ai.keep_alive.clone(),
            self.shared.clone(),
        );
        std::thread::spawn(move || {
            crate::proc::eco_thread();
            push_event(
                &sh,
                UiEvent::Warm(crate::ollama::warm(&host, &model, &keep)),
            );
        });
        self.layout();
        self.kick();
    }

    /// Have Ollama free the model now (its memory goes back at once).
    pub(super) fn ask_unload(&mut self) {
        if !self.model.begin_unload() {
            return;
        }
        let (host, model, sh) = (
            self.model.ai.host.clone(),
            self.model.ai.model.clone(),
            self.shared.clone(),
        );
        std::thread::spawn(move || {
            crate::proc::eco_thread();
            push_event(
                &sh,
                UiEvent::Unload(crate::ollama::warm(&host, &model, "0")),
            );
        });
        self.layout();
        self.kick();
    }

    pub(super) fn ask_pull_stop(&mut self) {
        self.pull_cancel.store(true, Ordering::Relaxed);
    }

    /// Give every answer on screen its wrapped height from DirectWrite, before the pill's size is
    /// worked out from it. Measures again only when the text changed.
    pub(super) fn measure_replies(&mut self) {
        let rend = &self.rend;
        if self.model.ask.reply.stale() {
            let r = &mut self.model.ask.reply;
            let h = rend.wrapped_h(&r.shown, REPLY_W);
            r.measured_h(h);
        }
        if let Some(c) = self.model.ai_card.as_mut() {
            if c.reply.stale() {
                let h = rend.wrapped_h(&c.reply.shown, REPLY_W);
                c.reply.measured_h(h);
            }
        }
        let ask_box = self.model.ask_box_h();
        self.model.ask.reply.settle(ask_box);
        let card_box = self.model.ai_box_h();
        if let Some(c) = self.model.ai_card.as_mut() {
            c.reply.settle(card_box);
        }
    }

    /// The Ask hotkey: open the Ask pane with the cursor ready (or close it again).
    pub(super) fn toggle_ask(&mut self, hwnd: HWND) {
        if self.model.typing == Some(Field::Ask) {
            self.end_keyboard(true);
            return;
        }
        if !self.model.panels().contains(&Panel::Ask) {
            return;
        }
        self.select_panel(Panel::Ask);
        self.hover = true;
        self.begin_typing(hwnd, Field::Ask);
        self.after_model_change(hwnd);
    }

    /// The wheel over the answer: scroll it. False when no answer is showing to scroll.
    pub(super) fn reply_wheel(&mut self, steps: i32) -> bool {
        let dy = -steps as f32 * 32.0;
        match self.anim.scene {
            Scene::ExpAsk if self.model.ask.reply.max_scroll(self.model.ask_box_h()) > 0.5 => {
                let h = self.model.ask_box_h();
                self.model.ask.reply.scroll_by(dy, h);
                true
            }
            Scene::ExpAi
                if self
                    .model
                    .ai_card
                    .as_ref()
                    .is_some_and(|c| c.reply.max_scroll(self.model.ai_box_h()) > 0.5) =>
            {
                let h = self.model.ai_box_h();
                if let Some(c) = self.model.ai_card.as_mut() {
                    c.reply.scroll_by(dy, h);
                }
                true
            }
            _ => false,
        }
    }
}

/// The first bytes of a text file (UTF-8, or UTF-16 with a mark), or None for anything else.
fn read_text_start(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut buf = vec![0u8; crate::ui_state::AI_INPUT_BYTES + 8];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    crate::transforms::text_of(&buf)
}
