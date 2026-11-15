use super::*;

#[test]
fn timer_fires_once_and_card_expires() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.timer = Some(crate::timer::Timer::start(
        crate::timer::TimerKind::Focus,
        1,
        1_000,
    ));
    assert_eq!(m.finish_timer(30_000, t0), None, "not due yet");
    assert_eq!(
        m.finish_timer(61_000, t0),
        Some(crate::timer::TimerKind::Focus)
    );
    assert_eq!((m.focus_done, m.timer.is_none()), (1, true));
    assert_eq!(m.finish_timer(99_000, t0), None, "only fires once");
    assert_eq!(m.scene(false, false), Scene::ExpTimerDone);
    m.expire(t0 + TIMER_DONE_HOLD);
    assert_eq!(m.scene(false, false), Scene::Idle);
}

#[test]
fn muted_mic_widens_idle_and_glows_red() {
    let mut m = Model::default();
    let w0 = m.size(Scene::Idle).0;
    m.mic_muted = true;
    assert_eq!(m.size(Scene::Idle).0, w0 + LOCK_W as f64);
    assert_eq!(m.glow(Scene::Idle).1, 0.8);
}

#[test]
fn idle_frames_stop_for_settled_failures_and_sentinel() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(st("a", TaskEvent::Failed, t0), t0);
    assert!(m.ambient(Scene::CompactTask, t0), "fresh failure pulses");
    let later = t0 + FAIL_PULSE;
    assert!(!m.ambient(Scene::CompactTask, later), "then rests");
    assert_eq!(m.tasks[0].fail_pulse(later), 0.0);
    m.apply_task(st("b", TaskEvent::Start, t0), t0);
    assert!(
        m.ambient(Scene::CompactTask, later),
        "running work still animates"
    );
    assert!(
        !m.ambient(Scene::Sentinel, later),
        "the sentinel bar is static"
    );
}

#[test]
fn primary_is_first_row() {
    let t0 = Instant::now();
    let mut m = Model::default();
    // Same rank and timestamp: the first inserted wins in both.
    m.apply_task(st("x", TaskEvent::Start, t0), t0);
    m.apply_task(st("y", TaskEvent::Start, t0), t0);
    assert_eq!(m.primary().unwrap().id, m.rows()[0].id);
    m.apply_task(st("z", TaskEvent::Failed, t0), t0 + Duration::from_secs(1));
    assert_eq!(m.primary().unwrap().id, "z");
    assert_eq!(m.primary().unwrap().id, m.rows()[0].id);
}

#[test]
fn unplugging_shows_the_card_the_other_way() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.set_power(Some(battery(true)), t0);
    m.set_power(Some(battery(false)), t0);
    assert_eq!(m.charge_since, Some(t0));
    assert!(!m.charge_in);
    m.expire(t0);
    assert_eq!(m.scene(false, false), Scene::ExpCharge);
    assert_eq!(m.expire(t0 + CHARGE_HOLD), None);
    // Plugging back in flips the direction.
    let t1 = t0 + CHARGE_HOLD;
    m.set_power(Some(battery(true)), t1);
    assert!(m.charge_in);
}

#[test]
fn plugging_in_shows_the_charge_card_briefly() {
    let t0 = Instant::now();
    let mut m = Model::default();
    // The first reading, or staying on one source, never triggers it.
    m.set_power(Some(battery(false)), t0);
    m.set_power(Some(battery(false)), t0);
    assert_eq!(m.charge_since, None);

    m.set_power(Some(battery(true)), t0);
    assert_eq!(m.charge_since, Some(t0));
    m.expire(t0);
    assert_eq!(m.scene(false, false), Scene::ExpCharge);
    // Hovering still opens the real panel (home), and a charge card is not interactive state.
    assert_eq!(m.scene(true, false), Scene::ExpHome);
    assert!(m.ambient(Scene::ExpCharge, t0));
    // The single expiry timer is armed for exactly the end of the card.
    assert_eq!(m.expire(t0), Some(t0 + CHARGE_HOLD));
    assert_eq!(m.expire(t0 + CHARGE_HOLD), None);
    assert_eq!(m.scene(false, false), Scene::Idle);
}

#[test]
fn paging_direction_belongs_to_one_scene_change() {
    let mut a = Anim::new(100.0, 24.0);
    let size = (380.0, 100.0);
    let glow = ([1.0; 3], 0.0);
    a.nav = 1.0;
    a.set_scene(Scene::ExpMedia, size, glow);
    assert_eq!((a.slide, a.nav), (1.0, 0.0));
    // Re-laying out the same scene keeps the side it slid in from.
    a.set_scene(Scene::ExpMedia, size, glow);
    assert_eq!(a.slide, 1.0);
    // A change nobody paged for (a peek) goes back to the plain drift.
    a.set_scene(Scene::ExpHome, size, glow);
    assert_eq!(a.slide, 0.0);
}

