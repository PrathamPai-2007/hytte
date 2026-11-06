//! The settings table: every row of the settings pane and window, and the notes that explain them.

use super::*;

pub(super) type Choice = (&'static str, Val);

/// One line of settings: a switch, a few steps, or the opacity slider.
pub enum Row {
    Switch(&'static str, &'static str, bool),
    /// Steps; the the last is the value in the file, and how it reads, when it is none of them (numbers only).
    Pick(
        &'static str,
        &'static str,
        Vec<Choice>,
        usize,
        Option<(String, Val)>,
    ),
    /// Steps plus a last "Custom" segment for a value only `config.toml` can hold: the
    /// segment opens the file with a note (`Action::EditConfig(hint)`). `String` is what the
    /// file holds when it is none of the steps (shown beside the label), empty otherwise.
    PickCustom(
        &'static str,
        &'static str,
        Vec<Choice>,
        usize,
        &'static str,
        String,
    ),
    /// Pill opacity slider, current value.
    Opacity(f32),
    /// Glow strength slider, current value (0 to `GLOW_MAX`).
    Glow(f32),
}

impl Row {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Row::Switch(l, ..) | Row::Pick(l, ..) | Row::PickCustom(l, ..) => l,
            Row::Opacity(_) => "Pill opacity when open",
            Row::Glow(_) => "Glow strength",
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Row::Switch(_, k, _) | Row::Pick(_, k, ..) | Row::PickCustom(_, k, ..) => k,
            Row::Opacity(_) => "general.pill_opacity",
            Row::Glow(_) => "general.glow_strength",
        }
    }

    pub(super) fn relabel(self, l: &'static str) -> Row {
        match self {
            Row::Switch(_, k, v) => Row::Switch(l, k, v),
            Row::Pick(_, k, o, i, off) => Row::Pick(l, k, o, i, off),
            Row::PickCustom(_, k, o, i, h, s) => Row::PickCustom(l, k, o, i, h, s),
            o @ (Row::Opacity(_) | Row::Glow(_)) => o,
        }
    }

    /// Where the value sits, to compare with the default's.
    pub(super) fn at(&self) -> f32 {
        match self {
            Row::Switch(_, _, on) => *on as u8 as f32,
            Row::Pick(_, _, _, i, off) => *i as f32 + if off.is_none() { 0.0 } else { 0.5 },
            Row::PickCustom(_, _, _, i, ..) => *i as f32,
            Row::Opacity(v) | Row::Glow(v) => *v,
        }
    }

    /// The action that writes this row's value as it is shown (for a default, the reset).
    pub fn set_action(&self) -> Option<Action> {
        match self {
            Row::Switch(_, k, on) => Some(Action::Set(k, Val::B(*on))),
            Row::Pick(_, k, _, _, Some((_, v))) => Some(Action::Set(k, *v)),
            Row::Pick(_, k, o, i, None) | Row::PickCustom(_, k, o, i, ..) => {
                o.get(*i).map(|(_, v)| Action::Set(k, *v))
            }
            Row::Opacity(v) => Some(Action::Set("general.pill_opacity", Val::F(*v as f64))),
            Row::Glow(v) => Some(Action::Set("general.glow_strength", Val::F(*v as f64))),
        }
    }
}

/// A titled run of rows. Each row also says which page of the pill's small settings pane
/// it appears on (-1: only in the big settings window).
pub struct Section {
    pub name: &'static str,
    pub rows: Vec<(Row, i8)>,
}

pub(super) fn pick_s(
    label: &'static str,
    key: &'static str,
    opts: &[(&'static str, &'static str)],
    cur: &str,
) -> Row {
    let i = opts.iter().position(|(_, v)| *v == cur).unwrap_or(0);
    Row::Pick(
        label,
        key,
        opts.iter().map(|(n, v)| (*n, Val::S(v))).collect(),
        i,
        None,
    )
}

/// `pick_s` with a trailing "Custom" segment (see `Row::PickCustom`).
pub(super) fn pick_s_custom(
    label: &'static str,
    key: &'static str,
    opts: &[(&'static str, &'static str)],
    cur: &str,
    hint: &'static str,
) -> Row {
    let found = opts.iter().position(|(_, v)| *v == cur);
    let custom = if found.is_none() {
        cur.to_string()
    } else {
        String::new()
    };
    Row::PickCustom(
        label,
        key,
        opts.iter().map(|(n, v)| (*n, Val::S(v))).collect(),
        found.unwrap_or(opts.len()),
        hint,
        custom,
    )
}

