use super::*;

#[test]
fn only_a_focus_timer_holds_alerts() {
    use crate::timer::{Timer, TimerKind};
    let mut m = Model::default();
    assert!(!m.holding());
    let wall = crate::timer::now_ms();
    for (kind, holds) in [
        (TimerKind::Plain, false),
        (TimerKind::Break, false),
        (TimerKind::Focus, true),
    ] {
        m.timer = Some(Timer::start(kind, 5, wall));
        assert_eq!(m.holding(), holds, "{kind:?}");
    }
    // Paused is still a session.
    m.timer = Some(focus_timer());
    m.timer.as_mut().unwrap().paused_ms = Some(60_000);
    assert!(m.holding());
}

#[test]
fn held_alerts_are_told_once_in_one_line() {
    let mut h = Held::default();
    assert_eq!(h.take_summary(), None);
    h.downloads = 2;
    h.failed = 1;
    assert_eq!(
        h.take_summary().as_deref(),
        Some("While you focused: 2 downloads, 1 task failed")
    );
    assert_eq!(h.take_summary(), None, "counts start over");
    h = Held {
        downloads: 1,
        done: 3,
        failed: 0,
        warnings: 1,
    };
    assert_eq!(
        h.take_summary().as_deref(),
        Some("While you focused: 1 download, 3 tasks finished, 1 warning")
    );
}

#[test]
fn an_agent_reply_waits_out_a_focus_session() {
    let t0 = Instant::now();
    let mut m = Model {
        finish_peek: Duration::from_secs(5),
        ..Default::default()
    };
    let reply = || {
        let mut u = st("agent:Claude Code:1", TaskEvent::Done, t0);
        if let TaskUpdate::Upsert(s) = &mut u {
            s.source = "agent".into();
            s.label = "Claude Code".into();
            s.message = Some("All done.".into());
        }
        u
    };
    m.timer = Some(focus_timer());
    m.apply_task(reply(), t0);
    assert!(m.chip.is_none(), "no card during focus");
    m.timer = None;
    m.apply_task(reply(), t0);
    assert!(m.chip.is_some(), "and back to normal afterwards");
}

#[test]
fn a_break_nudge_sits_below_cards_and_ends_by_itself() {
    use crate::breaks::Kind;
    let t0 = Instant::now();
    let mut m = Model::default();
    m.now = Some(t0);
    m.nudge = Some(Nudge {
        kind: Kind::Eyes,
        since: t0,
    });
    assert_eq!(m.scene(false, false), Scene::ExpNudge);
    assert_eq!(
        m.scene(true, false),
        Scene::ExpNudge,
        "hovering doesn't hide it"
    );
    // A card and the finished-timer card are never covered by it.
    m.timer_done = Some((crate::timer::TimerKind::Focus, t0));
    assert_eq!(m.scene(false, false), Scene::ExpTimerDone);
    m.timer_done = None;
    m.chip = Some(Chip::new("x", t0));
    assert_eq!(m.scene(false, false), Scene::ExpChip);
    m.chip = None;
    // The eye break animates its ring, and only it; neither outlives its hold.
    assert!(m.ambient(Scene::ExpNudge, t0));
    m.nudge = Some(Nudge {
        kind: Kind::Water,
        since: t0,
    });
    assert!(!m.ambient(Scene::ExpNudge, t0));
    assert_eq!(m.expire(t0), Some(t0 + Duration::from_secs(10)));
    assert!(m.nudge.is_some());
    m.expire(t0 + Duration::from_secs(10));
    assert!(m.nudge.is_none());
    assert_eq!(m.scene(false, false), Scene::Idle);
    m.nudge = Some(Nudge {
        kind: Kind::Eyes,
        since: t0,
    });
    m.expire(t0 + Duration::from_secs(19));
    assert!(m.nudge.is_some(), "twenty seconds for the eyes");
    m.expire(t0 + Duration::from_secs(20));
    assert!(m.nudge.is_none());
    assert!(!m.ambient(Scene::ExpNudge, t0), "frames stop with it");
}

#[test]
fn the_focus_summary_goes_when_the_finished_card_does() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.timer_done = Some((crate::timer::TimerKind::Focus, t0));
    m.focus_summary = Some("While you focused: 1 download".into());
    assert_eq!(m.size(Scene::ExpTimerDone).1, 134.0, "room for the line");
    m.expire(t0);
    assert!(m.focus_summary.is_some());
    m.timer_done = None;
    m.expire(t0);
    assert!(m.focus_summary.is_none());
    assert_eq!(m.size(Scene::ExpTimerDone).1, 114.0);
}

