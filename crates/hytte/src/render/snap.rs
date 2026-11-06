//! Offscreen pictures of the scenes, for eyeballing UI changes (`HYTTE_SNAP_DIR`).

use super::*;
use crate::ui_state::MediaInfo;
use hytte_proto::ports::PortInfo;

fn bitmap_props(opts: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: opts,
        ..Default::default()
    }
}

impl Renderer {
    /// A renderer drawing into an offscreen canvas-sized bitmap (no window).
    fn offscreen(scale: f32) -> windows::core::Result<Self> {
        let factory1: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dc = crate::gpu::offscreen(&factory1)?;
        let mut r = Self::build(factory1, dc.cast()?, None)?;
        r.set_scale(scale);
        let size = D2D_SIZE_U {
            width: r.px.0 as u32,
            height: r.px.1 as u32,
        };
        unsafe {
            let target =
                dc.CreateBitmap(size, None, 0, &bitmap_props(D2D1_BITMAP_OPTIONS_TARGET))?;
            dc.SetTarget(&target);
        }
        Ok(r)
    }

    /// An offscreen renderer of an arbitrary size (the settings window).
    fn offscreen_sized(w: i32, h: i32, scale: f32) -> windows::core::Result<Self> {
        let factory1: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dc = crate::gpu::offscreen(&factory1)?;
        let mut r = Self::build(factory1, dc.cast()?, None)?;
        r.px = (w, h);
        r.scale = scale;
        let size = D2D_SIZE_U {
            width: w as u32,
            height: h as u32,
        };
        unsafe {
            let target =
                dc.CreateBitmap(size, None, 0, &bitmap_props(D2D1_BITMAP_OPTIONS_TARGET))?;
            dc.SetTarget(&target);
        }
        Ok(r)
    }

    /// The canvas as premultiplied BGRA rows.
    fn read_back(&self) -> Vec<u8> {
        let dc: ID2D1DeviceContext = self.rt.cast().unwrap();
        let (w, h) = (self.px.0 as u32, self.px.1 as u32);
        unsafe {
            let target: ID2D1Bitmap = dc.GetTarget().unwrap().cast().unwrap();
            let cpu = dc
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: w,
                        height: h,
                    },
                    None,
                    0,
                    &bitmap_props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
                )
                .unwrap();
            cpu.CopyFromBitmap(None, &target, None).unwrap();
            let map = cpu.Map(D2D1_MAP_OPTIONS_READ).unwrap();
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                let row = map.bits.add((y * map.pitch) as usize);
                out.extend_from_slice(std::slice::from_raw_parts(row, (w * 4) as usize));
            }
            cpu.Unmap().unwrap();
            out
        }
    }
}

fn shoot(
    r: &mut Renderer,
    dir: &std::path::Path,
    name: &str,
    m: &Model,
    scene: Scene,
    light: bool,
) {
    shoot_at(r, dir, name, m, scene, light, None);
}

/// `shoot` with the pointer at a pill-relative point.
fn shoot_at(
    r: &mut Renderer,
    dir: &std::path::Path,
    name: &str,
    m: &Model,
    scene: Scene,
    light: bool,
    at: Option<(f32, f32)>,
) {
    let mut an = Anim::new(120.0, 24.0);
    an.set_scene(scene, m.size(scene), m.glow(scene));
    for _ in 0..240 {
        an.step(1.0 / 60.0);
    }
    let fr = Frame {
        model: m,
        cfg: &Config::default(),
        anim: &an,
        now: Instant::now(),
        dt: 1.0 / 60.0,
        mouse: at.map(|(x, y)| (x + (CANVAS_W - an.rect.w.pos as f32) / 2.0, y)),
        pressed: false,
        light,
        accent: BLUE,
        focus: None,
        armed: false,
        ringing: true,
        caret_on: true,
    };
    let crop = r.plan(an.rect.w.pos as f32, an.rect.h.pos as f32, false, 0.0, true);
    let mut hits = vec![];
    r.draw(&fr, crop, &mut hits);
    // Only the pill's box, read back from the canvas-sized target.
    let (x0, w, h) = (crop.clip.0, crop.clip.2 - crop.clip.0, crop.clip.3);
    let px = r.read_back();
    let cw = crop.w_px;
    // Premultiplied BGRA over the wallpaper, as an RGB PNG.
    let bg = if light {
        [226.0, 230.0, 236.0]
    } else {
        [46.0, 52.0, 58.0]
    };
    let img = image::RgbImage::from_fn(w as u32, h as u32, |x, y| {
        let i = ((y as i32 * cw + x0 + x as i32) * 4) as usize;
        let a = px[i + 3] as f32 / 255.0;
        // BGRA -> RGB
        let ch = |c: usize, b: f32| (px[i + c] as f32 + b * (1.0 - a)).min(255.0) as u8;
        image::Rgb([ch(2, bg[0]), ch(1, bg[1]), ch(0, bg[2])])
    });
    img.save(dir.join(format!("{name}.png"))).unwrap();
    SHOT.with(|v| v.borrow_mut().push(name.to_string()));
}

