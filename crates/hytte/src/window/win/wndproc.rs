//! The window procedure: every Windows message the pill handles.

use super::*;

pub(super) unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_TICK => {
            if let Some(sh) = SHARED.get() {
                sh.tick_pending.store(false, Ordering::SeqCst);
            }
            with_ui(|ui| ui.frame(hwnd));
            return LRESULT(0);
        }
        WM_UI => {
            with_ui(|ui| ui.handle_events(hwnd));
            return LRESULT(0);
        }
        WM_FG => {
            // No reassert_topmost here: the new foreground may be a game going fullscreen,
            // and a topmost window pushed over it can knock it out of exclusive mode.
            // T_FS reasserts once the decision is made.
            unsafe {
                SetTimer(Some(hwnd), T_FS, 150, None);
            }
            return LRESULT(0);
        }
        WM_DRAG_ARM | WM_DRAG_END => {
            with_ui(|ui| {
                ui.armed = msg == WM_DRAG_ARM;
                if !ui.armed {
                    ui.model.drop_over = false;
                }
                ui.layout();
                ui.kick();
            });
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            let scrubbing = with_ui(|ui| {
                ui.scrub.is_some() && {
                    ui.set_mouse_client(lparam);
                    ui.scrub_to();
                    true
                }
            });
            if scrubbing == Some(true) {
                return LRESULT(0);
            }
            // Past a 4 px threshold with the button held, a shelf tile becomes a real drag.
            let start = with_ui(|ui| {
                ui.set_mouse_client(lparam);
                let (id, (sx, sy)) = ui.drag?;
                let (mx, my) = ui.mouse?;
                if wparam.0 & 1 == 0 {
                    ui.drag = None;
                    return None;
                }
                if (mx - sx).abs() <= 4 && (my - sy).abs() <= 4 {
                    return None;
                }
                ui.drag = None;
                unsafe {
                    let _ = ReleaseCapture();
                }
                let it = ui.model.shelf.iter().find(|i| i.id == id)?;
                Some((id, it.path.clone()))
            })
            .flatten();
            if let Some((id, path)) = start {
                // DoDragDrop pumps messages: run it outside the UI borrow so frames keep flowing.
                let effect = shelf_drag(&path);
                with_ui(|ui| {
                    if effect != DROPEFFECT_NONE && ui.cfg.shelf.remove_after_drag {
                        crate::shelf::remove_keep_file(&mut ui.model.shelf, id);
                        ui.save_shelf();
                    }
                    ui.model.shelf.retain(|i| i.path.exists());
                    ui.after_model_change(hwnd);
                });
                return LRESULT(0);
            }
            with_ui(|ui| {
                ui.set_mouse_client(lparam);
                let inside = ui.pill_contains();
                if !ui.hover {
                    ui.bubble_pref = ui.bubble_hit();
                    ui.zone = None;
                }
                ui.follow_spot();
                if !ui.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    ui.tracking = true;
                }
                if inside && !ui.inside {
                    unsafe {
                        let _ = KillTimer(Some(hwnd), T_COLLAPSE);
                        if !ui.hover && ui.cfg.general.open_on != "click" {
                            let ms = ui.cfg.general.hover_delay_ms.min(2000);
                            if ms == 0 {
                                ui.open_hover();
                            } else {
                                SetTimer(Some(hwnd), T_DWELL, ms, None);
                            }
                        }
                    }
                } else if !inside && ui.inside {
                    unsafe {
                        let _ = KillTimer(Some(hwnd), T_DWELL);
                        if ui.hover {
                            SetTimer(Some(hwnd), T_COLLAPSE, ui.cfg.general.collapse_delay_ms.clamp(20, 5000), None);
                        }
                    }
                }
                ui.inside = inside;
                let over = ui.hit_at().is_some();
                ui.over_hit = over;
                ui.layout();
                ui.kick();
            });
            return LRESULT(0);
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = (wparam.0 >> 16) as i16 as i32;
            with_ui(|ui| {
                if msg == WM_MOUSEHWHEEL {
                    ui.hwheel(delta);
                } else {
                    ui.wheel(delta);
                }
                ui.after_model_change(hwnd);
            });
            return LRESULT(0);
        }
        WM_CLIPBOARDUPDATE => {
            with_ui(|ui| ui.on_clipboard(hwnd));
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            with_ui(|ui| {
                ui.tracking = false;
                ui.inside = false;
                ui.bubble_pref = None;
                ui.mouse = None;
                unsafe {
                    let _ = KillTimer(Some(hwnd), T_DWELL);
                    if ui.hover {
                        SetTimer(Some(hwnd), T_COLLAPSE, ui.cfg.general.collapse_delay_ms.clamp(20, 5000), None);
                    }
                }
                ui.layout();
                ui.kick();
            });
            return LRESULT(0);
        }
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                let hand = with_ui(|ui| ui.over_hit).unwrap_or(false);
                unsafe {
                    if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
                        SetCursor(Some(c));
                    }
                }
                return LRESULT(1);
            }
        }
        WM_LBUTTONDOWN => {
            with_ui(|ui| {
                ui.set_mouse_client(lparam);
                ui.pressed = ui.hit_at().is_some();
                ui.kick();
                ui.drag = match ui.hit_at().map(|h| h.action.clone()) {
                    Some(Action::ShelfTile(id)) => ui.mouse.map(|p| (id, p)),
                    _ => None,
                };
                ui.scrub = ui
                    .hit_at()
                    .filter(|h| {
                        matches!(
                            h.action,
                            Action::Volume | Action::Seek | Action::AppVolume(_) | Action::Opacity
                        )
                    })
                    .map(|h| (h.action.clone(), h.rect.0, h.rect.2));
                if ui.scrub.is_some() {
                    ui.scrub_to();
                }
                if ui.drag.is_some() || ui.scrub.is_some() {
                    // Capture so a fast flick off the pill still reaches us and starts the drag.
                    unsafe {
                        SetCapture(hwnd);
                    }
                }
            });
            return LRESULT(0);
        }
        WM_CAPTURECHANGED => {
            // Capture taken away mid-drag (Alt+Tab, a dialog): the drag is over.
            with_ui(|ui| ui.scrub = None);
        }
        WM_LBUTTONUP => {
            with_ui(|ui| {
                ui.set_mouse_client(lparam);
                ui.pressed = false;
                ui.kick();
                // A press that never became a drag is a click.
                ui.drag = None;
                unsafe {
                    let _ = ReleaseCapture();
                }
                if ui.scrub.is_some() {
                    // The level was applied while dragging; settle on the final one. A
                    // seek is sent now, once, to where the knob was let go.
                    ui.scrub_to();
                    if ui.scrubbing(&Action::Opacity) {
                        let v = ui.cfg.general.pill_opacity as f64;
                        ui.scrub = None;
                        ui.run_action(hwnd, Action::Set("general.pill_opacity", crate::config::Val::F(v)));
                        return;
                    }
                    if ui.scrubbing(&Action::Seek) {
                        if let Some(md) = &ui.model.media {
                            crate::media::control(crate::media::Cmd::Seek(md.pos_ms));
                        }
                    } else {
                        ui.model.speaker = crate::mic::speaker().or(ui.model.speaker);
                    }
                    ui.scrub = None;
                    ui.after_model_change(hwnd);
                    return;
                }
                // Click mode: the first click on the closed pill opens it and does nothing else.
                if ui.cfg.general.open_on == "click"
                    && !ui.hover
                    && ui.pill_contains()
                    && ui.hit_at().is_none()
                {
                    ui.open_hover();
                    return;
                }
                if let Some(a) = ui.hit_at().map(|h| h.action.clone()) {
                    ui.run_action(hwnd, a);
                }
            });
            return LRESULT(0);
        }
        WM_RBUTTONUP => {
            with_ui(|ui| {
                ui.set_mouse_client(lparam);
                if ui.pill_contains() {
                    ui.model.timer_panel = !ui.model.timer_panel;
                    if ui.model.timer_panel {
                        ui.hover = true;
                        unsafe {
                            let _ = KillTimer(Some(hwnd), T_COLLAPSE);
                        }
                    }
                    ui.after_model_change(hwnd);
                }
            });
            return LRESULT(0);
        }
        WM_TIMER => {
            let id = wparam.0;
            unsafe {
                let _ = KillTimer(Some(hwnd), id);
            }
            with_ui(|ui| match id {
                T_DWELL => {
                    if ui.inside {
                        ui.open_hover();
                    }
                }
                T_COLLAPSE => {
                    if !ui.inside && ui.kb.is_none() && !ui.model.pinned {
                        if let Some(z) = ui.zone.take() {
                            ui.model.selected = z.base;
                        }
                        ui.remember_panel();
                        ui.hover = false;
                        ui.model.timer_panel = false;
                        ui.model.close_media_extras();
                        match ui.cfg.general.start_panel.as_str() {
                            "auto" => ui.model.selected = None,
                            "home" => ui.model.selected = Some(Panel::Home),
                            _ => {}
                        }
                        ui.layout();
                        ui.kick();
                    }
                }
                T_EXPIRE | T_TIMER | T_SCRIPT => ui.after_model_change(hwnd),
                T_CARET => ui.caret_tick(hwnd),
                T_PLACE => ui.place_search(),
                T_WIDE => ui.wide_offer_due(),
                T_TRIM => {
                    if !ui.shared.animating.load(Ordering::SeqCst) {
                        // Drops cold pages (decoder and COM leftovers); they fault back in on use.
                        unsafe {
                            let _ = SetProcessWorkingSetSize(
                                GetCurrentProcess(),
                                usize::MAX,
                                usize::MAX,
                            );
                        }
                    }
                }
                T_GPU => ui.rebuild_gpu(hwnd),
                T_SHOT => ui.tick_shots(hwnd),
                T_FS => {
                    // Settled after a foreground change or move: stay above it,
                    // unless it is a game.
                    ui.follow_monitor();
                    ui.evaluate_fullscreen(hwnd);
                    if !ui.gaming {
                        reassert_topmost(hwnd);
                    }
                    ui.follow_desktop(hwnd);
                }
                _ => {}
            });
            return LRESULT(0);
        }
        0x0312 /* WM_HOTKEY */ => {
            if wparam.0 as i32 == MONITOR_HOTKEY_ID {
                with_ui(|ui| ui.next_monitor());
            } else if wparam.0 as i32 == NOTE_HOTKEY_ID {
                with_ui(|ui| ui.toggle_note(hwnd));
            } else {
                with_ui(|ui| ui.toggle_keyboard(hwnd));
            }
            return LRESULT(0);
        }
        WM_KEYDOWN => {
            with_ui(|ui| ui.key(hwnd, wparam.0 as u16));
            return LRESULT(0);
        }
        WM_CHAR => {
            with_ui(|ui| ui.type_char(hwnd, wparam.0 as u16));
            return LRESULT(0);
        }
        WM_KILLFOCUS => {
            // Clicked or tabbed elsewhere: keyboard mode ends without stealing focus back.
            with_ui(|ui| ui.end_keyboard(false));
        }
        WM_WTSSESSION_CHANGE => {
            // The PC was locked: the clipboard history is forgotten.
            if wparam.0 as u32 == windows::Win32::UI::WindowsAndMessaging::WTS_SESSION_LOCK {
                with_ui(|ui| ui.model.clips.clear());
            }
            return LRESULT(0);
        }
        WM_POWERBROADCAST => {
            // Power source / charge level changed, or the PC resumed.
            crate::power::poke();
            if wparam.0 as u32 == PBT_APMSUSPEND {
                // Going to sleep: the clipboard history is forgotten.
                with_ui(|ui| ui.model.clips.clear());
            }
            if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC {
                // The foreground may have changed fullscreen state while asleep.
                post(WM_FG);
            }
            return LRESULT(1);
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            with_ui(|ui| {
                ui.refresh_monitor();
                ui.kick();
            });
            post(WM_FG);
            return LRESULT(0);
        }
        WM_SETTINGCHANGE => {
            with_ui(|ui| {
                ui.anim.reduce = reduce_motion();
                ui.light = theme_is_light(&ui.cfg.general.theme);
                // The accent colour changing also arrives here (ImmersiveColorSet).
                ui.accent = windows_accent();
                ui.layout();
                ui.kick();
            });
            crate::tray::Tray::new(hwnd).refresh();
        }
        x if x == WM_TRAY => {
            let code = (lparam.0 & 0xFFFF) as u32;
            // Left or right click: the same menu (there is no window to open).
            if code == WM_RBUTTONUP || code == WM_LBUTTONUP {
                let (paused, auto, hotkey) = with_ui(|ui| {
                    (
                        ui.paused,
                        ui.cfg.general.autostart,
                        ui.cfg.general.hotkey.clone(),
                    )
                })
                .unwrap_or_default();
                let terminal = crate::setup::is_set_up();
                let agents = crate::agents::is_connected();
                crate::tray::Tray::new(hwnd).popup(
                    paused,
                    auto,
                    terminal,
                    agents,
                    monitors().len() > 1,
                    &hotkey,
                );
            }
            return LRESULT(0);
        }
        WM_COMMAND => {
            match (wparam.0 & 0xFFFF) as u32 {
                12 => unsafe {
                    let _ = DestroyWindow(hwnd);
                },
                10 => {
                    with_ui(|ui| {
                        ui.paused = !ui.paused;
                        ui.show_update(Instant::now());
                        ui.layout();
                        ui.kick();
                    });
                }
                11 => {
                    let _ = std::process::Command::new("explorer")
                        .arg(
                            crate::config::config_path()
                                .parent()
                                .unwrap_or(std::path::Path::new(".")),
                        )
                        .spawn();
                }
                14 => crate::setup::tray_setup(),
                19 => {
                    with_ui(|ui| {
                        crate::settings_win::open(&ui.cfg);
                    });
                }
                16 => crate::agents::tray(),
                15 => crate::update::check_now(),
                18 => open_url("https://github.com/PrathamPai-2007/hytte/blob/main/manual/README.md"),
                20 => {
                    with_ui(|ui| ui.toggle_keyboard(hwnd));
                }
                17 => {
                    with_ui(|ui| ui.next_monitor());
                }
                13 => {
                    with_ui(|ui| {
                        let g = &mut ui.cfg.general;
                        g.autostart = !g.autostart;
                        crate::config::ensure_autostart(g.autostart);
                        let _ = crate::config::set("general.autostart", crate::config::Val::B(g.autostart));
                    });
                }
                _ => {}
            }
            return LRESULT(0);
        }
        x if x != 0 && x == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            crate::tray::Tray::new(hwnd).add();
            // Explorer restarted: the taskbar is new, re-check fullscreen and stay on top.
            post(WM_FG);
            return LRESULT(0);
        }
        WM_DESTROY => {
            with_ui(|ui| ui.dnd_release());
            if LIVE.load(Ordering::Relaxed) {
                unsafe { PostQuitMessage(0) };
            }
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
