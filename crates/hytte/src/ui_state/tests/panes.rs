use super::*;

#[test]
fn panel_names_round_trip() {
    for p in [
        Panel::Tasks,
        Panel::Shelf,
        Panel::Media,
        Panel::Timer,
        Panel::Power,
        Panel::Home,
    ] {
        assert_eq!(Panel::from_name(p.name()), Some(p));
        assert_eq!(
            Panel::from_name(&format!(
                "{}
",
                p.name()
            )),
            Some(p)
        );
    }
    assert_eq!(Panel::from_name("nope"), None);
    assert_eq!(Panel::from_name(""), None);
}

#[test]
fn panes_follow_the_users_order_and_moves_are_clamped() {
    assert_eq!(Panel::arranged("").as_slice(), Panel::ALL.as_slice());
    // Unknown names and repeats are ignored; the rest keep their usual order after.
    let a = Panel::arranged("weather, nope,home,weather");
    assert_eq!(&a[..2], &[Panel::Weather, Panel::Home]);
    assert_eq!(a.len(), Panel::ALL.len());
    assert_eq!(
        move_pane("", "home", -1)
            .split(',')
            .take(3)
            .collect::<Vec<_>>(),
        ["shelf", "home", "media"]
    );
    // At an end a move changes nothing.
    let first = Panel::ALL[0].key();
    assert_eq!(move_pane("", first, -1), move_pane("", first, 0));
    // The strip uses the order, for the panes that exist.
    let mut m = Model::default();
    m.panes.order = "weather,notes,home".into();
    assert_eq!(
        m.panels(),
        vec![Panel::Weather, Panel::Notes, Panel::Home, Panel::Todo]
    );
}

#[test]
fn every_pane_but_the_quick_controls_can_be_switched_off() {
    let mut m = Model::default();
    m.panes = crate::config::Panes {
        notes: false,
        todo: false,
        weather: false,
        ..Default::default()
    };
    assert_eq!(
        m.panels(),
        vec![Panel::Home],
        "nothing left: quick controls"
    );
    assert_eq!(m.panel(), Panel::Home);
    m.panes.timer = false;
    m.panes.tasks = false;
    assert_eq!(m.panels(), vec![Panel::Home]);
}

#[test]
fn wider_search_is_offered_only_for_an_empty_answer_and_cached() {
    let place = |n: &str| crate::weather::Place {
        name: n.into(),
        region: "India".into(),
        lat: 13.5,
        lon: 80.0,
    };
    let mut m = Model::default();
    m.choosing = true;
    m.place_edit.set("sri city");
    assert!(!m.offer_wide(), "nothing has been searched yet");
    m.places_result("sri city".into(), Ok(vec![]));
    assert!(m.offer_wide() && m.weather_h() > 0.0);
    let h = m.weather_h();
    // Asking is the user's act; the query goes out once, and a second ask is ignored.
    assert_eq!(m.start_wide().as_deref(), Some("sri city"));
    assert_eq!(m.start_wide(), None);
    // An answer for text since changed is dropped.
    m.wide_result("sri".into(), Ok(vec![place("x")]));
    assert!(m.wide_busy && m.places.is_empty());
    m.wide_result("sri city".into(), Ok(vec![place("Sri City")]));
    assert!(m.places_wide && m.places.len() == 1 && !m.wide_offer);
    assert!(
        m.weather_h() > 0.0 && m.weather_h() != h,
        "credit line adds height"
    );
    // Typing again forgets it; the same query later is answered from the cache.
    m.reset_wide();
    m.places.clear();
    m.places_result("sri city".into(), Ok(vec![]));
    assert!(m.offer_wide());
    assert_eq!(m.start_wide(), None);
    assert_eq!(m.places[0].name, "Sri City");
    // A new Open-Meteo answer with matches never offers it.
    m.places_result("sri city".into(), Ok(vec![place("Sri City")]));
    assert!(!m.offer_wide());
    // Nothing found by the wider search either: no offer, message shows.
    let mut m = Model::default();
    m.choosing = true;
    m.place_edit.set("zzzz");
    m.places_result("zzzz".into(), Ok(vec![]));
    assert!(m.offer_wide());
    m.start_wide();
    m.wide_result("zzzz".into(), Ok(vec![]));
    assert!(!m.wide_offer && m.wide_for == "zzzz" && !m.places_wide);
}

