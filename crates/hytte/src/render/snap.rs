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
    let (w, h, px) = render_crop(r, m, scene, light, at);
    // Premultiplied BGRA over the wallpaper, as an RGB PNG.
    let bg = if light {
        [226.0, 230.0, 236.0]
    } else {
        [46.0, 52.0, 58.0]
    };
    let img = image::RgbImage::from_fn(w as u32, h as u32, |x, y| {
        let i = ((y * w as u32 + x) * 4) as usize;
        let a = px[i + 3] as f32 / 255.0;
        // BGRA -> RGB
        let ch = |c: usize, b: f32| (px[i + c] as f32 + b * (1.0 - a)).min(255.0) as u8;
        image::Rgb([ch(2, bg[0]), ch(1, bg[1]), ch(0, bg[2])])
    });
    img.save(dir.join(format!("{name}.png"))).unwrap();
    SHOT.with(|v| v.borrow_mut().push(name.to_string()));
}

/// Draw one settled scene and read back the pill's box (with its margin and glow) as
/// premultiplied BGRA rows: (width, height, pixels).
fn render_crop(
    r: &mut Renderer,
    m: &Model,
    scene: Scene,
    light: bool,
    at: Option<(f32, f32)>,
) -> (i32, i32, Vec<u8>) {
    let mut an = Anim::new(120.0, 24.0);
    an.set_scene(scene, m.size(scene), m.glow(scene));
    for _ in 0..240 {
        an.step(1.0 / 60.0);
    }
    let fr = Frame {
        model: m,
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
    let all = r.read_back();
    let cw = crop.w_px;
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let i = ((y * cw + x0) * 4) as usize;
        px.extend_from_slice(&all[i..i + (w * 4) as usize]);
    }
    (w, h, px)
}

