use super::*;

#[test]
fn script_panes_take_slots_keep_their_text_and_run_on_schedule() {
    let t0 = Instant::now();
    let mut m = Model {
        panes: crate::config::Panes::default(),
        ..Default::default()
    };
    m.apply_scripts(&[pane("Server", "curl x"), pane("Deploys", "")]);
    let ps = m.panels();
    assert!(ps.contains(&Panel::Script(0)) && ps.contains(&Panel::Script(1)));
    assert_eq!(m.panel_label(Panel::Script(1)), "Deploys");
    assert_eq!(
        Panel::from_name(Panel::Script(1).saved_name()),
        Some(Panel::Script(1))
    );
    // A switched-off pane leaves the tab strip.
    m.panes.script2 = false;
    assert!(!m.panels().contains(&Panel::Script(1)));
    m.panes.script2 = true;
    // Only the pane on screen runs, once until its interval passes; push-only never runs.
    let due = m.scripts_due(Some(0), t0);
    assert_eq!(due.len(), 1);
    assert_eq!((due[0].0, due[0].2.as_str()), (0, "curl x"));
    assert!(
        m.scripts_due(Some(0), t0 + Duration::from_secs(120))
            .is_empty(),
        "busy"
    );
    m.script_done(
        "Server",
        crate::scriptpane::Run {
            text: "[32mup[0m
3 users"
                .into(),
            code: Some(0),
            ..Default::default()
        },
        t0 + Duration::from_secs(1),
    );
    assert_eq!(
        m.scripts[0].text,
        "up
3 users"
    );
    assert!(m
        .scripts_due(Some(0), t0 + Duration::from_secs(59))
        .is_empty());
    assert_eq!(
        m.scripts_due(Some(0), t0 + Duration::from_secs(60)).len(),
        1
    );
    assert!(
        m.scripts_due(Some(1), t0).is_empty(),
        "push-only panes never run"
    );
    // A push lands by name, any case; an unknown name is ignored.
    assert!(m.script_text("deploys", "v2.0 live", None, t0));
    assert!(!m.script_text("nope", "x", None, t0));
    // Reloading the config keeps what a pane showed.
    m.apply_scripts(&[pane("Deploys", ""), pane("Server", "curl x")]);
    assert_eq!(m.scripts[0].text, "v2.0 live");
    // Failures say so.
    m.script_done(
        "Server",
        crate::scriptpane::Run {
            text: "refused".into(),
            code: Some(7),
            ..Default::default()
        },
        t0,
    );
    assert_eq!(m.scripts[1].problem.as_deref(), Some("Exited with code 7"));
    m.script_done(
        "Server",
        crate::scriptpane::Run {
            text: String::new(),
            code: None,
            ..Default::default()
        },
        t0,
    );
    assert_eq!(m.scripts[1].problem.as_deref(), Some("Stopped after 10 s"));
}

#[test]
fn the_script_footer_says_how_fresh_it_is() {
    let t0 = Instant::now();
    let mut s = ScriptView {
        command: "x".into(),
        every: Duration::from_secs(300),
        updated: Some(t0),
        ..Default::default()
    };
    assert_eq!(script_footer(&s, t0), "Updated just now · every 5 min");
    assert_eq!(
        script_footer(&s, t0 + Duration::from_secs(185)),
        "Updated 3 min ago · every 5 min"
    );
    s.every = Duration::from_secs(45);
    assert_eq!(
        script_footer(&s, t0 + Duration::from_secs(7300)),
        "Updated 2 h ago · every 45 s"
    );
    s.command.clear();
    assert_eq!(script_footer(&s, t0), "Updated just now");
    s.updated = None;
    assert_eq!(script_footer(&s, t0), "");
    s.problem = Some("Exited with code 1".into());
    assert_eq!(script_footer(&s, t0), "Exited with code 1");
}