thread_local! {
    /// Every picture taken this run, in order, for the contact sheet.
    static SHOT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `index.html` beside the pictures: all of them on one page, named, for review.
fn contact_sheet(dir: &std::path::Path) {
    let cells: String = SHOT.with(|v| {
        v.borrow()
            .iter()
            .map(|n| {
                format!("<figure><img src=\"{n}.png\"><figcaption>{n}</figcaption></figure>\n")
            })
            .collect()
    });
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Hytte snapshots</title>\n\
         <style>body{{background:#1d2026;color:#cfd5de;font:13px system-ui;margin:24px}}\
         main{{display:grid;grid-template-columns:repeat(auto-fill,minmax(420px,1fr));gap:20px}}\
         figure{{margin:0}}img{{width:100%;border-radius:6px}}</style>\n<main>\n{cells}</main>\n"
    );
    std::fs::write(dir.join("index.html"), page).unwrap();
}

/// A model with the shipped glow strength (`Model::default` has none).
fn model() -> Model {
    Model {
        glow_gain: 1.2,
        body_alpha: 1.0,
        ..Default::default()
    }
}

/// Every row of the settings table writes a key that `config.toml` accepts, and the file
/// still parses afterwards (a typo in a key would silently write nothing useful).
#[test]
fn settings_rows_write_valid_config() {
    let cfg = Config::default();
    let mut keys = std::collections::HashSet::new();
    for sec in settings_table(&cfg) {
        for (row, _) in sec.rows {
            let writes: Vec<(&str, Val)> = match row {
                Row::Switch(_, k, _) => vec![(k, Val::B(true)), (k, Val::B(false))],
                Row::Pick(_, k, opts, ..) | Row::PickCustom(_, k, opts, ..) => {
                    opts.into_iter().map(|(_, v)| (k, v)).collect()
                }
                Row::Opacity(_) => vec![("general.pill_opacity", Val::F(0.7))],
                Row::Glow(_) => vec![("general.glow_strength", Val::F(1.0))],
            };
            for (k, v) in writes {
                keys.insert(k);
                let text = crate::config::with_value("", k, v).expect(k);
                assert!(crate::config::parse(&text).is_some(), "{k} broke the file");
                let leaf = k.rsplit('.').next().unwrap();
                assert!(text.contains(leaf), "{k} wrote nothing: {text}");
            }
        }
    }
    // No key twice in the table (a row would fight another).
    let rows: usize = settings_table(&cfg).iter().map(|s| s.rows.len()).sum();
    assert_eq!(keys.len(), rows);
}

/// The reset of every row writes its default back as a valid file, and rows at their
/// defaults compare equal to themselves (so no reset is offered for them).
#[test]
fn resets_write_the_default() {
    let cfg = Config::default();
    for sec in settings_table(&cfg) {
        for (row, _) in sec.rows {
            let Some(Action::Set(k, v)) = row.set_action() else {
                continue;
            };
            assert_eq!(k, row.key());
            let text = crate::config::with_value("", k, v).expect(k);
            let back = crate::config::parse(&text).expect(k);
            let again = settings_table(&back)
                .into_iter()
                .flat_map(|s| s.rows)
                .find(|(r, _)| r.key() == k)
                .unwrap()
                .0;
            assert!((again.at() - row.at()).abs() < 0.004, "{k}");
        }
    }
}

/// The settings window, light and dark, at the top, scrolled, and with the pointer on a row.
#[test]
#[ignore]
fn settings_window_snapshots() {
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let (w, h, sc) = (1720, 1400, 2.0);
    let cfg = Config::default();
    type Shot = (&'static str, bool, f32, Option<(f32, f32)>);
    let shots: [Shot; 5] = [
        ("settings-dark", false, 0.0, None),
        ("settings-dark-hover", false, 0.0, Some((420.0, 150.0))),
        ("settings-dark-scrolled", false, 640.0, None),
        ("settings-light", true, 0.0, Some((560.0, 118.0))),
        ("settings-narrow-light", true, 0.0, None),
    ];
    for (name, light, scroll, mouse) in shots {
        let narrow = name.contains("narrow");
        // A fresh renderer each: eased values must not carry over from the last shot.
        let mut r = Renderer::offscreen_sized(if narrow { 1120 } else { w }, h, sc).unwrap();
        let view = SettingsView {
            mouse,
            pressed: false,
            light,
            accent: BLUE,
            scroll,
            scrolling: scroll > 0.0,
            dt: 0.016,
            open: 1.0,
            toast: None,
            query: "",
        };
        let mut hits = vec![];
        // Twice: the first frame seeds the eased values, the second settles them.
        r.draw_settings_window(&cfg, &view, &mut hits);
        r.draw_settings_window(&cfg, &view, &mut hits);
        let (pw, ph) = r.px;
        let px = r.read_back();
        let img = image::RgbImage::from_fn(pw as u32, ph as u32, |x, y| {
            let i = ((y as i32 * pw + x as i32) * 4) as usize;
            image::Rgb([px[i + 2], px[i + 1], px[i]])
        });
        img.save(dir.join(format!("{name}.png"))).unwrap();
        SHOT.with(|v| v.borrow_mut().push(name.to_string()));
    }
    // A custom quiet-hours value in the file, and the note "Custom" shows.
    let mut custom = Config::default();
    custom.general.quiet_hours = "22:30-06:00".into();
    let mut r = Renderer::offscreen_sized(w, h, sc).unwrap();
    let view = SettingsView {
        mouse: None,
        pressed: false,
        light: false,
        accent: BLUE,
        scroll: 330.0,
        scrolling: false,
        dt: 0.016,
        open: 1.0,
        toast: Some(("In config.toml, under [general], set quiet_hours to your times as \"HH:MM-HH:MM\" (24-hour clock, may wrap past midnight), for example \"22:30-06:00\". Save the file and it applies.", 1.0)),
        query: "",
    };
    let mut hits = vec![];
    r.draw_settings_window(&custom, &view, &mut hits);
    r.draw_settings_window(&custom, &view, &mut hits);
    let (pw, ph) = r.px;
    let px = r.read_back();
    image::RgbImage::from_fn(pw as u32, ph as u32, |x, y| {
        let i = ((y as i32 * pw + x as i32) * 4) as usize;
        image::Rgb([px[i + 2], px[i + 1], px[i]])
    })
    .save(dir.join("settings-custom.png"))
    .unwrap();
    SHOT.with(|v| v.borrow_mut().push("settings-custom".to_string()));
    // Searching, with a few values off their defaults (so resets show).
    let mut dirty = Config::default();
    dirty.general.hover_delay_ms = 500;
    dirty.general.pill_opacity = 0.7;
    dirty.general.theme = "auto".into();
    dirty.general.collapse_delay_ms = 450;
    let scenes = [
        ("settings-search", "hover", (690.0, 150.0), 0.016),
        ("settings-nomatch", "zzz", (690.0, 150.0), 0.016),
        // Resting on a label: its note appears after a moment.
        ("settings-tip", "", (330.0, 154.0), 1.0),
    ];
    for (name, query, mouse, dt) in scenes {
        let mut r = Renderer::offscreen_sized(w, h, sc).unwrap();
        let view = SettingsView {
            mouse: Some(mouse),
            pressed: false,
            light: false,
            accent: BLUE,
            scroll: 0.0,
            scrolling: false,
            dt,
            open: 1.0,
            toast: None,
            query,
        };
        let mut hits = vec![];
        r.draw_settings_window(&dirty, &view, &mut hits);
        r.draw_settings_window(&dirty, &view, &mut hits);
        let (pw, ph) = r.px;
        let px = r.read_back();
        image::RgbImage::from_fn(pw as u32, ph as u32, |x, y| {
            let i = ((y as i32 * pw + x as i32) * 4) as usize;
            image::Rgb([px[i + 2], px[i + 1], px[i]])
        })
        .save(dir.join(format!("{name}.png")))
        .unwrap();
        SHOT.with(|v| v.borrow_mut().push(name.to_string()));
    }
    contact_sheet(&dir);
}

#[test]
#[ignore]
fn snapshots() {
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let now = Instant::now();
    let wall = crate::timer::now_ms();

    let mut m = model();
    m.now = Some(now);
    m.timer_min = 25;
    m.rounds = 4;
    m.timer_break = 5;
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        shoot(
            &mut r,
            &dir,
            &format!("timer-setup{sfx}"),
            &m,
            Scene::ExpTimer,
            light,
        );
    }
    let mut t = crate::timer::Timer::start(crate::timer::TimerKind::Focus, 25, wall - 9 * 60_000);
    m.timer = Some(t.clone());
    m.focus_done = 1;
    shoot(&mut r, &dir, "timer-running", &m, Scene::ExpTimer, false);
    t.toggle_pause(wall);
    m.timer = Some(t);
    shoot(&mut r, &dir, "timer-paused", &m, Scene::ExpTimer, false);
    m.timer = None;
    m.timer_last = Some(crate::timer::Timer::start(
        crate::timer::TimerKind::Focus,
        25,
        wall - 25 * 60_000,
    ));
    for k in [
        crate::timer::TimerKind::Focus,
        crate::timer::TimerKind::Plain,
    ] {
        m.timer_done = Some((k, now));
        shoot(
            &mut r,
            &dir,
            &format!("timer-done-{k:?}"),
            &m,
            Scene::ExpTimerDone,
            false,
        );
    }
    m.timer_done = None;

    m.media = Some(MediaInfo {
        title: "Midnight City".into(),
        artist: "M83".into(),
        playing: true,
        app: "Spotify.exe".into(),
        pos_ms: 95_000,
        dur_ms: 243_000,
    });
    m.media_stamp = Some(now);
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        shoot(
            &mut r,
            &dir,
            &format!("media{sfx}"),
            &m,
            Scene::ExpMedia,
            light,
        );
    }
    shoot(
        &mut r,
        &dir,
        "media-compact",
        &m,
        Scene::CompactMedia,
        false,
    );
    // The pointer on the timeline: it thickens and shows its knob (seekable).
    shoot_at(
        &mut r,
        &dir,
        "media-seek-hover",
        &m,
        Scene::ExpMedia,
        false,
        Some((240.0, 67.0)),
    );
    // A title too long for the pill fades out at its end.
    if let Some(i) = m.media.as_mut() {
        i.title = "Midnight City (Official Video, Remastered 2021 Extended Cut)".into();
    }
    shoot(&mut r, &dir, "media-long", &m, Scene::ExpMedia, false);
    m.media = None;

    m.power = Some(crate::power::Battery {
        pct: 64,
        plugged: false,
        watts: Some(-7.4),
        mode: Mode::Balanced,
        mins_left: Some(190),
    });
    shoot(&mut r, &dir, "power", &m, Scene::ExpPower, false);
    shoot(&mut r, &dir, "power-light", &m, Scene::ExpPower, true);
    m.power.as_mut().unwrap().plugged = true;
    m.power.as_mut().unwrap().watts = Some(24.5);
    m.power.as_mut().unwrap().mins_left = None;
    shoot(&mut r, &dir, "power-charging", &m, Scene::ExpPower, false);
    m.speaker = Some((0.64, false));
    shoot(&mut r, &dir, "home", &m, Scene::ExpHome, false);
    shoot(&mut r, &dir, "home-light", &m, Scene::ExpHome, true);
    m.mic_muted = true;
    m.speaker = Some((0.92, false));
    m.awake = crate::ui_state::Awake::Until(now + std::time::Duration::from_secs(52 * 60));
    m.timer = Some(crate::timer::Timer::start(
        crate::timer::TimerKind::Plain,
        10,
        wall,
    ));
    shoot(&mut r, &dir, "home-active", &m, Scene::ExpHome, false);
    m.timer = None;
    for (g, name) in [
        (0, "settings-look"),
        (1, "settings-behaviour"),
        (2, "settings-features"),
    ] {
        m.settings = Some(g);
        shoot(&mut r, &dir, name, &m, Scene::ExpSettings, false);
    }
    shoot(
        &mut r,
        &dir,
        "settings-features-light",
        &m,
        Scene::ExpSettings,
        true,
    );
    m.settings = None;
    shoot(&mut r, &dir, "sentinel", &m, Scene::Sentinel, false);

    let mut m = model();
    m.now = Some(now);
    shoot(&mut r, &dir, "idle", &m, Scene::Idle, false);
    let task = |id: &str, label: &str, ev, progress, line: Option<&str>, err: Option<&str>| {
        crate::tasks::TaskUpdate::Upsert(crate::tasks::TaskState {
            task_id: id.into(),
            label: label.into(),
            progress,
            event: ev,
            duration_ms: Some(83_000),
            stderr_tail: err.map(Into::into),
            source: "run".into(),
            pid: None,
            delay_ms: 0,
            message: None,
            exit_code: None,
            line: line.map(Into::into),
            cwd: Some(r"C:\src\app".into()),
            updated: now,
        })
    };
    let t0 = now - std::time::Duration::from_secs(90);
    m.apply_task(
        task(
            "a",
            "cargo build --release",
            TaskEvent::Progress,
            Some(62),
            Some("Compiling hytte v1.1.0"),
            None,
        ),
        t0,
    );
    shoot(&mut r, &dir, "task-compact", &m, Scene::CompactTask, false);
    m.apply_task(
        task(
            "b",
            "npm test",
            TaskEvent::Start,
            None,
            Some("PASS src/app.test.ts"),
            None,
        ),
        t0,
    );
    m.apply_task(
        task(
            "c",
            "pytest -q",
            TaskEvent::Failed,
            None,
            None,
            Some("E   AssertionError: 3 != 4\nFAILED tests/test_math.py::test_add"),
        ),
        t0,
    );
    m.now = Some(now);
    shoot(&mut r, &dir, "tasks", &m, Scene::ExpTasks, false);
    shoot(&mut r, &dir, "tasks-light", &m, Scene::ExpTasks, true);
    // Every pane at once, the pointer resting on a tab (its name shows above it).
    let all = &mut m;
    all.media = Some(MediaInfo {
        title: "A very long song title that will never fit in the pill".into(),
        artist: "Someone".into(),
        playing: true,
        app: String::new(),
        pos_ms: 1_000,
        dur_ms: 200_000,
    });
    all.shelf = vec![crate::shelf::ShelfItem {
        id: 9,
        path: "a.txt".into(),
        name: "a.txt".into(),
        is_dir: false,
        owned: false,
    }];
    all.timer = Some(crate::timer::Timer::start(
        crate::timer::TimerKind::Plain,
        10,
        wall,
    ));
    all.power = Some(crate::power::Battery {
        pct: 80,
        plugged: true,
        watts: None,
        mode: Mode::Balanced,
        mins_left: None,
    });
    all.selected = Some(Panel::Media);
    let h = all.size(Scene::ExpMedia).1 as f32;
    shoot_at(
        &mut r,
        &dir,
        "tabs-hover",
        all,
        Scene::ExpMedia,
        false,
        Some((236.0, h - 14.0)),
    );
    // The optional pin, off and then on.
    all.pin_button = true;
    shoot(&mut r, &dir, "tabs-pin", all, Scene::ExpMedia, false);
    all.pinned = true;
    shoot(&mut r, &dir, "tabs-pinned", all, Scene::ExpMedia, false);
    all.pin_button = false;
    all.pinned = false;
    // One pane: a dev server started from a terminal task sits under it, linked.
    let mut m = model();
    m.now = Some(now);
    let mut dev = task(
        "d",
        "npm run dev",
        TaskEvent::Start,
        None,
        Some("ready in 412 ms"),
        None,
    );
    if let crate::tasks::TaskUpdate::Upsert(st) = &mut dev {
        st.pid = Some(700);
    }
    m.apply_task(dev, t0);
    m.ports = vec![
        PortInfo {
            port: 5173,
            pid: 4388,
            exe: "node.exe".into(),
        },
        PortInfo {
            port: 8000,
            pid: 9012,
            exe: "python.exe".into(),
        },
    ];
    m.port_parents = [(4388, vec![4300, 700, 12]), (9012, vec![1])]
        .into_iter()
        .collect();
    m.now = Some(now);
    shoot(&mut r, &dir, "dev", &m, Scene::ExpTasks, false);
    m.tasks.clear();
    shoot(&mut r, &dir, "dev-servers-only", &m, Scene::ExpTasks, false);
    m.ports.clear();
    m.shelf = [
        "report-final.pdf",
        "screenshot 2026-10-19.png",
        "notes.txt",
        "build",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| crate::shelf::ShelfItem {
        id: i as u64 + 1,
        path: n.into(),
        name: (*n).into(),
        is_dir: *n == "build",
        owned: false,
    })
    .collect();
    shoot(&mut r, &dir, "shelf", &m, Scene::ExpShelf, false);
    m.shelf_sel = Some(2);
    shoot(&mut r, &dir, "shelf-selected", &m, Scene::ExpShelf, false);
    // More than fit: scrolled two along, chevrons on both sides.
    let more = m.shelf.len() as u64;
    m.shelf.extend((0..4).map(|i| crate::shelf::ShelfItem {
        id: more + i + 1,
        path: format!("extra-{i}.txt").into(),
        name: format!("extra-{i}.txt"),
        is_dir: false,
        owned: false,
    }));
    m.shelf_sel = None;
    m.shelf_off = 2;
    shoot(&mut r, &dir, "shelf-scrolled", &m, Scene::ExpShelf, false);
    m.shelf_off = 0;
    let mut m = model();
    m.now = Some(now);
    let mut c = crate::ui_state::Chip::new("Saved invoice.pdf to Downloads", now);
    c.open = Some("C:\\x".into());
    m.chip = Some(c);
    shoot(&mut r, &dir, "chip", &m, Scene::ExpChip, false);
    m.chip = None;
    m.drop_over = true;
    shoot(&mut r, &dir, "drop", &m, Scene::ExpDrop, false);
    m.drop_over = false;
    m.power = Some(crate::power::Battery {
        pct: 41,
        plugged: true,
        watts: Some(31.0),
        mode: Mode::Balanced,
        mins_left: None,
    });
    m.charge_in = true;
    shoot(&mut r, &dir, "charge", &m, Scene::ExpCharge, false);
    contact_sheet(&dir);
}

/// Notes and To-do: empty, full, editing, long text, undo, both themes. A mock "now" keeps the
/// ring and time-left text stable.
#[test]
#[ignore]
fn jot_snapshots() {
    use crate::jot::Kind;
    use crate::ui_state::Field;
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let wall = crate::timer::now_ms();
    let mut m = model();
    m.now = Some(Instant::now());
    m.init_jot(&std::path::PathBuf::from("Z:\\none\\jot.json"));
    m.panes = Config::default().panes;
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        let mut m2 = model();
        m2.now = m.now;
        m2.init_jot(&std::path::PathBuf::from("Z:\\none\\jot.json"));
        m2.panes = Config::default().panes;
        shoot(
            &mut r,
            &dir,
            &format!("notes-empty{sfx}"),
            &m2,
            Scene::ExpNotes,
            light,
        );
        shoot(
            &mut r,
            &dir,
            &format!("todo-empty{sfx}"),
            &m2,
            Scene::ExpTodo,
            light,
        );
        m2.jot.add(
            Kind::Note,
            "Wi-Fi code at the studio: 4471",
            false,
            wall - 7 * 3_600_000,
        );
        m2.jot.add(
            Kind::Note,
            "Call the dentist back after two",
            false,
            wall - 18 * 3_600_000,
        );
        m2.jot.add(
            Kind::Note,
            "Passport renewal needs two photos and the old passport",
            true,
            wall,
        );
        m2.jot.add(
            Kind::Note,
            "A much longer note that cannot possibly fit on one line of the pill and so gets trimmed",
            true,
            wall - 1000,
        );
        shoot(
            &mut r,
            &dir,
            &format!("notes-full{sfx}"),
            &m2,
            Scene::ExpNotes,
            light,
        );
        m2.note_edit.set("Ask Mina about the invoice");
        m2.typing = Some(Field::Note);
        shoot(
            &mut r,
            &dir,
            &format!("notes-typing{sfx}"),
            &m2,
            Scene::ExpNotes,
            light,
        );
        m2.note_edit.set("A very long draft that runs well past the end of the field so that it scrolls sideways");
        shoot(
            &mut r,
            &dir,
            &format!("notes-scrolled{sfx}"),
            &m2,
            Scene::ExpNotes,
            light,
        );
        m2.note_edit.clear();
        m2.typing = None;
        m2.note_keep = true;
        let at = Some((130.0, 72.0));
        shoot_at(
            &mut r,
            &dir,
            &format!("notes-hover{sfx}"),
            &m2,
            Scene::ExpNotes,
            light,
            at,
        );
        let id = m2.jot.notes()[0].id;
        m2.delete_jot(id, Instant::now());
        shoot(
            &mut r,
            &dir,
            &format!("notes-undo{sfx}"),
            &m2,
            Scene::ExpNotes,
            light,
        );
        for t in [
            "Send the April invoice",
            "Book the car service",
            "Reply to the landlord",
            "Renew the domain",
        ] {
            m2.jot.add(Kind::Todo, t, false, wall);
        }
        let done = m2.jot.todos()[2].id;
        m2.jot.toggle(done, wall);
        shoot(
            &mut r,
            &dir,
            &format!("todo-full{sfx}"),
            &m2,
            Scene::ExpTodo,
            light,
        );
        for t in ["Water the plants", "Pay the council tax"] {
            m2.jot.add(Kind::Todo, t, false, wall);
        }
        shoot(
            &mut r,
            &dir,
            &format!("todo-six{sfx}"),
            &m2,
            Scene::ExpTodo,
            light,
        );
    }
    contact_sheet(&dir);
}