/// Frames for the landing page (`landing-page/public/frames`): the scene at 3x on a
/// transparent canvas, straight-alpha RGBA, as its `FramePill` expects. Prints each frame's
/// `[width, height, last painted row]` for the `FRAMES` table.
#[test]
#[ignore]
fn landing_frames() {
    use crate::config::ScriptPane;
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(3.0).unwrap();
    let now = Instant::now();
    let save = |r: &mut Renderer, name: &str, m: &Model, scene: Scene| {
        let (w, h, px) = render_crop(r, m, scene, false, None);
        let img = image::RgbaImage::from_fn(w as u32, h as u32, |x, y| {
            let i = ((y * w as u32 + x) * 4) as usize;
            let a = px[i + 3];
            let un = |c: u8| {
                if a == 0 {
                    0
                } else {
                    (c as u32 * 255 / a as u32).min(255) as u8
                }
            };
            image::Rgba([un(px[i + 2]), un(px[i + 1]), un(px[i]), a])
        });
        // One past the last row painted at all down the middle (the glow's end), as the
        // frames already on the site count it.
        let mid = (w / 2) as u32;
        let last = (0..h as u32)
            .rev()
            .find(|y| img.get_pixel(mid, *y)[3] > 0)
            .map_or(0, |y| y + 1);
        img.save(dir.join(format!("{name}.png"))).unwrap();
        println!("\"{name}\": [{w}, {h}, {last}],");
    };
    let mut m = model();
    let c = hytte_proto::Cmd {
        id: Some("deploy".into()),
        title: Some("Deploy".into()),
        text: Some("Uploading assets…".into()),
        progress: Some(62),
        ..hytte_proto::Cmd::card()
    };
    m.card(&c, now);
    m.show_card(now);
    save(&mut r, "card", &m, Scene::ExpChip);
    let mut m = model();
    m.panes = Config::default().panes;
    m.apply_scripts(&[ScriptPane {
        name: "Server".into(),
        command: "curl -s localhost:3000/health".into(),
        every: 300,
        ..Default::default()
    }]);
    m.script_text(
        "Server",
        "● up  ·  v2.0.0\n3 users online\np95 142 ms",
        None,
        now,
    );
    m.selected = Some(Panel::Script(0));
    save(&mut r, "script-pane", &m, Scene::ExpScript(0));
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
        for row in sec.rows {
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
        for row in sec.rows {
            let Some(Action::Set(k, v)) = row.set_action() else {
                continue;
            };
            assert_eq!(k, row.key());
            let text = crate::config::with_value("", k, v).expect(k);
            let back = crate::config::parse(&text).expect(k);
            let again = settings_table(&back)
                .into_iter()
                .flat_map(|s| s.rows)
                .find(|r| r.key() == k)
                .unwrap();
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
        album: String::new(),
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
            agent: Default::default(),
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
        album: String::new(),
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
        // Nothing found for two seconds: the wider search is offered, then answers.
        m.places_err = false;
        m.place_edit.set("sri city");
        m.places_for = "sri city".into();
        m.wide_offer = true;
        s(&mut r, "weather-ask-wide-offer", &m);
        m.wide_offer = false;
        m.wide_busy = true;
        s(&mut r, "weather-ask-wide-busy", &m);
        m.wide_busy = false;
        m.places = vec![place("Sri City", "Andhra Pradesh, India")];
        m.places_wide = true;
        s(&mut r, "weather-ask-wide-found", &m);
        m.places.clear();
        m.places_wide = false;
        m.wide_for = "sri city".into();
        s(&mut r, "weather-ask-wide-none", &m);
        m.wide_for.clear();
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

/// The Media pane with the sleep timer and the lyrics page in every state. The song and its
/// lines are invented.
#[test]
#[ignore]
fn media_snapshots() {
    use crate::lyrics::{Line, Lyrics};
    use crate::sleep::{Kind, Sleep};
    use crate::ui_state::LyricState;
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let now = Instant::now();
    let synced = Lyrics::Synced(
        [
            (0u64, "We left the boats to find their way"),
            (6_000, "The harbour lights are coming on"),
            (
                12_000,
                "And nobody is counting the hours we have been gone, nobody is counting at all",
            ),
            (20_000, ""),
        ]
        .into_iter()
        .map(|(at_ms, t)| Line {
            at_ms,
            text: t.into(),
        })
        .collect(),
    );
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        let mut m = model();
        m.now = Some(now);
        m.media_stamp = Some(now);
        m.media = Some(MediaInfo {
            album: "Harbour".into(),
            title: "Low Tide".into(),
            artist: "The Lamplighters".into(),
            playing: true,
            app: "Spotify.exe".into(),
            pos_ms: 6_900,
            dur_ms: 214_000,
        });
        let s = |r: &mut Renderer, n: &str, m: &Model| {
            shoot(r, &dir, &format!("{n}{sfx}"), m, Scene::ExpMedia, light)
        };
        s(&mut r, "media", &m);
        m.sleep = Some(Sleep::new(
            Kind::Minutes(28),
            m.media.as_ref(),
            m.media_stamp,
            now,
        ));
        s(&mut r, "media-sleep-on", &m);
        m.sleep_open = true;
        s(&mut r, "media-sleep-menu-set", &m);
        m.sleep = None;
        s(&mut r, "media-sleep-menu", &m);
        m.sleep_open = false;
        m.sleep = Some(Sleep::new(
            Kind::EndOfTrack,
            m.media.as_ref(),
            m.media_stamp,
            now,
        ));
        s(&mut r, "media-sleep-end", &m);
        m.sleep = None;
        m.lyrics_open = true;
        s(&mut r, "lyrics-off", &m);
        m.lyrics_on = true;
        s(&mut r, "lyrics-idle", &m);
        m.lyric_state = LyricState::Loading;
        s(&mut r, "lyrics-loading", &m);
        m.lyric_state = LyricState::Failed;
        s(&mut r, "lyrics-failed", &m);
        m.lyric_state = LyricState::Ready(Lyrics::NotFound);
        s(&mut r, "lyrics-none", &m);
        m.lyric_state = LyricState::Ready(Lyrics::Instrumental);
        s(&mut r, "lyrics-instrumental", &m);
        m.lyric_state = LyricState::Ready(synced.clone());
        s(&mut r, "lyrics-synced", &m);
        m.media.as_mut().unwrap().pos_ms = 13_000;
        s(&mut r, "lyrics-long-line", &m);
        m.media.as_mut().unwrap().pos_ms = 21_000;
        s(&mut r, "lyrics-pause", &m);
        // Gliding to the next line (a quarter second after it starts), and "Live" unticked.
        m.media.as_mut().unwrap().pos_ms = 6_250;
        s(&mut r, "lyrics-slide", &m);
        m.lyrics_still = true;
        s(&mut r, "lyrics-not-live", &m);
        m.lyrics_still = false;
        m.media.as_mut().unwrap().pos_ms = 1_000;
        m.lyric_state = LyricState::Ready(Lyrics::Plain(
            [
                "First made-up line",
                "Second made-up line",
                "Third made-up line",
                "Fourth",
            ]
            .map(String::from)
            .to_vec(),
        ));
        s(&mut r, "lyrics-plain", &m);
        m.sleep = Some(Sleep::new(
            Kind::Minutes(15),
            m.media.as_ref(),
            m.media_stamp,
            now,
        ));
        s(&mut r, "lyrics-sleep-on", &m);
    }
    contact_sheet(&dir);
}

/// Every skin on Home, Media, Notes and a bare pill, dark and light.
#[test]
#[ignore]
fn skin_snapshots() {
    use crate::jot::Kind;
    use crate::skin::Skin;
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let wall = crate::timer::now_ms();
    for light in [false, true] {
        for skin in Skin::ALL {
            let mut m = model();
            m.now = Some(Instant::now());
            m.skin = skin;
            m.media_stamp = m.now;
            m.media = Some(MediaInfo {
                album: String::new(),
                title: "Low Tide".into(),
                artist: "The Lamplighters".into(),
                playing: true,
                app: "x".into(),
                pos_ms: 70_000,
                dur_ms: 214_000,
            });
            m.init_jot(&std::path::PathBuf::from("Z:\\none\\jot.json"));
            m.jot.add(
                Kind::Note,
                "Wi-Fi code at the studio: 4471",
                false,
                wall - 7 * 3_600_000,
            );
            m.jot
                .add(Kind::Note, "Passport renewal needs two photos", true, wall);
            let n = format!("{}{}", skin.name(), if light { "-light" } else { "" });
            shoot(
                &mut r,
                &dir,
                &format!("skin-home-{n}"),
                &m,
                Scene::ExpHome,
                light,
            );
            shoot(
                &mut r,
                &dir,
                &format!("skin-media-{n}"),
                &m,
                Scene::ExpMedia,
                light,
            );
            shoot(
                &mut r,
                &dir,
                &format!("skin-notes-{n}"),
                &m,
                Scene::ExpNotes,
                light,
            );
            m.timer = Some(crate::timer::Timer::start(
                crate::timer::TimerKind::Focus,
                25,
                wall - 10 * 60_000,
            ));
            m.media = None;
            shoot(
                &mut r,
                &dir,
                &format!("skin-idle-{n}"),
                &m,
                Scene::Idle,
                light,
            );
        }
    }
    contact_sheet(&dir);
}

/// Live cards (`hytte card`): plain, progress, done, buttons, error, long text.
#[test]
#[ignore]
fn card_snapshots() {
    use hytte_proto::Cmd;
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let now = Instant::now();
    let s = |v: &str| Some(v.to_string());
    let cases: Vec<(&str, Cmd)> = vec![
        ("card-plain", Cmd { title: s("Backup finished"), ..Cmd::card() }),
        (
            "card-progress",
            Cmd {
                title: s("Deploy"),
                text: s("Uploading assets…"),
                progress: Some(40),
                ..Cmd::card()
            },
        ),
        (
            "card-done",
            Cmd {
                title: s("Deploy"),
                text: s("Live at hytte.app"),
                progress: Some(40),
                done: true,
                open: s("https://example.com"),
                ..Cmd::card()
            },
        ),
        (
            "card-buttons",
            Cmd {
                title: s("Restart the dev server?"),
                text: s("package.json changed"),
                buttons: vec!["Restart".into(), "Later".into()],
                ..Cmd::card()
            },
        ),
        (
            "card-error",
            Cmd {
                title: s("CI failed"),
                text: s("3 tests failed on main"),
                icon: s("error"),
                open: s("https://example.com"),
                ..Cmd::card()
            },
        ),
        (
            "card-long",
            Cmd {
                title: s("Nightly report"),
                text: s("All 1,204 checks passed, the bundle grew by 3 KB and the slowest page now loads in 1.4 s on a cold cache"),
                progress: Some(75),
                ..Cmd::card()
            },
        ),
    ];
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        for (name, c) in &cases {
            let mut m = model();
            m.card(c, now);
            m.show_card(now);
            shoot(
                &mut r,
                &dir,
                &format!("{name}{sfx}"),
                &m,
                Scene::ExpChip,
                light,
            );
        }
    }
    contact_sheet(&dir);
}

/// Script panes: output with the tab strip, a failure, push-only and running empties, a long
/// output, and the settings window's rows for them.
#[test]
#[ignore]
fn script_snapshots() {
    use crate::config::ScriptPane;
    use crate::scriptpane::Run;
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let now = Instant::now();
    let panes = [
        ScriptPane {
            name: "Server".into(),
            command: "curl -s localhost:3000/health".into(),
            every: 300,
            ..Default::default()
        },
        ScriptPane {
            name: "Deploys".into(),
            command: String::new(),
            every: 300,
            ..Default::default()
        },
    ];
    let fresh = || {
        let mut m = model();
        m.panes = Config::default().panes;
        m.apply_scripts(&panes);
        m
    };
    let ok = Run {
        text: "\x1b[32m● up\x1b[0m  ·  v2.0.0\n3 users online\np95 142 ms".into(),
        code: Some(0),
        ..Default::default()
    };
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        let mut m = fresh();
        m.script_done(
            "Server",
            ok.clone(),
            now - std::time::Duration::from_secs(190),
        );
        m.selected = Some(Panel::Script(0));
        shoot(
            &mut r,
            &dir,
            &format!("script-output{sfx}"),
            &m,
            Scene::ExpScript(0),
            light,
        );
        m.script_done(
            "Server",
            Run {
                text: "curl: (7) Failed to connect to localhost port 3000".into(),
                code: Some(7),
                ..Default::default()
            },
            now,
        );
        shoot(
            &mut r,
            &dir,
            &format!("script-failed{sfx}"),
            &m,
            Scene::ExpScript(0),
            light,
        );
        let mut m = fresh();
        m.selected = Some(Panel::Script(1));
        shoot(
            &mut r,
            &dir,
            &format!("script-push-empty{sfx}"),
            &m,
            Scene::ExpScript(1),
            light,
        );
        m.script_text(
            "Deploys",
            "web  v2.0.0  live\napi  v1.9.3  rolling 60%",
            None,
            now,
        );
        shoot(
            &mut r,
            &dir,
            &format!("script-pushed{sfx}"),
            &m,
            Scene::ExpScript(1),
            light,
        );
        let mut m = fresh();
        m.scripts[0].busy = true;
        m.selected = Some(Panel::Script(0));
        shoot(
            &mut r,
            &dir,
            &format!("script-running{sfx}"),
            &m,
            Scene::ExpScript(0),
            light,
        );
        let long: String = (1..=12)
            .map(|i| {
                format!(
                    "job {i:02}  passed in {} s, a long line that runs past the edge\n",
                    i * 3
                )
            })
            .collect();
        m.script_done(
            "Server",
            Run {
                text: long,
                code: Some(0),
                ..Default::default()
            },
            now,
        );
        shoot(
            &mut r,
            &dir,
            &format!("script-long{sfx}"),
            &m,
            Scene::ExpScript(0),
            light,
        );
    }
    // The settings window's rows for the two panes (search narrows the list to them).
    let mut cfg = Config::default();
    cfg.script_pane = panes.to_vec();
    cfg.panes.script2 = false;
    let mut r = Renderer::offscreen_sized(1720, 900, 2.0).unwrap();
    let view = SettingsView {
        mouse: None,
        pressed: false,
        light: false,
        accent: BLUE,
        scroll: 0.0,
        scrolling: false,
        dt: 0.016,
        open: 1.0,
        toast: None,
        query: "script",
    };
    let mut hits = vec![];
    r.draw_settings_window(&cfg, &view, &mut hits);
    r.draw_settings_window(&cfg, &view, &mut hits);
    let (pw, ph) = r.px;
    let px = r.read_back();
    image::RgbImage::from_fn(pw as u32, ph as u32, |x, y| {
        let i = ((y as i32 * pw + x as i32) * 4) as usize;
        image::Rgb([px[i + 2], px[i + 1], px[i]])
    })
    .save(dir.join("script-settings.png"))
    .unwrap();
    SHOT.with(|v| v.borrow_mut().push("script-settings".to_string()));
    contact_sheet(&dir);
}

