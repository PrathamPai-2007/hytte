//! Script panes on the UI thread: run a pane's command when it is due, only while that pane
//! is on screen (plus one round at start, so the panes have something to show). The commands
//! run on worker threads; their output comes back as `UiEvent::Script`.

use super::*;

impl Ui {
    /// Start any script pane command that is due, and arm `T_SCRIPT` for the next run of the
    /// pane on screen. Cheap to call: with no script pane showing it only stops the timer.
    pub(super) fn script_refresh(&mut self, hwnd: HWND) {
        if self.paused || self.gaming {
            return;
        }
        let now = Instant::now();
        let showing = match self.anim.scene {
            Scene::ExpScript(i) => Some(i),
            _ => None,
        };
        let due = if !self.scripts_primed {
            self.scripts_primed = true;
            self.model.scripts_due(None, now)
        } else if showing.is_some() {
            self.model.scripts_due(showing, now)
        } else {
            vec![]
        };
        for (_, name, command) in due {
            let sh = self.shared.clone();
            std::thread::spawn(move || {
                crate::proc::eco_thread();
                let run = crate::scriptpane::run(&command, crate::scriptpane::TIMEOUT);
                push_event(&sh, UiEvent::Script(name, run));
            });
        }
        let next = showing
            .and_then(|i| self.model.scripts.get(i as usize))
            .filter(|s| !s.command.is_empty())
            .map(|s| {
                let since = s
                    .ran
                    .map_or(Duration::ZERO, |r| now.saturating_duration_since(r));
                s.every.saturating_sub(since)
            });
        unsafe {
            match next {
                Some(d) => {
                    SetTimer(Some(hwnd), T_SCRIPT, d.as_millis() as u32 + 50, None);
                }
                None => {
                    let _ = KillTimer(Some(hwnd), T_SCRIPT);
                }
            }
        }
    }
}