#[test]
fn a_live_card_updates_in_place_wherever_it_is() {
    let t0 = Instant::now();
    let mut m = Model::default();
    let first = hytte_proto::Cmd {
        title: Some("Deploy".into()),
        text: Some("Uploading…".into()),
        progress: Some(40),
        ..card("deploy")
    };
    assert_eq!(m.card(&first, t0), "deploy");
    assert!(m.chip.is_none(), "cards queue; the window shows them");
    m.show_card(t0);
    let c = m.chip.as_ref().unwrap();
    assert_eq!(c.summary, "Deploy\nUploading…");
    assert_eq!((c.tone, c.hold), (Tone::Info, CARD_SILENCE));
    // An update keeps the title, changes the body and progress, and restarts the hold.
    let t1 = t0 + Duration::from_secs(20);
    let upd = hytte_proto::Cmd {
        text: Some("Almost".into()),
        progress: Some(90),
        ..card("deploy")
    };
    m.card(&upd, t1);
    let c = m.chip.as_ref().unwrap();
    assert_eq!(c.summary, "Deploy\nAlmost");
    assert_eq!(c.card.as_ref().unwrap().progress, Some(90));
    m.expire(t1 + Duration::from_secs(29));
    assert!(m.chip.is_some(), "30 s of silence, not 30 s in all");
    // Done fills the bar, ticks, and stays the normal ten seconds.
    let t2 = t1 + Duration::from_secs(29);
    m.card(
        &hytte_proto::Cmd {
            done: true,
            text: Some("Live".into()),
            ..card("deploy")
        },
        t2,
    );
    let c = m.chip.as_ref().unwrap();
    assert_eq!(
        (c.tone, c.card.as_ref().unwrap().progress),
        (Tone::Done, Some(100))
    );
    assert_eq!(c.hold, Duration::from_secs(10));
    // A second card waits behind the first and is updated while it waits.
    m.card(
        &hytte_proto::Cmd {
            title: Some("Tests".into()),
            ..card("tests")
        },
        t2,
    );
    m.card(
        &hytte_proto::Cmd {
            icon: Some("error".into()),
            text: Some("3 failed".into()),
            ..card("tests")
        },
        t2,
    );
    assert_eq!(m.cards.len(), 1);
    m.expire(t2 + Duration::from_secs(10));
    assert!(m.chip.is_none());
    m.show_card(t2 + Duration::from_secs(10));
    let c = m.chip.as_ref().unwrap();
    assert_eq!(
        (c.summary.as_str(), c.tone),
        ("Tests\n3 failed", Tone::Error)
    );
    assert!(m.has_card("tests") && !m.has_card("deploy"));
    // Clear takes it away from the chip or the queue.
    m.card(
        &hytte_proto::Cmd {
            clear: true,
            ..card("tests")
        },
        t2,
    );
    assert!(m.chip.is_none() && !m.has_card("tests"));
}

#[test]
fn card_buttons_report_back_and_an_ask_puts_the_card_back_in_line() {
    let t0 = Instant::now();
    let mut m = Model::default();
    let ask = hytte_proto::Cmd {
        title: Some("Restart server?".into()),
        buttons: vec!["Restart".into(), "Later".into()],
        open: Some("https://example.com".into()),
        ttl: Some(60),
        ..hytte_proto::Cmd::card()
    };
    let id = m.card(&ask, t0);
    assert!(id.starts_with("card:"), "cards without an id get one");
    m.show_card(t0);
    let c = m.chip.as_ref().unwrap();
    let labels: Vec<&str> = c.extra.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(labels, ["Open", "Restart", "Later"]);
    assert_eq!(c.extra[1].1, ChipAction::Card(id.clone(), "Restart".into()));
    assert_eq!(c.hold, Duration::from_secs(60));
    // Another card is already waiting; an urgent chip puts this one back first in line.
    m.card(
        &hytte_proto::Cmd {
            title: Some("later".into()),
            ..hytte_proto::Cmd::card()
        },
        t0,
    );
    m.requeue_card();
    assert!(m.chip.is_none());
    assert_eq!(m.cards.front().and_then(Chip::card_id), Some(id.as_str()));
    // A plain chip is never requeued.
    m.chip = Some(Chip::new("download done", t0));
    m.requeue_card();
    assert_eq!(m.cards.len(), 2);
}

