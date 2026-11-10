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
    // A card, the finished-timer card and the settings pane are never covered by it.
    m.settings = Some(0);
    assert_eq!(m.scene(false, false), Scene::ExpSettings);
    m.settings = None;
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