#[test]
fn squash_adds_motion_unless_reduced() {
    let mut a = Anim::new(100.0, 24.0);
    a.squash();
    assert!(a.rect.w.vel > 0.0 && a.rect.h.vel < 0.0);
    let mut b = Anim::new(100.0, 24.0);
    b.reduce = true;
    b.squash();
    assert_eq!((b.rect.w.vel, b.rect.h.vel), (0.0, 0.0));
}

#[test]
fn accent_follows_the_colour_and_ignores_grey() {
    // Premultiplied BGRA, fully opaque: a red picture and a grey one.
    let red: Vec<u8> = (0..16).flat_map(|_| [20u8, 30, 220, 255]).collect();
    let c = accent_of(&red).expect("red has a hue");
    assert!(c[0] > 0.9 && c[1] < 0.3 && c[2] < 0.3, "{c:?}");
    let grey: Vec<u8> = (0..16).flat_map(|_| [128u8, 128, 128, 255]).collect();
    assert_eq!(accent_of(&grey), None);
    let black: Vec<u8> = (0..16).flat_map(|_| [0u8, 0, 0, 255]).collect();
    assert_eq!(accent_of(&black), None);
}

#[test]
fn media_glow_takes_the_art_colour_only_when_asked() {
    let mut m = Model::default();
    m.art_accent = Some([0.9, 0.2, 0.2]);
    assert_eq!(m.glow(Scene::ExpMedia).0, [1.0; 3]);
    m.adaptive_glow = true;
    assert_eq!(m.glow(Scene::ExpMedia), ([0.9, 0.2, 0.2], 0.40));
    // Other scenes keep their own colours.
    assert_eq!(m.glow(Scene::ExpHome).0, [1.0; 3]);
}

#[test]
fn output_line_is_kept_between_updates_and_adds_a_row_line() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(st("a", TaskEvent::Start, t0), t0);
    let plain = m.size(Scene::ExpTasks).1;
    let with_line = |line: Option<&str>| {
        let mut u = st("a", TaskEvent::Progress, t0);
        if let TaskUpdate::Upsert(s) = &mut u {
            s.line = line.map(str::to_owned);
        }
        u
    };
    m.apply_task(with_line(Some("compiling foo")), t0);
    assert_eq!(m.tasks[0].line.as_deref(), Some("compiling foo"));
    assert_eq!(m.size(Scene::ExpTasks).1, plain + LINE_H as f64);
    // A progress update without a line doesn't blank the ticker.
    m.apply_task(with_line(None), t0);
    assert_eq!(m.tasks[0].line.as_deref(), Some("compiling foo"));
    // A finished task has no ticker.
    m.apply_task(st("a", TaskEvent::Done, t0), t0);
    assert!(!m.tasks[0].shows_line());
}

#[test]
fn a_second_activity_splits_off_as_a_bubble() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(st("a", TaskEvent::Start, t0), t0);
    assert_eq!(m.bubble(Scene::CompactTask), None);

    // Music playing beside a task.
    m.media = Some(MediaInfo {
        album: String::new(),
        title: "t".into(),
        artist: "a".into(),
        playing: true,
        app: String::new(),
        pos_ms: 0,
        dur_ms: 0,
    });
    assert_eq!(m.bubble(Scene::CompactTask), Some(Bubble::Media));
    // The timer outranks the music, and also joins compact media.
    m.timer = Some(crate::timer::Timer::start(
        crate::timer::TimerKind::Plain,
        5,
        0,
    ));
    assert_eq!(m.bubble(Scene::CompactTask), Some(Bubble::Timer));
    assert_eq!(m.bubble(Scene::CompactMedia), Some(Bubble::Timer));
    // Expanded scenes show these as panels instead.
    assert_eq!(m.bubble(Scene::ExpTasks), None);
    assert_eq!(m.bubble(Scene::Idle), None);
}

#[test]
fn a_bubble_opens_its_own_pane_and_sits_right_of_the_pill() {
    assert_eq!(Bubble::Timer.panel(), Panel::Timer);
    assert_eq!(Bubble::Media.panel(), Panel::Media);
    // Out (s = 1) it starts 8 px past the pill's right edge; at 0 it is tucked behind it.
    assert_eq!(Bubble::Timer.x(292.0, 1.0), 292.0 + 8.0);
    assert!(Bubble::Media.x(292.0, 0.0) < 292.0);
}