#[test]
fn listeners_are_named_by_project_docker_wsl_or_exe() {
    assert_eq!(
        port_name("node.exe", Some(r"C:\code\shop-front")),
        "shop-front"
    );
    assert_eq!(
        port_name("node.exe", Some(r"C:\code\shop-front\")),
        "shop-front"
    );
    assert_eq!(port_name("node.exe", None), "node");
    assert_eq!(port_name("node.exe", Some(r"C:\")), "node");
    assert_eq!(
        port_name("com.docker.backend.exe", Some(r"C:\code\x")),
        "Docker"
    );
    assert_eq!(port_name("wslrelay.exe", None), "WSL");
    assert_eq!(port_name("vmmem", None), "WSL");
}

#[test]
fn answers_wait_out_the_click_guard() {
    let t = Instant::now();
    assert!(!ask_click_ready(Some(t), t + Duration::from_millis(100)));
    assert!(ask_click_ready(Some(t), t + Duration::from_millis(400)));
    assert!(ask_click_ready(None, t));
}

#[test]
fn restart_is_offered_only_for_a_tracked_runnable_command() {
    let t0 = Instant::now();
    let mut m = Model::default();
    let port = PortInfo {
        port: 3000,
        pid: 500,
        exe: "node.exe".into(),
    };
    assert!(m.restart_task(&port).is_none(), "no tracked command");
    let up = |id: &str, source: &str| match st(id, TaskEvent::Start, t0) {
        TaskUpdate::Upsert(mut s) => {
            s.source = source.into();
            s.pid = Some(500);
            s.cwd = Some("C:/x".into());
            TaskUpdate::Upsert(s)
        }
        u => u,
    };
    m.apply_task(up("agent", "agent"), t0);
    assert!(m.restart_task(&port).is_none(), "an agent can't be rerun");
    m.tasks.clear();
    m.apply_task(up("dev", "run"), t0);
    assert_eq!(m.restart_task(&port).map(|t| t.id.as_str()), Some("dev"));
}

#[test]
fn a_cut_command_is_not_rerunnable() {
    let now = Instant::now();
    let mut t = TaskView {
        id: "t".into(),
        label: "cargo build".into(),
        event: TaskEvent::Failed,
        progress: None,
        duration_ms: None,
        exit_code: Some(1),
        stderr: vec![],
        pid: None,
        attention: None,
        line: None,
        source: "run".into(),
        cwd: Some("C:/x".into()),
        started: now,
        changed: now,
        visible_after: now,
    };
    assert!(t.rerunnable());
    t.label.push('…');
    assert!(!t.rerunnable());
}

#[test]
fn shelf_row_scrolls_within_its_items() {
    let item = |id| ShelfItem {
        id,
        path: "x".into(),
        name: "x".into(),
        is_dir: false,
        owned: false,
    };
    let mut m = Model {
        shelf: (1..=8).map(item).collect(),
        ..Default::default()
    };
    assert_eq!(m.shelf_page().1.len(), SHELF_VISIBLE);
    m.scroll_shelf(10);
    assert_eq!(m.shelf_page().0, 3, "stops with the last tile in view");
    m.scroll_shelf(-1);
    assert_eq!(m.shelf_page().1[0].id, 3);
    // Items removed from the end: the row slides back instead of showing a gap.
    m.shelf.truncate(6);
    assert_eq!(m.shelf_page(), (1, &m.shelf[1..6]));
    m.shelf.truncate(3);
    assert_eq!(m.shelf_page().0, 0);
    m.scroll_shelf(-5);
    assert_eq!(m.shelf_off, 0);
}

#[test]
fn swipe_pages_once_per_gesture() {
    let t0 = Instant::now();
    let ms = |n: u64| t0 + Duration::from_millis(n);
    let mut s = Swipe::default();
    // A swipe: rising then decaying deltas every 8 ms, with a long inertia tail.
    let swipe = [10, 25, 30, 22, 14, 9, 6, 4, 3, 2, 2, 1, 1, 1];
    let mut pages = vec![];
    for (i, d) in swipe.iter().enumerate() {
        pages.extend(s.feed(*d, ms(i as u64 * 8)));
    }
    assert_eq!(pages, vec![1], "one page, early in the swipe");
    // A second swipe during that swipe's inertia still pages: its deltas jump up.
    let mut t = swipe.len() as u64 * 8;
    let mut second = vec![];
    for d in [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 12, 28, 34, 30, 15, 6] {
        second.extend(s.feed(d, ms(t)));
        t += 8;
    }
    assert_eq!(second, vec![1]);
    // After a pause, going left pages back, once it has moved far enough.
    assert_eq!(s.feed(-30, ms(t + 300)), None);
    assert_eq!(s.feed(-30, ms(t + 308)), None);
    assert_eq!(s.feed(-30, ms(t + 316)), Some(-1));
    // Tilt-wheel notches 300 ms apart: one page each.
    let mut s = Swipe::default();
    assert_eq!(s.feed(120, ms(0)), Some(1));
    assert_eq!(s.feed(120, ms(300)), Some(1));
    // Reversing mid-stream: one stray event is ignored, a second turns it round.
    assert_eq!(s.feed(-120, ms(320)), None);
    assert_eq!(s.feed(-120, ms(330)), Some(-1));
}

/// Recorded from a real touchpad: one fast swipe left (alternating deltas while the finger
/// moves, a lone spike the wrong way, then the inertia tail) pages exactly once.
#[test]
fn noisy_fast_swipe_pages_once() {
    let t0 = Instant::now();
    let mut s = Swipe::default();
    let swipe = [
        -220, -251, -97, -227, -128, -146, -463, -174, -172, -571, -196, -205, -408, -199, -198,
        -115, 467, -943, -745, -686, -670, -697, -639, -664, -611, -811, -820, -452, -541, -524,
        -512, -525, -511, -549, -390, -435, -417, -648, -403, -563, -339, -344, -478, -285, -278,
        -247, -286, -278,
    ];
    let pages: Vec<i32> = swipe
        .iter()
        .enumerate()
        .filter_map(|(i, d)| s.feed(*d, t0 + Duration::from_millis(i as u64 * 4)))
        .collect();
    assert_eq!(pages, vec![-1]);
}

#[test]
fn notes_todo_and_weather_follow_home_and_can_be_switched_off() {
    let mut m = Model::default();
    assert_eq!(
        m.panels(),
        vec![Panel::Home, Panel::Notes, Panel::Todo, Panel::Weather]
    );
    assert!(m.tabs(), "the strip shows even with nothing else going on");
    m.panes.todo = false;
    m.selected = Some(Panel::Todo);
    assert_eq!(m.panel(), Panel::Home, "a hidden pane can't stay selected");
    assert_eq!(Panel::from_name("To-do"), Some(Panel::Todo));
    assert_eq!(Panel::of(Scene::ExpWeather), Some(Panel::Weather));
    // Scenes follow the panel, and the typing panes grow with their rows.
    m.selected = Some(Panel::Notes);
    assert_eq!(m.scene(true, false), Scene::ExpNotes);
    let empty = m.size(Scene::ExpNotes).1;
    let now = crate::timer::now_ms();
    for i in 0..3 {
        m.jot
            .add(crate::jot::Kind::Note, &format!("n{i}"), true, now);
    }
    assert_eq!(m.size(Scene::ExpNotes).1, empty + 60.0);
    for i in 3..9 {
        m.jot
            .add(crate::jot::Kind::Note, &format!("n{i}"), true, now);
    }
    assert_eq!(
        m.size(Scene::ExpNotes).1,
        empty + 90.0,
        "no taller than four rows"
    );
}

#[test]
fn notes_are_added_edited_deleted_and_undone() {
    let mut m = Model::default();
    m.note_edit.set("  buy milk ");
    assert!(m.submit_jot(Field::Note));
    assert!(m.note_edit.is_empty(), "the field clears");
    assert!(m.jot_dirty);
    assert_eq!(m.jot.notes()[0].text, "buy milk");
    assert!(!m.submit_jot(Field::Note), "nothing to add");
    let id = m.jot.notes()[0].id;
    m.begin_edit(id);
    assert_eq!(m.typing, Some(Field::Note));
    assert_eq!(m.note_edit.text(), "buy milk");
    m.note_edit.set("buy oat milk");
    assert!(m.submit_jot(Field::Note));
    assert_eq!(m.jot.notes()[0].text, "buy oat milk");
    assert_eq!(m.jot.items.len(), 1, "an edit doesn't add");
    // Esc on an edit drops it and leaves the field empty.
    m.begin_edit(id);
    m.end_typing();
    assert!(m.note_edit.is_empty() && m.typing.is_none() && m.jot_edit.is_none());
    let t0 = Instant::now();
    m.delete_jot(id, t0);
    assert!(m.jot.items.is_empty());
    assert!(m.jot_undo.is_some());
    m.undo_jot();
    assert_eq!(m.jot.items.len(), 1);
    // Undo goes after a few seconds, with its own deadline in `expire`.
    m.delete_jot(id, t0);
    let next = m.expire(t0 + Duration::from_secs(1));
    assert_eq!(next, Some(t0 + UNDO_HOLD));
    m.expire(t0 + UNDO_HOLD);
    assert!(m.jot_undo.is_none());
}

#[test]
fn a_draft_survives_leaving_the_field_and_wheel_scrolls_only_long_lists() {
    let mut m = Model::default();
    m.note_edit.set("half a thought");
    m.typing = Some(Field::Note);
    m.end_typing();
    assert_eq!(m.note_edit.text(), "half a thought", "the draft stays");
    m.selected = Some(Panel::Notes);
    let now = crate::timer::now_ms();
    for i in 0..4 {
        m.jot
            .add(crate::jot::Kind::Note, &format!("n{i}"), true, now + i);
    }
    assert!(!m.scroll_jot(1), "four rows fit: the wheel pages instead");
    m.jot.add(crate::jot::Kind::Note, "n4", true, now + 4);
    assert!(m.scroll_jot(1));
    assert!(!m.scroll_jot(1), "at the end it pages again");
    assert_eq!(m.jot_window(Scene::ExpNotes), (1, 4));
}

#[test]
fn temporary_notes_set_the_next_deadline() {
    let mut m = Model::default();
    let t0 = Instant::now();
    assert_eq!(m.expire(t0), None);
    m.jot.add(
        crate::jot::Kind::Note,
        "soon",
        false,
        crate::timer::now_ms(),
    );
    let next = m.expire(t0).expect("a deadline for the 24 h note");
    let secs = next.duration_since(t0).as_secs();
    assert!((86_395..=86_400).contains(&secs), "{secs}");
}

#[test]
fn panels_and_selection() {
    // (a running timer adds its own pane)
    let mut m = no_extra_panes();
    assert_eq!(m.panels(), vec![Panel::Home]);
    assert!(!m.tabs());
    m.ports = vec![PortInfo {
        port: 3000,
        pid: 9,
        exe: "node.exe".into(),
    }];
    m.shelf = vec![ShelfItem {
        id: 1,
        path: "a".into(),
        name: "a".into(),
        is_dir: false,
        owned: false,
    }];
    // Shelf and music to the left of home, dev work and the battery to its right; a
    // listening port puts the Tasks pane up even without a task.
    m.power = Some(crate::power::Battery {
        pct: 50,
        plugged: false,
        watts: None,
        mode: crate::power::Mode::Balanced,
        mins_left: None,
    });
    assert_eq!(
        m.panels(),
        vec![Panel::Shelf, Panel::Home, Panel::Tasks, Panel::Power]
    );
    assert!(m.tabs());
    assert_eq!(m.panel(), Panel::Home, "home is the default");
    m.selected = Some(Panel::Tasks);
    assert_eq!(m.panel(), Panel::Tasks);
    m.ports.clear();
    assert_eq!(m.panel(), Panel::Home, "stale selection falls back to home");
    // A running task in the collapsed pill opens on its own pane.
    m.selected = None;
    m.apply_task(st("t", TaskEvent::Start, Instant::now()), Instant::now());
    assert_eq!(m.panel(), Panel::Tasks);
}

#[test]
fn wheel_steps_and_wraps() {
    let mut m = no_extra_panes();
    m.ports = vec![PortInfo {
        port: 3000,
        pid: 9,
        exe: "node.exe".into(),
    }];
    assert_eq!(m.panels(), vec![Panel::Home, Panel::Tasks]);
    assert_eq!(m.step_panel(1), Panel::Tasks);
    assert_eq!(m.step_panel(1), Panel::Home, "wraps forward");
    assert_eq!(m.step_panel(-1), Panel::Tasks, "wraps backward");
}

#[test]
fn drop_breaks_through_fullscreen_suppression() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.now = Some(t0);
    assert_eq!(m.scene(false, true), Scene::Sentinel);
    m.drop_over = true;
    assert_eq!(
        m.scene(false, true),
        Scene::ExpDrop,
        "drag over the notch while fullscreen"
    );
    m.drop_over = false;
    m.shelf = vec![ShelfItem {
        id: 1,
        path: "a".into(),
        name: "a".into(),
        is_dir: false,
        owned: false,
    }];
    m.force_until = Some(t0 + Duration::from_secs(4));
    m.peek(Panel::Shelf, t0 + Duration::from_secs(4));
    assert_eq!(
        m.scene(false, true),
        Scene::ExpShelf,
        "feedback after the drop"
    );
    m.expire(t0 + Duration::from_secs(5));
    assert_eq!(m.scene(false, true), Scene::Sentinel);
}