/// Weather: the place chooser in each state, then every sky, loading, offline and stale.
#[test]
#[ignore]
fn weather_snapshots() {
    use crate::ui_state::Field;
    use crate::weather::{Forecast, Hour, Place};
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let wall = crate::timer::now_ms();
    let place = |n: &str, reg: &str| Place {
        name: n.into(),
        region: reg.into(),
        lat: 0.0,
        lon: 0.0,
    };
    let bergen = place("Bergen", "Vestland, Norway");
    let fc = |code: u8, day: bool, codes: [u8; 5], age_min: u64| Forecast {
        place: bergen.clone(),
        fetched_ms: wall - age_min * 60_000,
        now: Hour {
            hour: 14,
            temp: 11.4,
            code,
            day,
        },
        hi: 14.2,
        lo: 7.6,
        next: (0..5)
            .map(|i| Hour {
                hour: 15 + i as u8,
                temp: 12.0 + (i as f32) * 0.4 - if i > 3 { 2.5 } else { 0.0 },
                code: codes[i],
                day: i < 3,
            })
            .collect(),
    };
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        let mut m = model();
        m.now = Some(Instant::now());
        m.init_jot(&std::path::PathBuf::from("Z:\\none\\jot.json"));
        let s = |r: &mut Renderer, n: &str, m: &Model| {
            shoot(r, &dir, &format!("{n}{sfx}"), m, Scene::ExpWeather, light)
        };
        // Place chooser: nothing typed, typing, matches, no match, searching, offline.
        s(&mut r, "weather-ask", &m);
        m.typing = Some(Field::Place);
        m.place_edit.set("b");
        s(&mut r, "weather-ask-short", &m);
        m.place_edit.set("ber");
        m.places = vec![
            place("Berlin", "State of Berlin, Germany"),
            place("Bergen", "Vestland, Norway"),
            place("Bern", "Switzerland"),
            place("Bergamo", "Lombardy, Italy"),
        ];
        m.places_for = "ber".into();
        m.place_sel = 1;
        s(&mut r, "weather-ask-matches", &m);
        m.places.clear();
        m.place_edit.set("zzzz");
        m.places_for = "zzzz".into();
        s(&mut r, "weather-ask-none", &m);
        m.places_busy = true;
        s(&mut r, "weather-ask-searching", &m);
        m.places_busy = false;
        m.places_err = true;
        s(&mut r, "weather-ask-offline", &m);
        // A place is chosen.
        let mut m = model();
        m.now = Some(Instant::now());
        m.weather_place = Some(bergen.clone());
        s(&mut r, "weather-loading", &m);
        m.weather_err = true;
        s(&mut r, "weather-failed", &m);
        m.weather_err = false;
        for (name, code, day, codes) in [
            ("rain", 61u8, true, [61, 3, 3, 2, 1]),
            ("sunny", 0, true, [0, 0, 1, 2, 1]),
            ("night", 0, false, [0, 0, 1, 2, 3]),
            ("partly-night", 2, false, [2, 2, 3, 3, 0]),
            ("cloud", 3, true, [3, 3, 45, 3, 3]),
            ("snow", 73, true, [73, 71, 3, 3, 0]),
            ("thunder", 95, true, [95, 80, 61, 3, 2]),
            ("fog", 45, true, [45, 45, 3, 2, 0]),
        ] {
            m.forecast = Some(fc(code, day, codes, 5));
            s(&mut r, &format!("weather-{name}"), &m);
        }
        m.forecast = Some(fc(61, true, [61, 3, 3, 2, 1], 95));
        s(&mut r, "weather-stale", &m);
        m.weather_err = true;
        s(&mut r, "weather-stale-offline", &m);
        m.weather_err = false;
        m.imperial = true;
        m.forecast = Some(fc(61, true, [61, 3, 3, 2, 1], 5));
        s(&mut r, "weather-fahrenheit", &m);
        // Changing the place keeps the old one to fall back on.
        m.choosing = true;
        s(&mut r, "weather-change", &m);
    }
    contact_sheet(&dir);
}