/// The accent skin under several Windows accents, the skin discs, and the key hints.
#[test]
#[ignore]
fn accent_and_hint_snapshots() {
    use crate::skin::{set_accent, Skin};
    use crate::ui_state::Field;
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let base = || {
        let mut m = model();
        m.now = Some(Instant::now());
        m.panes = Config::default().panes;
        m.init_jot(&std::path::PathBuf::from(r"Z:\none\jot.json"));
        m
    };
    let accents = [
        ("blue", [0.0, 0.47, 0.84]),
        ("yellow", [1.0, 0.73, 0.0]),
        ("red", [0.91, 0.07, 0.14]),
        ("green", [0.06, 0.49, 0.06]),
        ("purple", [0.53, 0.09, 0.6]),
    ];
    for (name, a) in accents {
        set_accent(Some(a));
        for light in [false, true] {
            let sfx = if light { "-light" } else { "" };
            let mut m = base();
            m.skin = Skin::Accent;
            shoot(
                &mut r,
                &dir,
                &format!("accent-{name}-home{sfx}"),
                &m,
                Scene::ExpHome,
                light,
            );
            shoot(
                &mut r,
                &dir,
                &format!("accent-{name}-idle{sfx}"),
                &m,
                Scene::Idle,
                light,
            );
        }
    }
    set_accent(None);
    // Key hints: keyboard mode on Home, on Notes while typing, and over a card.
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        let mut m = base();
        m.keyboard = true;
        shoot(
            &mut r,
            &dir,
            &format!("hint-home{sfx}"),
            &m,
            Scene::ExpHome,
            light,
        );
        m.note_edit.set("Call the plumber");
        m.typing = Some(Field::Note);
        shoot(
            &mut r,
            &dir,
            &format!("hint-typing{sfx}"),
            &m,
            Scene::ExpNotes,
            light,
        );
        m.typing = None;
        let c = hytte_proto::Cmd {
            title: Some("Restart the dev server?".into()),
            buttons: vec!["Restart".into(), "Later".into()],
            ..hytte_proto::Cmd::card()
        };
        m.card(&c, Instant::now());
        m.show_card(Instant::now());
        shoot(
            &mut r,
            &dir,
            &format!("hint-card{sfx}"),
            &m,
            Scene::ExpChip,
            light,
        );
    }
    contact_sheet(&dir);
}

