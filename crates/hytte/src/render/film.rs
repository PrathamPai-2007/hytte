//! Motion review: scripted transitions driven with a fixed time step.
//!
//! - `HYTTE_SNAP_DIR=<dir> cargo test filmstrips -- --ignored` renders every script frame by
//!   frame and writes `motion.html`, which plays each strip at full speed, a quarter, or one
//!   frame at a time. Put an older run's folder at `<dir>/before` and each strip shows beside
//!   its "before".
//! - The continuity tests run the same scripts without drawing and fail if any scene's
//!   opacity or offset jumps between two frames.

use super::*;
use crate::ui_state::{Chip, MediaInfo, Shown};

/// The step the scripts are driven with (a 120 Hz display).
const STEP: f64 = 1.0 / 120.0;
/// Largest change in a scene's opacity between two 120 Hz frames that still reads as motion.
pub const MAX_ALPHA_STEP: f32 = 0.12;
/// Largest move of a scene's content (its slide or drift) between two 120 Hz frames.
pub const MAX_OFFSET_STEP: f32 = 6.0;

/// What happens at a moment of a script.
#[derive(Clone, Copy)]
enum Cue {
    /// The pill shows this scene.
    Show(Scene),
    /// The user pages to this scene (-1 previous, +1 next).
    Page(Scene, f32),
}

pub struct Script {
    pub name: &'static str,
    pub what: &'static str,
    /// The scene at the start, settled.
    start: Scene,
    cues: Vec<(f64, Cue)>,
    /// Seconds, all told.
    len: f64,
}

pub fn scripts() -> Vec<Script> {
    use Cue::*;
    vec![
        Script {
            name: "task-open-close",
            what: "A task starts (compact), the pointer opens the list, leaves, the task ends.",
            start: Scene::Idle,
            cues: vec![
                (0.10, Show(Scene::CompactTask)),
                (0.80, Show(Scene::ExpTasks)),
                (1.70, Show(Scene::CompactTask)),
                (2.40, Show(Scene::Idle)),
            ],
            len: 3.1,
        },
        Script {
            name: "media-morph",
            what: "Compact media opens into the media pane and folds back: the art travels.",
            start: Scene::CompactMedia,
            cues: vec![
                (0.10, Show(Scene::ExpMedia)),
                (1.00, Show(Scene::CompactMedia)),
            ],
            len: 1.8,
        },
        Script {
            name: "paging",
            what: "Paging Home, Media, Tasks, then back.",
            start: Scene::ExpHome,
            cues: vec![
                (0.10, Page(Scene::ExpMedia, 1.0)),
                (0.70, Page(Scene::ExpTasks, 1.0)),
                (1.30, Page(Scene::ExpMedia, -1.0)),
            ],
            len: 2.0,
        },
        Script {
            name: "reverse-aba",
            what: "Home to Media and straight back 80 ms later (the reversal must not pop).",
            start: Scene::ExpHome,
            cues: vec![(0.10, Show(Scene::ExpMedia)), (0.18, Show(Scene::ExpHome))],
            len: 0.9,
        },
        Script {
            name: "reverse-abc",
            what: "Home to Media to Tasks inside 80 ms (Home must keep fading, not vanish).",
            start: Scene::ExpHome,
            cues: vec![(0.10, Show(Scene::ExpMedia)), (0.18, Show(Scene::ExpTasks))],
            len: 0.9,
        },
        Script {
            name: "page-reverse",
            what: "A page to Media reversed halfway through its slide.",
            start: Scene::ExpHome,
            cues: vec![
                (0.10, Page(Scene::ExpMedia, 1.0)),
                (0.22, Page(Scene::ExpHome, -1.0)),
            ],
            len: 0.9,
        },
        Script {
            name: "card",
            what: "A card arrives over the compact task and leaves again.",
            start: Scene::CompactTask,
            cues: vec![
                (0.10, Show(Scene::ExpChip)),
                (1.20, Show(Scene::CompactTask)),
            ],
            len: 1.9,
        },
    ]
}

/// The moment every script starts at: frames are drawn at `epoch() + t`, so a strip is the same
/// pictures on every run (the media timeline and the spinner read the clock).
fn epoch() -> Instant {
    static E: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *E.get_or_init(Instant::now)
}

