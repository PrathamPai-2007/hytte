//! The shelf side of the window: staged items, screenshots, thumbnails.

use super::*;

impl Ui {
    /// Put staged items on the shelf and open it to show them.
    pub(super) fn add_staged(
        &mut self,
        staged: Vec<crate::shelf::ShelfItem>,
        now: Instant,
    ) -> usize {
        let n = crate::shelf::insert(&self.cfg.shelf, &mut self.model.shelf, staged);
        if n > 0 {
            self.save_shelf();
            self.model.peek(Panel::Shelf, now + Duration::from_secs(5));
            self.model.force_until = Some(now + Duration::from_secs(5));
            self.anim.squash();
        }
        n
    }

    /// A screenshot went on the shelf: it leaves after `screenshot_secs`, and with
    /// `delete_after_paste` it is deleted once pasted.
    pub(super) fn track_shot(&mut self, hwnd: HWND, path: std::path::PathBuf, now: Instant) {
        self.shots.push((path.clone(), now));
        self.paste = self.cfg.shelf.delete_after_paste.then(|| Paste {
            path,
            until: now + PASTE_WINDOW,
            key_down: ctrl_v_down(),
            due: None,
        });
        self.arm_shots(hwnd);
    }

    pub(super) fn arm_shots(&self, hwnd: HWND) {
        let ms = if self.paste.is_some() && !self.gaming {
            60
        } else if !self.shots.is_empty() && self.cfg.shelf.screenshot_secs > 0 {
            1000
        } else {
            0
        };
        if ms > 0 {
            unsafe {
                SetTimer(Some(hwnd), T_SHOT, ms, None);
            }
        }
    }

    /// Take screenshots off the shelf that outstayed `screenshot_secs` or were pasted.
    pub(super) fn tick_shots(&mut self, hwnd: HWND) {
        let now = Instant::now();
        let life = Duration::from_secs(self.cfg.shelf.screenshot_secs);
        let mut gone: Vec<std::path::PathBuf> = Vec::new();
        if !life.is_zero() {
            self.shots.retain(|(p, at)| {
                let old = now.duration_since(*at) >= life;
                if old {
                    gone.push(p.clone());
                }
                !old
            });
        }
        if !self.cfg.shelf.delete_after_paste {
            self.paste = None;
        }
        if let Some(p) = &mut self.paste {
            let clip_changed = unsafe { GetClipboardSequenceNumber() } != self.snip_seq;
            let down = (!self.gaming).then(ctrl_v_down);
            match paste_step(p, now, clip_changed, down) {
                PasteStep::Wait => {}
                PasteStep::Drop => self.paste = None,
                PasteStep::Delete => {
                    gone.push(p.path.clone());
                    self.paste = None;
                }
            }
        }
        if !gone.is_empty() {
            for p in gone {
                // Only what is still on the shelf: a file the user dragged out is theirs.
                if let Some(id) = self
                    .model
                    .shelf
                    .iter()
                    .find(|i| i.owned && i.path == p)
                    .map(|i| i.id)
                {
                    crate::shelf::remove(&mut self.model.shelf, id);
                    if self.model.shelf_sel == Some(id) {
                        self.model.shelf_sel = None;
                    }
                }
                self.shots.retain(|(q, _)| *q != p);
            }
            self.save_shelf();
            self.layout();
            self.after_model_change(hwnd);
        }
        self.arm_shots(hwnd);
    }

    /// Sounds are silent right now (`[general] quiet_hours`).
    pub(super) fn quiet(&self) -> bool {
        let spec = &self.cfg.general.quiet_hours;
        if spec.is_empty() {
            return false;
        }
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        crate::config::in_quiet_hours(spec, t.wHour as u32 * 60 + t.wMinute as u32)
    }

    pub(super) fn save_shelf(&self) {
        if self.cfg.shelf.persist {
            crate::shelf::save(&self.model.shelf);
        }
    }

    /// Ask a worker for thumbnails of visible shelf tiles we do not have yet.
    pub(super) fn request_thumbs(&mut self) {
        for id in self.rend.missing_thumbs(&self.model) {
            if !self.thumb_req.insert(id) {
                continue;
            }
            if let Some(it) = self.model.shelf.iter().find(|i| i.id == id) {
                let path = it.path.clone();
                let sh = self.shared.clone();
                std::thread::spawn(move || {
                    let b = crate::shelf::thumbnail(&path).map(Arc::new);
                    push_event(&sh, UiEvent::Thumb(id, b));
                });
            }
        }
    }

    /// Keep the pill on the virtual desktop the user is looking at.
    pub(super) fn follow_desktop(&self, hwnd: HWND) {
        let Some(vdm) = &self.vdm else { return };
        unsafe {
            if vdm
                .IsWindowOnCurrentVirtualDesktop(hwnd)
                .map(|b| b.as_bool())
                .unwrap_or(true)
            {
                return;
            }
            let fg = GetForegroundWindow();
            if fg.0.is_null() {
                return;
            }
            if let Ok(id) = vdm.GetWindowDesktopId(fg) {
                let _ = vdm.MoveWindowToDesktop(hwnd, &id);
            }
        }
    }
}