/// The welcome tour: all six cards (the intro's last one with and without a hotkey).
#[test]
#[ignore]
fn welcome_snapshots() {
    use crate::welcome::{chip, Tour, STEPS};
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let now = Instant::now();
    for light in [false, true] {
        let sfx = if light { "-light" } else { "" };
        for (tour, tag) in [(Tour::Intro, "intro"), (Tour::WhatsNew, "new")] {
            for step in 0..STEPS {
                let mut m = model();
                m.chip = Some(chip(tour, step, "", now));
                let name = format!("welcome-{tag}-{}{sfx}", step + 1);
                shoot(&mut r, &dir, &name, &m, Scene::ExpChip, light);
            }
        }
        let mut m = model();
        m.chip = Some(chip(Tour::Intro, STEPS - 1, "Ctrl+Alt+F9", now));
        let name = format!("welcome-intro-3-hotkey{sfx}");
        shoot(&mut r, &dir, &name, &m, Scene::ExpChip, light);
    }
    contact_sheet(&dir);
}

/// The Agents pane: one agent working, one waiting beside two working, long names.
#[test]
#[ignore]
fn agent_snapshots() {
    use crate::tasks::{AgentInfo, TaskState, TaskUpdate};
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let now = Instant::now();
    let agent = |id: &str,
                 label: &str,
                 project: &str,
                 ev: TaskEvent,
                 line: Option<&str>,
                 msg: Option<&str>,
                 info: AgentInfo| {
        TaskUpdate::Upsert(TaskState {
            task_id: id.into(),
            label: label.into(),
            progress: None,
            event: ev,
            duration_ms: Some(83_000),
            stderr_tail: None,
            source: "agent".into(),
            pid: Some(4242),
            delay_ms: 0,
            message: msg.map(Into::into),
            exit_code: None,
            line: line.map(Into::into),
            cwd: Some(format!(r"C:\src\{project}")),
            agent: info,
            updated: now,
        })
    };
    let busy = |files: &[&str], subagents: u32, context: u64| AgentInfo {
        file: files.first().map(|f| f.to_string()),
        subagent: subagents > 0,
        context: Some(context),
    };
    let t0 = now - std::time::Duration::from_secs(120);

    let mut m = model();
    m.now = Some(now);
    m.tasks.clear();
    m.panes.agents = true;
    m.apply_task(
        agent(
            "a",
            "Claude Code",
            "hytte",
            TaskEvent::Progress,
            Some("Editing render/agents.rs"),
            None,
            busy(&["render/agents.rs"], 0, 84_000),
        ),
        t0,
    );
    m.expire(now);
    shoot(&mut r, &dir, "agents-one", &m, Scene::ExpAgents, false);

    m.apply_task(
        agent(
            "b",
            "Codex",
            "billing-service",
            TaskEvent::NeedsInput,
            None,
            Some("Allow: git push origin main?"),
            Default::default(),
        ),
        now,
    );
    m.apply_task(
        agent(
            "c",
            "Gemini CLI",
            "docs",
            TaskEvent::Progress,
            Some("Running: cargo test"),
            None,
            busy(&[], 2, 1_250_000),
        ),
        t0,
    );
    m.expire(now);
    shoot(&mut r, &dir, "agents-three", &m, Scene::ExpAgents, false);
    shoot(
        &mut r,
        &dir,
        "agents-three-light",
        &m,
        Scene::ExpAgents,
        true,
    );

    let mut m = model();
    m.now = Some(now);
    m.tasks.clear();
    m.apply_task(
        agent(
            "d",
            "Claude Code",
            "a-very-long-project-folder-name-that-will-not-fit",
            TaskEvent::NeedsInput,
            None,
            Some("Allow: rm -rf node_modules && npm install --legacy-peer-deps --force"),
            busy(&["a.rs", "b.rs"], 1, 190_000),
        ),
        t0,
    );
    m.expire(now);
    shoot(&mut r, &dir, "agents-long", &m, Scene::ExpAgents, false);

    // The reply card of a finished turn, with what the agent did.
    let mut m = model();
    m.now = Some(now);
    m.finish_peek = std::time::Duration::from_secs(8);
    m.apply_task(
        agent(
            "e",
            "Claude Code",
            "hytte",
            TaskEvent::Progress,
            Some("Editing cards.rs"),
            None,
            busy(&["cards.rs"], 0, 60_000),
        ),
        t0,
    );
    m.apply_task(
        agent(
            "e",
            "Claude Code",
            "hytte",
            TaskEvent::Done,
            None,
            Some("The card queue is in; alerts wait their turn now."),
            busy(&[], 0, 84_000),
        ),
        now,
    );
    shoot(&mut r, &dir, "agent-done-card", &m, Scene::ExpChip, false);
    shoot(
        &mut r,
        &dir,
        "agent-done-card-light",
        &m,
        Scene::ExpChip,
        true,
    );
}