#[test]
fn a_chip_keeps_its_own_hold_time_and_the_kill_confirm_dies_with_it() {
    let t0 = Instant::now();
    let mut m = Model::default();
    let mut c = Chip::new("hog", t0);
    c.hold = Duration::from_secs(20);
    m.chip = Some(c);
    m.chip_armed = Some(t0 + KILL_CONFIRM);
    // The single deadline timer wakes for the confirm window first, then the chip's end.
    assert_eq!(m.expire(t0), Some(t0 + KILL_CONFIRM));
    assert!(m.chip.is_some() && m.chip_armed.is_some());
    assert_eq!(
        m.expire(t0 + KILL_CONFIRM),
        Some(t0 + Duration::from_secs(20))
    );
    assert!(m.chip.is_some() && m.chip_armed.is_none());
    // Past the default 10 s the long-hold chip is still there; at 20 s it goes.
    m.expire(t0 + Duration::from_secs(11));
    assert!(m.chip.is_some());
    m.expire(t0 + Duration::from_secs(20));
    assert!(m.chip.is_none());
    // Dismissing a chip also drops a pending Kill confirm.
    m.chip = Some(Chip::new("x", t0));
    m.chip_armed = Some(t0 + Duration::from_secs(1));
    m.dismiss_chip();
    m.expire(t0);
    assert!(m.chip_armed.is_none());
}

#[test]
fn calendar_heads_up_comes_due_once() {
    let t0 = Instant::now();
    let ev = || crate::calendar::NextEvent {
        subject: "Standup".into(),
        start_ms: 1_000_000_000 + 30 * 60_000,
        url: None,
    };
    let wall = 1_000_000_000;
    let lead = 10 * 60_000;
    let mut m = Model::default();
    m.set_calendar(Some(ev()), wall, lead, t0);
    // Due 20 minutes from now, and the single deadline timer knows.
    assert_eq!(m.calendar_due(t0), None);
    assert_eq!(m.expire(t0), Some(t0 + Duration::from_secs(20 * 60)));
    assert_eq!(
        m.calendar_due(t0 + Duration::from_secs(20 * 60))
            .map(|e| e.subject),
        Some("Standup".into())
    );
    // Shown: the same event coming back from the worker doesn't re-arm it.
    m.set_calendar(Some(ev()), wall, lead, t0);
    assert!(m.calendar.is_none());
    // A different event does; and one already inside its lead time is due at once.
    let mut next = ev();
    next.subject = "Review".into();
    next.start_ms = wall + 4 * 60_000;
    m.set_calendar(Some(next), wall, lead, t0);
    assert!(m.calendar_due(t0).is_some());
    // Nothing upcoming clears it.
    m.set_calendar(None, wall, lead, t0);
    assert!(m.calendar.is_none());
}

#[test]
fn a_stopped_command_is_not_an_error() {
    let t0 = Instant::now();
    for code in [130, 143, -1_073_741_510] {
        let mut m = Model::default();
        m.apply_task(shell("a", "dev", TaskEvent::Start, 0, t0), t0);
        let mut end = shell("a", "dev", TaskEvent::Failed, 0, t0);
        if let TaskUpdate::Upsert(s) = &mut end {
            s.exit_code = Some(code);
        }
        m.apply_task(end, t0 + Duration::from_secs(9));
        assert!(m.tasks.is_empty(), "exit {code} left a row");
    }
}

#[test]
fn a_failed_row_goes_when_its_terminal_closes() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(st("a", TaskEvent::Failed, t0), t0);
    m.apply_task(st("b", TaskEvent::Start, t0), t0);
    m.apply_task(TaskUpdate::Gone("a".into()), t0);
    m.apply_task(TaskUpdate::Gone("b".into()), t0);
    // Only a failed row is dropped; a running one is the registry's to mark Lost.
    assert_eq!(m.tasks.len(), 1);
    assert_eq!(m.tasks[0].id, "b");
}