/// Focus and breaks: the three nudges, the finished-focus card with its summary, and the quiet
/// hairline on a bare pill (and the plain timer's fuse beside it for comparison).
#[test]
#[ignore]
fn focus_snapshots() {
    use crate::breaks::Kind;
    use crate::timer::{Timer, TimerKind};
    use crate::ui_state::Nudge;
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let wall = crate::timer::now_ms();
    let now = Instant::now();
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        let mut m = model();
        m.now = Some(now);
        for (name, kind, ago) in [
            ("nudge-eyes", Kind::Eyes, 6),
            ("nudge-eyes-late", Kind::Eyes, 17),
            ("nudge-water", Kind::Water, 1),
            ("nudge-stretch", Kind::Stretch, 1),
        ] {
            m.nudge = Some(Nudge {
                kind,
                since: now - std::time::Duration::from_secs(ago),
            });
            shoot(
                &mut r,
                &dir,
                &format!("{name}{sfx}"),
                &m,
                Scene::ExpNudge,
                light,
            );
        }
        m.nudge = None;
        // Focus finished, with and without what was held back.
        let mut t = Timer::start(TimerKind::Focus, 25, wall - 25 * 60_000);
        t.ends_ms = wall - 9 * 60_000;
        m.timer_last = Some(t);
        m.timer_done = Some((TimerKind::Focus, now));
        m.timer_break = 5;
        shoot(
            &mut r,
            &dir,
            &format!("focus-done{sfx}"),
            &m,
            Scene::ExpTimerDone,
            light,
        );
        m.focus_summary = Some("While you focused: 2 downloads, 1 task failed".into());
        shoot(
            &mut r,
            &dir,
            &format!("focus-done-summary{sfx}"),
            &m,
            Scene::ExpTimerDone,
            light,
        );
        m.timer_done = None;
        m.focus_summary = None;
        // The line on a bare pill: a focus session (quiet) and a plain timer (the fuse).
        m.timer = Some(Timer::start(TimerKind::Focus, 25, wall - 10 * 60_000));
        shoot(
            &mut r,
            &dir,
            &format!("line-focus{sfx}"),
            &m,
            Scene::Idle,
            light,
        );
        m.timer = Some(Timer::start(TimerKind::Focus, 25, wall - 24 * 60_000));
        shoot(
            &mut r,
            &dir,
            &format!("line-focus-last-minute{sfx}"),
            &m,
            Scene::Idle,
            light,
        );
        m.timer = Some(Timer::start(TimerKind::Plain, 25, wall - 10 * 60_000));
        shoot(
            &mut r,
            &dir,
            &format!("line-plain{sfx}"),
            &m,
            Scene::Idle,
            light,
        );
    }
    contact_sheet(&dir);
}