/// Script panes that show rows: values, progress, tones, a clickable row, an armed one.
#[test]
#[ignore]
fn script_row_snapshots() {
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let now = Instant::now();
    let mut m = model();
    m.now = Some(now);
    m.apply_scripts(&[crate::config::ScriptPane {
        name: "Stack".into(),
        command: String::new(),
        every: 300,
        ..Default::default()
    }]);
    m.panes.script1 = true;
    let json = r#"{"rows":[
        {"text":"api","value":"up 3d","tone":"good","action":"restart-api"},
        {"text":"worker","value":"2 failed","tone":"bad","progress":35},
        {"text":"db","value":"82% disk","tone":"warn","progress":82},
        {"text":"cache","value":"ok"},
        {"text":"Deploy to production with a very long name that gets cut off","value":"idle"}
    ]}"#;
    m.script_text("Stack", json, None, now);
    shoot(&mut r, &dir, "script-rows", &m, Scene::ExpScript(0), false);
    shoot(
        &mut r,
        &dir,
        "script-rows-light",
        &m,
        Scene::ExpScript(0),
        true,
    );
    m.armed_action = Some((
        "restart-api".into(),
        now + std::time::Duration::from_secs(3),
    ));
    shoot(
        &mut r,
        &dir,
        "script-rows-armed",
        &m,
        Scene::ExpScript(0),
        false,
    );

    // A card whose button runs an action.
    let mut m = model();
    m.now = Some(now);
    m.apply_actions(&[crate::config::NamedAction {
        name: "restart-api".into(),
        command: "x".into(),
        confirm: true,
        timeout: 60,
        ..Default::default()
    }]);
    let c = hytte_proto::Cmd {
        title: Some("api is down".into()),
        text: Some("health check failed 3 times".into()),
        buttons: vec!["Restart:restart-api".into(), "Later".into()],
        icon: Some("error".into()),
        ..hytte_proto::Cmd::card()
    };
    m.card(&c, now);
    m.show_card(now);
    shoot(&mut r, &dir, "card-action", &m, Scene::ExpChip, false);
    m.armed_action = Some((
        "restart-api".into(),
        now + std::time::Duration::from_secs(3),
    ));
    shoot(&mut r, &dir, "card-action-armed", &m, Scene::ExpChip, false);
}