#[test]
fn an_agent_reply_comes_as_a_card_with_copy() {
    let t0 = Instant::now();
    let mut m = Model {
        finish_peek: Duration::from_secs(5),
        ..Default::default()
    };
    let done = |msg: Option<&str>| {
        let mut u = st("agent:Claude Code:1", TaskEvent::Done, t0);
        if let TaskUpdate::Upsert(s) = &mut u {
            s.source = "agent".into();
            s.label = "Claude Code".into();
            s.message = msg.map(Into::into);
        }
        u
    };
    m.apply_task(done(None), t0);
    assert!(m.chip.is_none());
    m.apply_task(done(Some("\n## **Fixed the bug.**\nDetails follow.")), t0);
    let c = m.chip.as_ref().unwrap();
    assert_eq!(c.summary, "Claude Code: Fixed the bug.");
    assert_eq!(c.hold, Duration::from_secs(5));
    assert_eq!(
        c.copy.as_deref(),
        Some("## **Fixed the bug.**\nDetails follow.")
    );
    // A pending ask is not pushed aside.
    let mut ask = Chip::new("Allow?", t0);
    ask.tone = Tone::Warn;
    m.chip = Some(ask);
    m.apply_task(done(Some("Other reply")), t0);
    assert_eq!(m.chip.as_ref().unwrap().summary, "Allow?");
    // "Open on finish" off: no card at all.
    m.chip = None;
    m.finish_peek = Duration::ZERO;
    m.apply_task(done(Some("Quiet reply")), t0);
    assert!(m.chip.is_none());
}

fn chip_of(text: &str, prio: Prio, now: Instant) -> Chip {
    let mut c = Chip::new(text, now);
    c.prio = prio;
    c
}

fn waiting(m: &Model) -> Vec<&str> {
    m.cards.iter().map(|c| c.summary.as_str()).collect()
}

#[test]
fn an_alert_that_arrives_over_a_card_waits_instead_of_being_dropped() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.push_chip(chip_of("download one", Prio::Alert, t0), t0);
    assert_eq!(m.chip.as_ref().unwrap().summary, "download one");
    m.push_chip(chip_of("download two", Prio::Alert, t0), t0);
    assert_eq!(waiting(&m), ["download two"]);
    // The first folds away on its hold, then the waiting one takes the slot.
    let t1 = t0 + CHIP_HOLD;
    m.expire(t1);
    assert!(m.chip.is_none());
    m.show_card(t1);
    assert_eq!(m.chip.as_ref().unwrap().summary, "download two");
    // Its hold counts from when it was shown, not from when it arrived.
    assert_eq!(m.chip.as_ref().unwrap().since, t1);
}

#[test]
fn waiting_chips_go_most_urgent_first_and_equal_ones_in_turn() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.push_chip(chip_of("on screen", Prio::Info, t0), t0);
    for (text, prio) in [
        ("news", Prio::Info),
        ("alert a", Prio::Alert),
        ("update", Prio::Update),
        ("alert b", Prio::Alert),
        ("card", Prio::Card),
        ("ask", Prio::Ask),
    ] {
        m.push_chip(chip_of(text, prio, t0), t0);
    }
    assert_eq!(
        waiting(&m),
        ["ask", "update", "card", "alert a", "alert b", "news"]
    );
}

#[test]
fn a_new_chip_does_not_jump_a_more_urgent_one_that_waits() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.cards.push_back(chip_of("ask", Prio::Ask, t0));
    m.push_chip(chip_of("news", Prio::Info, t0), t0);
    assert!(m.chip.is_none(), "the slot is free but the ask is first");
    assert_eq!(waiting(&m), ["ask", "news"]);
}

#[test]
fn an_ask_takes_the_slot_and_what_it_displaced_comes_back_after() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.push_chip(chip_of("download", Prio::Alert, t0), t0);
    m.push_chip(chip_of("another", Prio::Alert, t0), t0);
    // What the window does for an ask: put the shown chip back, then show the ask.
    m.requeue_card();
    m.chip = Some(chip_of("allow?", Prio::Ask, t0));
    assert_eq!(waiting(&m), ["download", "another"]);
    // Plain news is not worth showing twice.
    let mut n = Model::default();
    n.push_chip(chip_of("saved", Prio::Info, t0), t0);
    n.requeue_card();
    assert!(n.cards.is_empty());
}