#[test]
fn the_task_list_scrolls_past_four_rows() {
    let t0 = Instant::now();
    let mut m = Model::default();
    for i in 0..7 {
        m.apply_task(st(&format!("t{i}"), TaskEvent::Start, t0), t0);
    }
    assert_eq!(m.rows().len(), 4);
    assert_eq!(m.rows_hidden(), (0, 3));
    assert!(m.scroll_tasks(10));
    assert!(!m.scroll_tasks(1), "at the end: the wheel pages instead");
    assert_eq!(m.rows_hidden(), (3, 0));
    assert_eq!(m.rows().len(), 4);
    m.scroll_tasks(-1);
    assert_eq!(m.rows_hidden(), (2, 1));
    // Rows leaving keeps the window valid.
    m.task_first = 9;
    assert_eq!(m.rows().len(), 4);
}

fn rows_json(n: usize) -> String {
    let rows: Vec<String> = (0..n)
        .map(|i| {
            format!(
                r#"{{"text":"svc{i}","value":"up","tone":"good","progress":{}}}"#,
                i * 10
            )
        })
        .collect();
    format!(r#"{{"rows":[{}]}}"#, rows.join(","))
}

#[test]
fn a_pane_shows_rows_pushed_as_json_and_falls_back_to_text() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_scripts(&[pane("Stack", "")]);
    assert!(m.script_text("Stack", &rows_json(3), None, t0));
    let s = &m.scripts[0];
    assert_eq!((s.rows.len(), s.text.as_str()), (3, ""));
    // Rows are taller than text lines, and the footer sits under the last one.
    let boxes = script_row_boxes(&s.rows);
    assert_eq!(boxes[0], (10.0, SCRIPT_ROW_H + SCRIPT_BAR_H));
    assert_eq!(script_footer_y(s), boxes[2].0 + boxes[2].1 + 6.0);
    assert_eq!(m.script_h(0), script_footer_y(&m.scripts[0]) as f64 + 28.0);
    // Plain text afterwards replaces the rows; broken JSON is just text.
    m.script_text("Stack", "all fine", None, t0);
    assert_eq!(
        (m.scripts[0].rows.len(), m.scripts[0].text.as_str()),
        (0, "all fine")
    );
    m.script_text("Stack", "{not json", None, t0);
    assert_eq!(m.scripts[0].text, "{not json");
    // A run that printed rows (parsed on its worker thread) shows them.
    let run = crate::scriptpane::Run {
        rows: crate::scriptpane::parse_rows(&rows_json(2)),
        code: Some(0),
        ..Default::default()
    };
    m.script_done("Stack", run, t0);
    assert_eq!(
        (m.scripts[0].rows.len(), m.scripts[0].problem.clone()),
        (2, None)
    );
    // And a failing one says so beside them.
    let run = crate::scriptpane::Run {
        rows: crate::scriptpane::parse_rows(&rows_json(1)),
        code: Some(3),
        ..Default::default()
    };
    m.script_done("Stack", run, t0);
    assert_eq!(m.scripts[0].problem.as_deref(), Some("Exited with code 3"));
}

#[test]
fn a_card_button_names_an_action_only_when_config_has_it() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_actions(&[crate::config::NamedAction {
        name: "restart-api".into(),
        command: "x".into(),
        ..Default::default()
    }]);
    let c = hytte_proto::Cmd {
        buttons: vec![
            "Restart:restart-api".into(),
            "Open: docs".into(),
            "Wipe:no-such-action".into(),
            "Later".into(),
        ],
        ..card("srv")
    };
    m.card(&c, t0);
    m.show_card(t0);
    let extra = &m.chip.as_ref().unwrap().extra;
    assert_eq!(
        extra[0],
        ("Restart".into(), ChipAction::Action("restart-api".into()))
    );
    // Everything else is an ordinary button that only reports back to a waiting script.
    for (i, label) in ["Open: docs", "Wipe:no-such-action", "Later"]
        .iter()
        .enumerate()
    {
        assert_eq!(
            extra[i + 1],
            (
                label.to_string(),
                ChipAction::Card("srv".into(), label.to_string())
            )
        );
    }
}