/// A model with something to show in every scene the scripts use.
pub fn film_model() -> Model {
    let now = epoch();
    let mut m = Model {
        glow_gain: 1.2,
        body_alpha: 1.0,
        ai: crate::config::Ai {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    m.now = Some(now);
    m.speaker = Some((0.64, false));
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
    m.apply_task(
        crate::tasks::TaskUpdate::Upsert(crate::tasks::TaskState {
            task_id: "a".into(),
            label: "cargo build --release".into(),
            progress: Some(62),
            event: TaskEvent::Progress,
            duration_ms: Some(83_000),
            stderr_tail: None,
            source: "run".into(),
            pid: None,
            delay_ms: 0,
            message: None,
            exit_code: None,
            line: Some("Compiling hytte v1.1.0".into()),
            cwd: Some(r"C:\src\app".into()),
            agent: Default::default(),
            updated: now,
        }),
        now - std::time::Duration::from_secs(90),
    );
    m.chip = Some(Chip::new("Saved invoice.pdf to Downloads", now));
    m
}

/// Drive `s` with the fixed step, calling `each` after every step with the time and the state.
pub fn run(s: &Script, m: &Model, mut each: impl FnMut(f64, &Anim)) {
    let (w, h) = m.size(s.start);
    let mut an = Anim::new(w, h);
    an.set_scene(s.start, m.size(s.start), m.glow(s.start));
    for _ in 0..240 {
        an.step(STEP);
    }
    let steps = (s.len / STEP).round() as usize;
    let mut cues = s.cues.iter().peekable();
    for i in 0..steps {
        let t = i as f64 * STEP;
        while let Some((at, cue)) = cues.peek() {
            if *at > t {
                break;
            }
            let scene = match *cue {
                Cue::Show(sc) => sc,
                Cue::Page(sc, dir) => {
                    an.nav = dir;
                    sc
                }
            };
            an.set_scene(scene, m.size(scene), m.glow(scene));
            an.set_bubble(m.bubble(scene));
            an.nav = 0.0;
            cues.next();
        }
        an.step(STEP);
        each(t, &an);
    }
}

/// The largest opacity and offset step any scene takes between two frames of `s`, and when.
pub fn continuity(s: &Script, m: &Model) -> (f32, f32, f64) {
    let mut prev: Option<Vec<Shown>> = None;
    let (mut da, mut dp, mut at) = (0.0f32, 0.0f32, 0.0);
    run(s, m, |t, an| {
        let now: Vec<Shown> = an.shown().iter().copied().collect();
        if let Some(before) = &prev {
            let alpha = |l: &[Shown], sc: Scene| {
                l.iter()
                    .filter(|x| x.scene == sc)
                    .map(|x| x.alpha)
                    .sum::<f32>()
            };
            for sc in before.iter().chain(&now).map(|x| x.scene) {
                let step = (alpha(&now, sc) - alpha(before, sc)).abs();
                if step > da {
                    (da, at) = (step, t);
                }
                let a = before.iter().find(|x| x.scene == sc && x.alpha > 0.01);
                let b = now.iter().find(|x| x.scene == sc && x.alpha > 0.01);
                if let (Some(a), Some(b)) = (a, b) {
                    dp = dp.max((a.dx - b.dx).hypot(a.dy - b.dy));
                }
            }
        }
        prev = Some(now);
    });
    (da, dp, at)
}

#[test]
fn every_scripted_transition_is_continuous() {
    let m = film_model();
    let mut bad = vec![];
    for s in scripts() {
        let (da, dp, at) = continuity(&s, &m);
        if da > MAX_ALPHA_STEP || dp > MAX_OFFSET_STEP {
            bad.push(format!(
                "{}: opacity step {da:.2}, offset step {dp:.1} px (at {at:.3} s)",
                s.name
            ));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Every strip, frame by frame, and `motion.html` to play them.
#[test]
#[ignore]
fn filmstrips() {
    let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
    let scale = 1.5;
    let m = film_model();
    let mut r = Renderer::offscreen(scale).unwrap();
    let mut strips = vec![];
    for s in scripts() {
        let sub = dir.join(s.name);
        let _ = std::fs::remove_dir_all(&sub);
        std::fs::create_dir_all(&sub).unwrap();
        // One picture per 60 Hz frame, cropped to the tallest the pill gets in this strip.
        let mut tallest = 0.0f32;
        run(&s, &m, |_, an| tallest = tallest.max(an.rect.h.pos as f32));
        let crop_h = (((tallest + 30.0) * scale).ceil() as u32).min(r.px.1 as u32);
        let mut n = 0;
        let mut i = 0usize;
        let (da, dp, _) = continuity(&s, &m);
        run(&s, &m, |t, an| {
            i += 1;
            if !i.is_multiple_of(2) {
                return;
            }
            let fr = Frame {
                model: &m,
                anim: an,
                now: epoch() + std::time::Duration::from_secs_f64(t),
                dt: (2.0 * STEP) as f32,
                mouse: None,
                pressed: false,
                light: false,
                accent: BLUE,
                focus: None,
                armed: false,
                ringing: false,
                caret_on: true,
            };
            let crop = r.plan(
                an.rect.w.pos as f32,
                an.rect.h.pos as f32,
                false,
                an.bubble_room(),
                true,
            );
            let mut hits = vec![];
            r.draw(&fr, crop, &mut hits);
            let px = r.read_back();
            let cw = r.px.0 as u32;
            let bg = [46.0, 52.0, 58.0];
            let img = image::RgbImage::from_fn(cw, crop_h, |x, y| {
                let k = ((y * cw + x) * 4) as usize;
                let a = px[k + 3] as f32 / 255.0;
                let ch = |c: usize, b: f32| (px[k + c] as f32 + b * (1.0 - a)).min(255.0) as u8;
                image::Rgb([ch(2, bg[0]), ch(1, bg[1]), ch(0, bg[2])])
            });
            img.save(sub.join(format!("{n:03}.png"))).unwrap();
            n += 1;
        });
        strips.push((s.name, s.what, n, da, dp));
    }
    let has_before = dir.join("before").is_dir();
    let mut body = String::new();
    for (name, what, n, da, dp) in &strips {
        let before = if has_before && dir.join("before").join(name).is_dir() {
            format!("<div><b>before</b><img data-strip=\"before/{name}\" data-n=\"{n}\"></div>")
        } else {
            String::new()
        };
        let flag = if *da > MAX_ALPHA_STEP || *dp > MAX_OFFSET_STEP {
            " class=bad"
        } else {
            ""
        };
        body.push_str(&format!(
            "<section><h2>{name}</h2><p>{what}</p><p{flag}>largest step: opacity {da:.2} (limit {MAX_ALPHA_STEP}), offset {dp:.1} px (limit {MAX_OFFSET_STEP})</p>\
             <div class=row>{before}<div><b>now</b><img data-strip=\"{name}\" data-n=\"{n}\"></div></div></section>\n"
        ));
    }
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Hytte motion</title>\n\
         <style>body{{background:#1b1e22;color:#ddd;font:14px system-ui;margin:24px}}\
         .row{{display:flex;gap:16px;flex-wrap:wrap}} img{{width:460px;display:block;border-radius:6px}}\
         .bad{{color:#ff7b72}} section{{margin-bottom:36px}} header{{position:sticky;top:0;background:#1b1e22;padding:8px 0}}\
         button{{margin-right:6px}}</style>\n\
         <header>Speed: <button data-s=1>1x</button><button data-s=0.25>0.25x</button><button data-s=0>step</button>\
         <button id=next>next frame</button> <span id=f></span></header>\n{body}\n\
         <script>\n\
         const imgs=[...document.querySelectorAll('img[data-strip]')];let speed=1,t=0,last=performance.now(),frame=0;\n\
         const max=Math.max(...imgs.map(i=>+i.dataset.n));\n\
         function show(){{for(const i of imgs){{const n=+i.dataset.n;const k=Math.min(frame%(max+30),n-1);i.src=i.dataset.strip+'/'+String(k).padStart(3,'0')+'.png'}}document.getElementById('f').textContent='frame '+(frame%(max+30))}}\n\
         function tick(now){{if(speed>0){{t+=(now-last)/1000*speed;const f=Math.floor(t*60);if(f!==frame){{frame=f;show()}}}}last=now;requestAnimationFrame(tick)}}\n\
         document.querySelectorAll('button[data-s]').forEach(b=>b.onclick=()=>{{speed=+b.dataset.s}});\n\
         document.getElementById('next').onclick=()=>{{speed=0;frame++;show()}};\n\
         show();requestAnimationFrame(tick);\n\
         </script>\n"
    );
    std::fs::write(dir.join("motion.html"), page).unwrap();
    for (name, _, n, da, dp) in &strips {
        println!("{name}: {n} frames, opacity step {da:.2}, offset step {dp:.1} px");
    }
}

/// Spike (Task 6): the glow as five strokes of the silhouette, against the same glow cut from a
/// cached mask in nine slices. Prints process CPU per draw.
#[test]
#[ignore]
fn glow_cost_spike() {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let cpu = || {
        let mut t = [FILETIME::default(); 4];
        let [a, b, k, u] = &mut t;
        unsafe { GetProcessTimes(GetCurrentProcess(), a, b, k, u).unwrap() };
        let f = |f: &FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64;
        (f(&t[2]) + f(&t[3])) / 1e4 // ms
    };
    for scale in [1.0f32, 2.0] {
        let r = Renderer::offscreen(scale).unwrap();
        let (w, h) = (380.0f32, 128.0f32);
        let g = r.pill_geometry(w, h, 24.0, 8.0).unwrap();
        let n = 300;
        let time = |f: &dyn Fn()| {
            unsafe {
                r.rt.BeginDraw();
                f();
                r.rt.EndDraw(None, None).unwrap();
            }
            let t0 = cpu();
            for _ in 0..n {
                unsafe {
                    r.rt.BeginDraw();
                    r.rt.SetTransform(&mat(scale, 40.0, 0.0));
                    f();
                    r.rt.EndDraw(None, None).unwrap();
                }
            }
            (cpu() - t0) / n as f64
        };
        let strokes = time(&|| unsafe {
            for j in 1..=5 {
                r.rt.DrawGeometry(&g, r.solid(color(BLUE, 0.1)), j as f32 * 4.4, &r.round);
            }
        });
        // The mask: the same strokes once, into an alpha bitmap the size of the glow.
        let (mw, mh) = (((w + 56.0) * scale) as u32, ((h + 28.0) * scale) as u32);
        let mask = unsafe {
            let t =
                r.rt.CreateCompatibleRenderTarget(
                    None,
                    Some(&D2D_SIZE_U {
                        width: mw,
                        height: mh,
                    }),
                    Some(&D2D1_PIXEL_FORMAT {
                        format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    }),
                    D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
                )
                .unwrap();
            let rt: ID2D1RenderTarget = t.cast().unwrap();
            rt.BeginDraw();
            rt.SetTransform(&mat(scale, 28.0, 0.0));
            for j in 1..=5 {
                rt.DrawGeometry(&g, r.solid(color(WHITE, 0.1)), j as f32 * 4.4, &r.round);
            }
            rt.EndDraw(None, None).unwrap();
            t.GetBitmap().unwrap()
        };
        let sliced = time(&|| unsafe {
            r.rt.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            let (cw, ch) = (mw as f32 / 3.0, mh as f32 / 3.0);
            for i in 0..3 {
                for j in 0..3 {
                    let src = D2D_RECT_F {
                        left: i as f32 * cw,
                        top: j as f32 * ch,
                        right: (i + 1) as f32 * cw,
                        bottom: (j + 1) as f32 * ch,
                    };
                    let dst = rect(
                        i as f32 * cw / scale,
                        j as f32 * ch / scale,
                        cw / scale * 1.1,
                        ch / scale * 1.1,
                    );
                    r.rt.FillOpacityMask(
                        &mask,
                        r.solid(color(BLUE, 1.0)),
                        D2D1_OPACITY_MASK_CONTENT_GRAPHICS,
                        Some(&dst),
                        Some(&src),
                    );
                }
            }
            r.rt.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        });
        let fill = time(&|| unsafe {
            r.rt.FillGeometry(&g, r.solid(color(BLUE, 0.9)), None::<&ID2D1Brush>);
        });
        println!(
            "scale {scale}: five strokes {strokes:.2} ms, nine mask slices {sliced:.2} ms, one body fill {fill:.2} ms (CPU per draw)"
        );
    }
}