pub(super) fn is_numeric(opts: &[Choice]) -> bool {
    matches!(opts.first(), Some((_, Val::I(_) | Val::F(_))))
}

pub(super) fn row_label_opacity() -> &'static str {
    "Pill opacity when open"
}

/// Where the glow slider ends: 20 % past the old "Strong" (1.6).
pub const GLOW_MAX: f32 = 1.92;

/// The unit a number in `config.toml` is in, from the end of its key.
pub(super) fn unit(key: &str) -> &'static str {
    match key {
        k if k.ends_with("_ms") => " ms",
        k if k.ends_with("secs") => " s",
        k if k.ends_with("_min") => " min",
        k if k.ends_with("_mb") => " MB",
        k if k.ends_with("_pct") => "%",
        _ => "",
    }
}

pub(super) fn label_key(key: &str) -> &str {
    key.rsplit('.').next().unwrap_or(key)
}

/// What a setting does, for the hover note and the note "Custom" writes above the key. Empty
/// for a setting whose label says it all.
pub fn help(key: &str) -> &'static str {
    match key {
        "general.theme" => "Dark, light, or follow the Windows color mode.",
        "general.hover_delay_ms" => "How long the pointer must rest on the pill before it opens. Instant opens at once.",
        "general.open_on" => "Hover opens the pill when you rest the pointer on it; Click waits for a click.",
        "general.collapse_delay_ms" => "How long the pill stays open after the pointer leaves it.",
        "general.pill_opacity" => "How see-through the open pill is.",
        "general.idle_opacity" => "How faint the closed pill gets while you are not using it.",
        "general.pill_scale" => "Scales the whole pill and its panels (1.0 is normal).",
        "general.animation" => "Speed of the open and close animation.",
        "general.glow_strength" => "How strong the colored glow around the pill is. 100% is the strongest the slider goes; 0% is off.",
        "general.adaptive_glow" => "Tints the glow with the colors of the album art that is playing.",
        "general.autostart" => "Starts Hytte when you sign in to Windows.",
        "general.start_panel" => "Which panel opens first: the last one you used, the busiest one, or Home.",
        "general.remember_panel" => "Keeps the last-used panel after Hytte restarts. Works with Panel on launch = Last used.",
        "general.pin_button" => "Adds a pin to the open pill. Pinned, it stays open until you unpin it.",
        "general.quiet_hours" => "No sounds between these hours. Custom lets you type your own times.",
        "general.fullscreen" => "What the pill does over a fullscreen or maximized window: shrink to a thin line, hide, or carry on as usual.",
        "general.monitor" => "Which screen the pill sits on: the primary one, the one with the active window, or the one under the cursor.",
        "general.finish_peek_secs" => "Opens the pill for a moment to show a task that finished or failed. Off keeps it closed.",
        "general.hide_from_capture" => "Keeps the pill out of screenshots, screen shares and recordings.",
        "general.add_to_path" => "Lets you run the hytte command from any terminal.",
        "general.check_updates" => "Checks GitHub for a new Hytte release and offers to install it.",
        "timer.sound" => "Plays a short chime when a timer or focus round ends.",
        "timer.break_min" => "Length of the short break between focus rounds, in minutes.",
        "timer.long_break_min" => "Length of the longer break after several focus rounds, in minutes.",
        "timer.rounds" => "How many focus rounds run before a long break.",
        "general.task_rows" => "How many terminal and agent tasks show before the list scrolls.",
        "agent.sound" => "Plays a sound when an AI agent is waiting for your answer.",
        "shelf.mode" => "Link keeps a pointer to the original file; Copy stores its own copy on the shelf.",
        "shelf.max_items" => "The most items the shelf holds; the oldest drops off first.",
        "shelf.persist" => "Keeps shelf items after Hytte restarts.",
        "shelf.screenshot_secs" => "How long a screenshot stays on the shelf, in seconds. Keep never removes it.",
        "shelf.delete_after_paste" => "Deletes the screenshot file about two seconds after you paste it somewhere.",
        "shelf.remove_after_drag" => "Takes an item off the shelf once you have dragged it out.",
        "shelf.screenshots" => "Adds new screenshots (Win+Shift+S, Print Screen) to the shelf.",
        "downloads.enabled" => "Shows a card when a file finishes downloading.",
        "calendar.enabled" => "Warns shortly before an event in your Windows calendar starts.",
        "calendar.lead_min" => "How many minutes before an event the warning appears.",
        "hog.enabled" => "Alerts when one app keeps using a lot of CPU or memory.",
        "hog.cpu_pct" => "CPU use that counts as heavy, in percent.",
        "hog.mem_mb" => "Memory use that counts as heavy, in megabytes.",
        "hog.secs" => "How long the usage must last before you are alerted, in seconds.",
        "ports.show_all" => "Lists every program listening on a port, not only likely dev servers.",
        "ports.poll_secs" => "How often the ports list refreshes while it is open, in seconds.",
        "shell.threshold_ms" => "Terminal commands that run longer than this show up as tasks, in milliseconds.",
        _ => "",
    }
}