#[test]
fn a_result_interrupts_news_but_not_an_ask_or_an_update() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.push_chip(chip_of("download", Prio::Alert, t0), t0);
    m.interrupt(chip_of("copied", Prio::Info, t0), t0);
    assert_eq!(m.chip.as_ref().unwrap().summary, "copied");
    assert_eq!(waiting(&m), ["download"]);
    m.chip = Some(chip_of("allow?", Prio::Ask, t0));
    m.interrupt(chip_of("saved", Prio::Info, t0), t0);
    assert_eq!(m.chip.as_ref().unwrap().summary, "allow?");
    assert_eq!(waiting(&m), ["download", "saved"]);
}

#[test]
fn a_chip_with_the_same_key_replaces_the_old_one() {
    let t0 = Instant::now();
    let mut m = Model::default();
    let keyed = |text: &str, key: &str, now: Instant| {
        let mut c = chip_of(text, Prio::Alert, now);
        c.key = Some(key.into());
        c
    };
    m.push_chip(keyed("busy 1", "hog:node", t0), t0);
    let t1 = t0 + Duration::from_secs(4);
    m.push_chip(keyed("busy 2", "hog:node", t1), t1);
    assert_eq!(m.chip.as_ref().unwrap().summary, "busy 2");
    assert_eq!(m.chip.as_ref().unwrap().since, t1, "the hold starts again");
    assert!(m.cards.is_empty());
    // And while waiting.
    m.push_chip(keyed("file a", "dl:a", t1), t1);
    m.push_chip(keyed("file b", "dl:b", t1), t1);
    m.push_chip(keyed("file a, again", "dl:a", t1), t1);
    assert_eq!(waiting(&m), ["file b", "file a, again"]);
}

#[test]
fn at_most_eight_wait_and_the_least_urgent_newest_goes() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.push_chip(chip_of("on screen", Prio::Alert, t0), t0);
    for i in 0..12 {
        m.push_chip(chip_of(&format!("n{i}"), Prio::Info, t0), t0);
    }
    assert_eq!(m.cards.len(), MAX_WAITING);
    assert_eq!(m.cards.back().unwrap().summary, "n7");
    // An urgent one still gets in, pushing the last news out.
    m.push_chip(chip_of("ask", Prio::Ask, t0), t0);
    assert_eq!(m.cards.len(), MAX_WAITING);
    assert_eq!(m.cards.front().unwrap().summary, "ask");
    assert_eq!(m.cards.back().unwrap().summary, "n6");
}

#[test]
fn waiting_alerts_go_stale_but_live_cards_do_not() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.push_chip(chip_of("on screen", Prio::Alert, t0), t0);
    m.push_chip(chip_of("short", Prio::Alert, t0), t0);
    m.card(
        &hytte_proto::Cmd {
            text: Some("deploy".into()),
            ..card("deploy")
        },
        t0,
    );
    assert_eq!(m.cards.len(), 2);
    // The next deadline includes the waiting alert's.
    let next = m.expire(t0 + Duration::from_secs(1)).unwrap();
    assert!(next <= t0 + WAIT_STALE, "{next:?}");
    // It outlasts the chip on screen, then shows.
    m.expire(t0 + CHIP_HOLD);
    assert!(waiting(&m).contains(&"short"));
    // Left waiting past WAIT_STALE it is stale; the live card stays for its script.
    m.expire(t0 + WAIT_STALE);
    assert!(m.has_card("deploy"));
    assert!(!waiting(&m).contains(&"short"));
}

#[test]
fn starting_the_camera_or_mic_says_so_once_and_stopping_says_nothing() {
    let say = |old, new, app| privacy_start(old, new, app);
    assert_eq!(
        say((false, false), (true, false), Some("Zoom")).as_deref(),
        Some("Camera in use: Zoom")
    );
    assert_eq!(
        say((false, false), (false, true), Some("Teams")).as_deref(),
        Some("Microphone in use: Teams")
    );
    assert_eq!(
        say((false, false), (true, true), None).as_deref(),
        Some("Camera and microphone in use")
    );
    // The mic joins a call that already has the camera: only the mic is new.
    assert_eq!(
        say((true, false), (true, true), Some("Zoom")).as_deref(),
        Some("Microphone in use: Zoom")
    );
    // Stopping, or nothing changing, is silent.
    assert_eq!(say((true, true), (false, false), Some("Zoom")), None);
    assert_eq!(say((true, false), (true, false), Some("Zoom")), None);
    assert_eq!(say((true, true), (true, false), None), None);
    // A blank app name is left out.
    assert_eq!(
        say((false, false), (true, false), Some("  ")).as_deref(),
        Some("Camera in use")
    );
}