/// The Shelf pane with the clipboard history: tiles and clips, clips only, a long list.
#[test]
#[ignore]
fn clip_snapshots() {
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Renderer::offscreen(2.0).unwrap();
    let now = Instant::now();
    let mut m = model();
    m.now = Some(now);
    m.panes.shelf = true;
    m.apply_clipboard(true);
    for t in [
        "cargo test --workspace -- --nocapture",
        "https://github.com/PrathamPai-2007/hytte/releases/tag/v1.7.0",
        "multi line\ntext that\nspans lines",
        r"C:\Users\prath\Desktop\projects\Hytte\crates\hytte\src\clipboard.rs",
    ] {
        m.add_clip(t);
    }
    shoot(&mut r, &dir, "clips-only", &m, Scene::ExpShelf, false);
    shoot(&mut r, &dir, "clips-only-light", &m, Scene::ExpShelf, true);
    for (i, name) in ["notes.txt", "photo.png", "report.pdf"].iter().enumerate() {
        m.shelf.push(crate::shelf::ShelfItem {
            id: i as u64 + 1,
            path: format!(r"C:\x\{name}").into(),
            name: (*name).into(),
            is_dir: false,
            owned: false,
        });
    }
    shoot(&mut r, &dir, "clips-and-tiles", &m, Scene::ExpShelf, false);
    for i in 0..14 {
        m.add_clip(&format!("clip number {i}"));
    }
    m.scroll_clips(2);
    shoot(&mut r, &dir, "clips-scrolled", &m, Scene::ExpShelf, false);
}