/// The shorter name a row has in the pill's small settings pane.
pub(super) fn pane_label(key: &str) -> Option<&'static str> {
    Some(match key {
        "general.theme" => "Theme",
        "general.hover_delay_ms" => "Open delay",
        "general.pill_opacity" => "Opacity",
        "general.glow_strength" => "Glow",
        "general.adaptive_glow" => "Art-tinted glow",
        "general.autostart" => "Start with Windows",
        "general.fullscreen" => "In fullscreen",
        "general.monitor" => "Show on screen",
        "general.finish_peek_secs" => "Peek on finish",
        "timer.sound" => "Timer chime",
        "general.check_updates" => "Update check",
        "shelf.screenshots" => "Shelve screenshots",
        "downloads.enabled" => "Download alerts",
        "calendar.enabled" => "Calendar alerts",
        "hog.enabled" => "CPU / RAM alerts",
        _ => return None,
    })
}

/// Whole-number steps; the one nearest to what the file says is lit.
pub(super) fn pick_i(
    label: &'static str,
    key: &'static str,
    opts: &[(&'static str, i64)],
    cur: i64,
) -> Row {
    let i = (0..opts.len())
        .min_by_key(|&i| (opts[i].1 - cur).abs())
        .unwrap_or(0);
    Row::Pick(
        label,
        key,
        opts.iter().map(|(n, v)| (*n, Val::I(*v))).collect(),
        i,
        (opts[i].1 != cur).then(|| (format!("{cur}{}", unit(label_key(key))), Val::I(cur))),
    )
}

pub(super) fn pick_f(
    label: &'static str,
    key: &'static str,
    opts: &[(&'static str, f64)],
    cur: f64,
) -> Row {
    let d = |i: usize| (opts[i].1 - cur).abs();
    let i = (0..opts.len())
        .min_by(|&a, &b| d(a).total_cmp(&d(b)))
        .unwrap_or(0);
    Row::Pick(
        label,
        key,
        opts.iter().map(|(n, v)| (*n, Val::F(*v))).collect(),
        i,
        ((opts[i].1 - cur).abs() >= 1e-3)
            .then(|| (format!("{}", (cur * 100.0).round() / 100.0), Val::F(cur))),
    )
}

/// Every setting that is a switch, a slider or a short list of steps. Lists and text
/// (allow / deny lists, ports, hotkey, folders) stay in config.toml.
pub fn settings_table(c: &Config) -> Vec<Section> {
    let (g, sh) = (&c.general, &c.shelf);
    let sw = Row::Switch;
    vec![
        Section {
            name: "Look and feel",
            rows: vec![
                (
                    pick_s(
                        "Color theme",
                        "general.theme",
                        &[("Dark", "dark"), ("Light", "light"), ("System", "auto")],
                        &g.theme,
                    ),
                    0,
                ),
                (
                    pick_i(
                        "Wait before opening on hover",
                        "general.hover_delay_ms",
                        &[("Instant", 0), ("0.2s", 200), ("0.5s", 500), ("1s", 1000)],
                        g.hover_delay_ms as i64,
                    ),
                    0,
                ),
                (
                    pick_s(
                        "How the pill opens",
                        "general.open_on",
                        &[("Hover", "hover"), ("Click", "click")],
                        &g.open_on,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Wait before closing",
                        "general.collapse_delay_ms",
                        &[("0.1s", 100), ("0.3s", 300), ("0.6s", 600), ("1s", 1000)],
                        g.collapse_delay_ms as i64,
                    ),
                    -1,
                ),
                (Row::Opacity(g.pill_opacity), 0),
                (
                    pick_f(
                        "Opacity when idle",
                        "general.idle_opacity",
                        &[("100%", 1.0), ("75%", 0.75), ("50%", 0.5), ("30%", 0.3)],
                        g.idle_opacity as f64,
                    ),
                    -1,
                ),
                (
                    pick_f(
                        "Pill size",
                        "general.pill_scale",
                        &[
                            ("Small", 0.85),
                            ("Normal", 1.0),
                            ("Large", 1.2),
                            ("XL", 1.4),
                        ],
                        g.pill_scale as f64,
                    ),
                    -1,
                ),
                (
                    pick_s(
                        "Animation speed",
                        "general.animation",
                        &[("Slow", "calm"), ("Normal", "normal"), ("Fast", "snappy")],
                        &g.animation,
                    ),
                    -1,
                ),
                (Row::Glow(g.glow_strength), 0),
                (
                    sw(
                        "Tint glow from album art",
                        "general.adaptive_glow",
                        g.adaptive_glow,
                    ),
                    0,
                ),
            ],
        },
        Section {
            name: "Behaviour",
            rows: vec![
                (
                    sw("Start Hytte with Windows", "general.autostart", g.autostart),
                    1,
                ),
                (
                    pick_s(
                        "Panel shown on launch",
                        "general.start_panel",
                        &[("Last used", "last"), ("Busiest", "auto"), ("Home", "home")],
                        &g.start_panel,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Remember last panel after restart",
                        "general.remember_panel",
                        g.remember_panel,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Show a pin button on the open pill",
                        "general.pin_button",
                        g.pin_button,
                    ),
                    -1,
                ),
                (
                    pick_s_custom(
                        "Mute sounds during quiet hours",
                        "general.quiet_hours",
                        &[
                            ("Off", ""),
                            ("Night", "22:00-07:00"),
                            ("Late", "00:00-08:00"),
                            ("Work", "09:00-17:00"),
                        ],
                        &g.quiet_hours,
                        "general.quiet_hours",
                    ),
                    -1,
                ),
                (
                    pick_s(
                        "When an app is fullscreen",
                        "general.fullscreen",
                        &[("Thin line", "line"), ("Hide", "hide"), ("Show", "off")],
                        &g.fullscreen,
                    ),
                    1,
                ),
                (
                    pick_s(
                        "Show the pill on screen",
                        "general.monitor",
                        &[
                            ("Main", "primary"),
                            ("With app", "active"),
                            ("Cursor", "cursor"),
                        ],
                        &g.monitor,
                    ),
                    1,
                ),
                (
                    pick_i(
                        "Open the pill when a task ends",
                        "general.finish_peek_secs",
                        &[("Off", 0), ("3 s", 3), ("5 s", 5), ("10 s", 10)],
                        g.finish_peek_secs as i64,
                    ),
                    1,
                ),
                (
                    sw(
                        "Hide pill from screenshots and recordings",
                        "general.hide_from_capture",
                        g.hide_from_capture,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Run hytte from any terminal (PATH)",
                        "general.add_to_path",
                        g.add_to_path,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Check for updates automatically",
                        "general.check_updates",
                        g.check_updates,
                    ),
                    2,
                ),
            ],
        },
        Section {
            name: "Timer",
            rows: vec![
                (
                    sw(
                        "Play a chime when the timer ends",
                        "timer.sound",
                        c.timer.sound,
                    ),
                    1,
                ),
                (
                    pick_i(
                        "Short break length",
                        "timer.break_min",
                        &[("3 min", 3), ("5 min", 5), ("10 min", 10)],
                        c.timer.break_min as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Long break length",
                        "timer.long_break_min",
                        &[
                            ("10 min", 10),
                            ("15 min", 15),
                            ("20 min", 20),
                            ("30 min", 30),
                        ],
                        c.timer.long_break_min as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Focus rounds before a long break",
                        "timer.rounds",
                        &[("2", 2), ("3", 3), ("4", 4), ("6", 6)],
                        c.timer.rounds as i64,
                    ),
                    -1,
                ),
            ],
        },
        Section {
            name: "AI agents",
            rows: vec![
                (
                    pick_i(
                        "Task rows shown before scrolling",
                        "general.task_rows",
                        &[("3", 3), ("4", 4), ("6", 6), ("8", 8)],
                        g.task_rows as i64,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Sound when an AI agent needs you",
                        "agent.sound",
                        c.agent.sound,
                    ),
                    -1,
                ),
            ],
        },
        Section {
            name: "Shelf",
            rows: vec![
                (
                    pick_s(
                        "Shelf stores files as",
                        "shelf.mode",
                        &[("Link", "reference"), ("Copy", "copy")],
                        &sh.mode,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Most items on the shelf",
                        "shelf.max_items",
                        &[("6", 6), ("12", 12), ("24", 24), ("48", 48)],
                        sh.max_items as i64,
                    ),
                    -1,
                ),
                (
                    sw("Keep the shelf after restart", "shelf.persist", sh.persist),
                    -1,
                ),
                (
                    pick_i(
                        "Remove screenshots after",
                        "shelf.screenshot_secs",
                        &[("Never", 0), ("30 s", 30), ("1 min", 60), ("5 min", 300)],
                        sh.screenshot_secs as i64,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Delete a screenshot after pasting it",
                        "shelf.delete_after_paste",
                        sh.delete_after_paste,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Remove an item after dragging it out",
                        "shelf.remove_after_drag",
                        sh.remove_after_drag,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Add new screenshots to the shelf",
                        "shelf.screenshots",
                        sh.screenshots,
                    ),
                    2,
                ),
            ],
        },
        Section {
            name: "Alerts",
            rows: vec![
                (
                    sw(
                        "Notify when a download finishes",
                        "downloads.enabled",
                        c.downloads.enabled,
                    ),
                    2,
                ),
                (
                    sw(
                        "Warn before calendar events",
                        "calendar.enabled",
                        c.calendar.enabled,
                    ),
                    2,
                ),
                (
                    pick_i(
                        "Warn this long before an event",
                        "calendar.lead_min",
                        &[("2 min", 2), ("5 min", 5), ("10 min", 10), ("15 min", 15)],
                        c.calendar.lead_min as i64,
                    ),
                    -1,
                ),
                (
                    sw("Alert on CPU or memory hogs", "hog.enabled", c.hog.enabled),
                    2,
                ),
                (
                    pick_i(
                        "CPU level that counts as heavy",
                        "hog.cpu_pct",
                        &[("50%", 50), ("70%", 70), ("85%", 85), ("95%", 95)],
                        c.hog.cpu_pct as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Memory level that counts as heavy",
                        "hog.mem_mb",
                        &[
                            ("0.5 GB", 512),
                            ("1 GB", 1024),
                            ("2 GB", 2048),
                            ("4 GB", 4096),
                        ],
                        c.hog.mem_mb as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Only if it lasts at least",
                        "hog.secs",
                        &[("10 s", 10), ("30 s", 30), ("60 s", 60), ("2 min", 120)],
                        c.hog.secs as i64,
                    ),
                    -1,
                ),
            ],
        },
        Section {
            name: "Ports and terminal",
            rows: vec![
                (
                    sw(
                        "List every listening port",
                        "ports.show_all",
                        c.ports.show_all,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Refresh the ports list every",
                        "ports.poll_secs",
                        &[("2 s", 2), ("5 s", 5), ("10 s", 10), ("30 s", 30)],
                        c.ports.poll_secs as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Show terminal commands slower than",
                        "shell.threshold_ms",
                        &[("1 s", 1000), ("3 s", 3000), ("5 s", 5000), ("10 s", 10000)],
                        c.shell.threshold_ms as i64,
                    ),
                    -1,
                ),
            ],
        },
    ]
}