#[test]
fn an_action_that_needs_confirming_arms_first_and_runs_on_the_second_click() {
    let t0 = Instant::now();
    let mut m = Model::default();
    assert_eq!(m.press_action("wipe", true, t0), Press::Arm);
    m.now = Some(t0);
    assert!(m.action_armed("wipe") && !m.action_armed("other"));
    // The armed state fades after a few seconds, and a late second click only arms again.
    m.expire(t0 + KILL_CONFIRM);
    assert!(!m.action_armed("wipe"));
    assert_eq!(m.press_action("wipe", true, t0 + KILL_CONFIRM), Press::Arm);
    let t1 = t0 + KILL_CONFIRM + Duration::from_secs(1);
    assert_eq!(m.press_action("wipe", true, t1), Press::Run);
    assert!(!m.action_armed("wipe"));
    // Running: more clicks do nothing until it finishes.
    assert_eq!(m.press_action("wipe", true, t1), Press::Busy);
    m.action_finished("wipe");
    assert_eq!(m.press_action("wipe", true, t1), Press::Arm);
    // No confirm: runs at once.
    assert_eq!(m.press_action("ping", false, t1), Press::Run);
    // The arming deadline is something the timer must wake for.
    let mut m = Model::default();
    m.press_action("wipe", true, t0);
    assert_eq!(m.expire(t0), Some(t0 + KILL_CONFIRM));
}

#[test]
fn a_finished_action_says_how_it_went() {
    let t0 = Instant::now();
    let run = |code, text: &str| crate::scriptpane::Run {
        text: text.into(),
        code,
        ..Default::default()
    };
    let ok = action_card("restart-api", &run(Some(0), "api restarted\nextra"), 60, t0);
    assert_eq!(
        (ok.summary.as_str(), ok.tone, ok.note.as_deref()),
        ("restart-api finished", Tone::Done, Some("api restarted"))
    );
    let bad = action_card("restart-api", &run(Some(3), ""), 60, t0);
    assert_eq!(
        (bad.summary.as_str(), bad.tone, bad.note),
        ("restart-api failed (exit 3)", Tone::Warn, None)
    );
    let slow = action_card("restart-api", &run(None, ""), 45, t0);
    assert_eq!(slow.summary, "restart-api stopped after 45 s");
    assert_eq!(slow.key.as_deref(), Some("action:restart-api"));
}

#[test]
fn a_packs_rows_name_their_actions_with_the_packs_prefix() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_scripts(&[crate::config::ScriptPane {
        name: "PRs".into(),
        command: "x".into(),
        pack: Some("github-prs".into()),
        dir: Some(r"C:\p\github-prs".into()),
        ..Default::default()
    }]);
    let json = r##"{"rows":[{"text":"#12","action":"open"},{"text":"#13"}]}"##;
    m.script_text("PRs", json, None, t0);
    assert_eq!(
        m.scripts[0].rows[0].action.as_deref(),
        Some("github-prs-open")
    );
    assert_eq!(m.scripts[0].rows[1].action, None);
    // Its command runs in the pack's folder.
    let due = m.scripts_due(None, t0);
    assert_eq!(
        due[0].3.as_deref(),
        Some(std::path::Path::new(r"C:\p\github-prs"))
    );
    // A pane from config.toml keeps the names it was given.
    let mut n = Model::default();
    n.apply_scripts(&[pane("Mine", "")]);
    n.script_text("Mine", json, None, t0);
    assert_eq!(n.scripts[0].rows[0].action.as_deref(), Some("open"));
}