#[test]
fn bubble_room_only_while_it_is_out() {
    let mut a = Anim::new(100.0, 24.0);
    assert_eq!(a.bubble_room(), 0.0);
    a.set_bubble(Some(Bubble::Timer));
    a.step(0.05);
    assert_eq!(a.bubble_room(), BUBBLE_ROOM);
    // The kind survives while it retracts.
    a.set_bubble(None);
    assert_eq!(a.bubble_kind, Some(Bubble::Timer));
}

#[test]
fn a_scene_brought_back_while_leaving_continues_from_where_it_was() {
    let (size, glow) = ((380.0, 100.0), ([1.0; 3], 0.1));
    let mut a = Anim::new(380.0, 100.0);
    a.set_scene(Scene::ExpHome, size, glow);
    while a.step(1.0 / 120.0) {}
    a.set_scene(Scene::ExpMedia, size, glow);
    for _ in 0..8 {
        a.step(1.0 / 120.0);
    }
    let home = |a: &Anim| {
        a.shown()
            .iter()
            .find(|s| s.scene == Scene::ExpHome)
            .map_or(0.0, |s| s.alpha)
    };
    let before = home(&a);
    assert!(before > 0.2 && before < 0.95, "half faded: {before}");
    a.set_scene(Scene::ExpHome, size, glow);
    assert_eq!(home(&a), before, "the same layer, not a fresh one");
    a.step(1.0 / 120.0);
    assert!((home(&a) - before).abs() < 0.12);
    while a.step(1.0 / 120.0) {}
    assert_eq!(home(&a), 1.0);
    assert_eq!(a.shown().iter().count(), 1, "media has gone");
}

#[test]
fn quick_changes_keep_every_leaving_scene_fading_and_drop_only_the_faintest() {
    let (size, glow) = ((380.0, 100.0), ([1.0; 3], 0.1));
    let mut a = Anim::new(380.0, 100.0);
    a.set_scene(Scene::ExpHome, size, glow);
    while a.step(1.0 / 120.0) {}
    for s in [Scene::ExpMedia, Scene::ExpTasks] {
        a.set_scene(s, size, glow);
        a.step(1.0 / 120.0);
    }
    let scenes: Vec<Scene> = a.shown().iter().map(|s| s.scene).collect();
    assert_eq!(
        scenes,
        [Scene::ExpHome, Scene::ExpMedia, Scene::ExpTasks],
        "home still fading under the two after it"
    );
    // A fourth: the faintest leaving one (media, barely in) makes room.
    a.set_scene(Scene::ExpWeather, size, glow);
    let scenes: Vec<Scene> = a.shown().iter().map(|s| s.scene).collect();
    assert_eq!(scenes.last(), Some(&Scene::ExpWeather));
    assert!(!scenes.contains(&Scene::ExpMedia), "{scenes:?}");
    assert!(scenes.contains(&Scene::ExpHome), "{scenes:?}");
}

#[test]
fn reduced_motion_leaves_only_the_current_scene() {
    let (size, glow) = ((380.0, 100.0), ([1.0; 3], 0.1));
    let mut a = Anim::new(380.0, 100.0);
    a.set_scene(Scene::ExpHome, size, glow);
    a.reduce = true;
    a.set_scene(Scene::ExpMedia, size, glow);
    let shown: Vec<_> = a.shown().iter().copied().collect();
    assert_eq!(shown.len(), 1);
    assert_eq!((shown[0].scene, shown[0].alpha), (Scene::ExpMedia, 1.0));
    assert_eq!((shown[0].dx, shown[0].dy), (0.0, 0.0));
}

#[test]
fn the_glow_colour_blends_and_stays_a_colour() {
    let size = (300.0, 32.0);
    let (amber, red) = ([1.0, 0.7, 0.1], [1.0, 0.25, 0.25]);
    let mut a = Anim::new(300.0, 32.0);
    // Unseen, the colour doesn't blend: the glow comes up in amber.
    a.set_scene(Scene::CompactTask, size, (amber, 1.0));
    assert_eq!(a.glow_rgb, amber);
    while a.step(1.0 / 120.0) {}
    a.set_scene(Scene::ExpTasks, size, (red, 0.9));
    let mut between = false;
    while a.step(1.0 / 120.0) {
        let g = a.glow_rgb[1];
        assert!(a.glow_rgb.iter().all(|c| (0.0..=1.0).contains(c)));
        assert!((0.25..=0.7).contains(&g), "green {g} stays between the two");
        between |= g > 0.3 && g < 0.65;
    }
    assert!(between, "it passed through the colours in between");
    assert!((a.glow_rgb[1] - 0.25).abs() < 0.01);
}
