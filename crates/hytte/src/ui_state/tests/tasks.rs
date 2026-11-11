use super::*;

#[test]
fn success_expires_failure_persists() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(st("a", TaskEvent::Done, t0), t0);
    m.apply_task(st("b", TaskEvent::Failed, t0), t0);
    assert_eq!(m.tasks.len(), 2);
    m.expire(t0 + Duration::from_secs(4));
    assert_eq!(m.tasks.len(), 1);
    assert!(m.tasks[0].failed());
    assert_eq!(m.tasks[0].stderr, vec!["boom", "bang"]);
    m.dismiss_task("b");
    assert!(m.tasks.is_empty());
}

#[test]
fn finished_rows_wait_for_an_away_user() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.set_idle(Duration::from_secs(90), t0);
    m.apply_task(st("a", TaskEvent::Start, t0), t0);
    m.apply_task(st("a", TaskEvent::Done, t0), t0);
    // Away: held well past SUCCESS_HOLD, and the deadline is a short poll.
    let t1 = t0 + Duration::from_secs(600);
    m.set_idle(Duration::from_secs(690), t1);
    assert_eq!(m.expire(t1), Some(t1 + AWAY_POLL));
    assert_eq!(m.tasks.len(), 1);
    // Back (input 1 s ago): the full hold runs from that input, then the row goes.
    let t2 = t1 + Duration::from_secs(5);
    m.set_idle(Duration::from_secs(1), t2);
    assert!(m.expire(t2).is_some());
    assert_eq!(m.tasks.len(), 1);
    m.expire(t2 + SUCCESS_HOLD);
    assert!(m.tasks.is_empty());
}

#[test]
fn a_stalled_download_expires_and_a_live_one_does_not() {
    let t0 = Instant::now();
    let mut m = Model::default();
    let dl = |id: &str, at| match st(id, TaskEvent::Start, at) {
        TaskUpdate::Upsert(mut s) => {
            s.source = "download".into();
            TaskUpdate::Upsert(s)
        }
        u => u,
    };
    m.apply_task(dl("stalled", t0), t0);
    m.apply_task(dl("live", t0), t0);
    let late = t0 + DOWNLOAD_STALE - Duration::from_secs(1);
    m.apply_task(dl("live", late), late); // its file grew
    m.expire(late);
    assert_eq!(m.tasks.len(), 2, "nothing is stale yet");
    let later = t0 + DOWNLOAD_STALE + Duration::from_secs(1);
    m.expire(later);
    assert_eq!(m.tasks.len(), 1);
    assert_eq!(m.tasks[0].id, "live");
    // The stalled one starts changing again: it is back.
    m.apply_task(dl("stalled", later), later);
    assert_eq!(m.tasks.len(), 2);
}

#[test]
fn a_repeated_wait_keeps_its_start() {
    let t0 = Instant::now();
    let t1 = t0 + Duration::from_secs(90);
    let mut m = Model::default();
    m.apply_task(st("a", TaskEvent::Start, t0), t0);
    m.apply_task(st("a", TaskEvent::NeedsInput, t0), t0);
    m.apply_task(st("a", TaskEvent::NeedsInput, t1), t1);
    assert_eq!(m.tasks[0].changed, t0);
    m.apply_task(st("a", TaskEvent::Resumed, t1), t1);
    assert_eq!(m.tasks[0].changed, t1);
}

#[test]
fn running_cannot_be_dismissed() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(st("a", TaskEvent::Start, t0), t0);
    m.dismiss_task("a");
    assert_eq!(m.tasks.len(), 1);
}

#[test]
fn scene_priority() {
    let t0 = Instant::now();
    let mut m = Model::default();
    assert_eq!(m.scene(false, false), Scene::Idle);
    assert_eq!(m.scene(false, true), Scene::Sentinel);
    assert_eq!(m.scene(true, true), Scene::ExpHome);
    m.apply_task(st("a", TaskEvent::Start, t0), t0);
    assert_eq!(m.scene(false, false), Scene::CompactTask);
    assert_eq!(m.scene(true, false), Scene::ExpTasks);
    m.drop_over = true;
    assert_eq!(m.scene(false, false), Scene::ExpDrop);
}

#[test]
fn leaving_a_scene_fades_its_content_out() {
    let mut a = Anim::new(120.0, 24.0);
    a.set_scene(Scene::ExpHome, (380.0, 86.0), ([1.0; 3], 0.1));
    while a.step(1.0 / 60.0) {}
    a.set_scene(Scene::Idle, (120.0, 24.0), ([1.0; 3], 0.0));
    assert_eq!(
        (a.prev, a.out.pos),
        (Scene::ExpHome, 1.0),
        "old content still fully shown"
    );
    a.step(0.05);
    assert!(a.out.pos > 0.0 && a.out.pos < 1.0, "fading, not cut");
    while a.step(1.0 / 60.0) {}
    assert_eq!(a.out.pos, 0.0);
}

#[test]
fn reduced_motion_snaps() {
    let mut a = Anim::new(120.0, 24.0);
    a.reduce = true;
    a.set_scene(Scene::ExpHome, (320.0, 84.0), ([1.0; 3], 0.1));
    assert_eq!(a.rect.w.pos, 320.0);
    assert!(!a.step(0.016));
}

#[test]
fn shell_task_hidden_until_threshold() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(shell("s", "cargo build", TaskEvent::Start, 3000, t0), t0);
    assert_eq!(m.scene(false, false), Scene::Idle, "not shown yet");
    let next = m.expire(t0 + Duration::from_millis(100)).unwrap();
    assert_eq!(
        next,
        t0 + Duration::from_millis(3000),
        "timer armed for the threshold"
    );
    m.expire(t0 + Duration::from_millis(3100));
    assert_eq!(m.scene(false, false), Scene::CompactTask);
}

#[test]
fn fast_shell_command_never_shows() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(shell("s", "ls", TaskEvent::Start, 3000, t0), t0);
    let t1 = t0 + Duration::from_millis(500);
    m.apply_task(shell("s", "ls", TaskEvent::Done, 0, t1), t1);
    assert!(m.tasks.is_empty());
}

#[test]
fn an_agent_ask_shows_over_fullscreen() {
    let t0 = Instant::now();
    let mut m = Model::default();
    assert_eq!(m.scene(false, true), Scene::Sentinel);
    let mut c = Chip::new("Claude Code: Running: cargo build", t0);
    c.extra = vec![("Allow".into(), ChipAction::Answer(1, true))];
    m.chip = Some(c);
    assert_ne!(m.scene(false, true), Scene::Sentinel);
}

#[test]
fn agent_clis_are_left_to_their_hooks() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(shell("s", "claude --resume", TaskEvent::Start, 0, t0), t0);
    m.apply_task(
        shell("o", r"C:in\opencode.exe", TaskEvent::Start, 0, t0),
        t0,
    );
    assert!(m.tasks.is_empty());
}

#[test]
fn ignored_shell_commands_are_dropped() {
    let t0 = Instant::now();
    let mut m = Model {
        ignore: vec!["vim".into()],
        ..Default::default()
    };
    m.apply_task(
        shell("s", r"C:\tools\Vim.exe file.txt", TaskEvent::Start, 0, t0),
        t0,
    );
    assert!(m.tasks.is_empty());
}

#[test]
fn attention_outranks_and_peeks() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(st("a", TaskEvent::Failed, t0), t0);
    let mut u = match st("b", TaskEvent::NeedsInput, t0) {
        TaskUpdate::Upsert(s) => s,
        _ => unreachable!(),
    };
    u.message = Some("approve edit?".into());
    m.apply_task(TaskUpdate::Upsert(u), t0);
    assert_eq!(m.primary().unwrap().id, "b");
    assert!(m.rows()[0].needs_input());
    m.peek(Panel::Tasks, t0 + Duration::from_secs(6));
    assert_eq!(m.scene(false, false), Scene::ExpTasks);
    m.expire(t0 + Duration::from_secs(7));
    assert_eq!(m.scene(false, false), Scene::CompactTask);
    assert!(
        m.tasks.iter().any(|t| t.needs_input()),
        "attention persists after the peek"
    );
}

fn agent_update(
    event: TaskEvent,
    message: Option<&str>,
    info: crate::tasks::AgentInfo,
    now: Instant,
) -> TaskUpdate {
    TaskUpdate::Upsert(TaskState {
        task_id: "agent:Claude Code:7".into(),
        label: "Claude Code".into(),
        source: "agent".into(),
        pid: Some(7),
        message: message.map(Into::into),
        agent: info,
        ..match st("x", event, now) {
            TaskUpdate::Upsert(s) => TaskState {
                stderr_tail: None,
                duration_ms: None,
                ..s
            },
            _ => unreachable!(),
        }
    })
}

fn touched(file: &str) -> crate::tasks::AgentInfo {
    crate::tasks::AgentInfo {
        file: Some(file.into()),
        ..Default::default()
    }
}

#[test]
fn an_agent_turn_counts_distinct_files_subagents_and_context() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(
        agent_update(TaskEvent::Resumed, None, Default::default(), t0),
        t0,
    );
    for f in ["a.rs", "b.rs", "a.rs"] {
        m.apply_task(agent_update(TaskEvent::Progress, None, touched(f), t0), t0);
    }
    let sub = crate::tasks::AgentInfo {
        subagent: true,
        context: Some(84_000),
        ..Default::default()
    };
    m.apply_task(agent_update(TaskEvent::Progress, None, sub, t0), t0);
    // A report with no context keeps the last one.
    m.apply_task(
        agent_update(TaskEvent::Progress, None, Default::default(), t0),
        t0,
    );
    let a = &m.tasks[0].agent;
    assert_eq!(
        (a.files.len(), a.subagents, a.context),
        (2, 1, Some(84_000))
    );
    assert_eq!(a.line().as_deref(), Some("Edited 2 files · 84k context"));
}

#[test]
fn a_new_prompt_starts_the_count_over() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_task(
        agent_update(TaskEvent::Progress, None, touched("a.rs"), t0),
        t0,
    );
    m.apply_task(
        agent_update(TaskEvent::Done, None, Default::default(), t0),
        t0,
    );
    assert_eq!(
        m.tasks[0].agent.files.len(),
        1,
        "the finished row still shows its turn"
    );
    m.apply_task(
        agent_update(TaskEvent::Resumed, None, touched("b.rs"), t0),
        t0,
    );
    assert_eq!(m.tasks[0].agent.files, ["b.rs"]);
}

#[test]
fn the_context_line_names_only_what_is_known() {
    let line = |files: usize, context: Option<u64>| {
        AgentStats {
            files: (0..files).map(|i| format!("f{i}")).collect(),
            context,
            subagents: 0,
        }
        .line()
    };
    assert_eq!(line(0, None), None);
    assert_eq!(line(1, None).as_deref(), Some("Edited 1 file"));
    assert_eq!(line(0, Some(84_000)).as_deref(), Some("84k context"));
    assert_eq!(
        line(4, Some(84_499)).as_deref(),
        Some("Edited 4 files · 84k context")
    );
    assert_eq!(line(0, Some(950)).as_deref(), Some("950 context"));
    assert_eq!(line(0, Some(1_250_000)).as_deref(), Some("1.2M context"));
    assert_eq!(line(0, Some(0)), None);
}

#[test]
fn the_finished_card_carries_the_line_when_something_is_known() {
    let t0 = Instant::now();
    let mut m = Model {
        finish_peek: Duration::from_secs(8),
        ..Default::default()
    };
    m.apply_task(
        agent_update(TaskEvent::Progress, None, touched("a.rs"), t0),
        t0,
    );
    let ctx = crate::tasks::AgentInfo {
        context: Some(84_000),
        ..Default::default()
    };
    m.apply_task(
        agent_update(TaskEvent::Done, Some("All done."), ctx, t0),
        t0,
    );
    let c = m.chip.as_ref().unwrap();
    assert_eq!(c.summary, "Claude Code: All done.");
    assert_eq!(c.note.as_deref(), Some("Edited 1 file · 84k context"));
    assert_eq!(m.size(Scene::ExpChip).1, 80.0 + CHIP_NOTE_H as f64);
    // Nothing known: the card is as it was.
    let mut m = Model {
        finish_peek: Duration::from_secs(8),
        ..Default::default()
    };
    m.apply_task(
        agent_update(TaskEvent::Done, Some("All done."), Default::default(), t0),
        t0,
    );
    let c = m.chip.as_ref().unwrap();
    assert_eq!(
        (c.summary.as_str(), c.note.as_deref()),
        ("Claude Code: All done.", None)
    );
    assert_eq!(m.size(Scene::ExpChip).1, 80.0);
}

fn agent_in(
    id: &str,
    project: &str,
    event: TaskEvent,
    msg: Option<&str>,
    now: Instant,
) -> TaskUpdate {
    let TaskUpdate::Upsert(base) = agent_update(event, msg, Default::default(), now) else {
        unreachable!()
    };
    TaskUpdate::Upsert(TaskState {
        task_id: id.into(),
        cwd: Some(format!(r"C:\src\{project}")),
        ..base
    })
}

#[test]
fn the_agents_pane_shows_only_with_an_agent_and_takes_the_agents_off_the_task_list() {
    let t0 = Instant::now();
    let mut m = no_extra_panes();
    m.now = Some(t0);
    assert!(!m.panels().contains(&Panel::Agents));
    m.apply_task(agent_in("a1", "hytte", TaskEvent::Progress, None, t0), t0);
    assert!(m.panels().contains(&Panel::Agents));
    // The only row is an agent: the Tasks pane has nothing of its own to list.
    assert!(m.rows().is_empty() && !m.panels().contains(&Panel::Tasks));
    assert_eq!(m.agent_count(), 1);
    // A terminal command lists in Tasks, the agent stays in Agents.
    m.apply_task(st("build", TaskEvent::Start, t0), t0);
    m.expire(t0);
    assert_eq!(m.rows().len(), 1);
    assert_eq!(m.agent_tasks().len(), 1);
    assert!(m.panels().contains(&Panel::Tasks) && m.panels().contains(&Panel::Agents));
    // What the collapsed pill shows opens its own pane: a waiting agent leads.
    m.apply_task(
        agent_in("a1", "hytte", TaskEvent::NeedsInput, Some("Run tests?"), t0),
        t0,
    );
    assert_eq!(m.panel(), Panel::Agents);
    // Switched off, agents go back to the Tasks pane as before.
    m.panes.agents = false;
    assert!(!m.panels().contains(&Panel::Agents));
    assert_eq!(m.rows().len(), 2);
    assert_eq!(m.agent_panel(), Panel::Tasks);
}

#[test]
fn agent_rows_put_the_waiting_ones_first_and_say_what_each_is_doing() {
    let t0 = Instant::now();
    let mut m = no_extra_panes();
    m.now = Some(t0);
    let t1 = t0 + Duration::from_secs(5);
    m.apply_task(agent_in("a", "api", TaskEvent::Progress, None, t0), t0);
    m.apply_task(
        agent_in("b", "web", TaskEvent::NeedsInput, Some("Allow rm?"), t0),
        t0,
    );
    let mut busy = agent_in("c", "docs", TaskEvent::Progress, None, t1);
    if let TaskUpdate::Upsert(s) = &mut busy {
        s.line = Some("Editing src/main.rs".into());
        s.agent.file = Some("src/main.rs".into());
        s.agent.context = Some(84_000);
    }
    m.apply_task(busy, t1);
    m.expire(t1);
    let lay = m.agents_layout();
    let ids: Vec<&str> = lay.rows.iter().map(|r| r.task.id.as_str()).collect();
    assert_eq!(ids[0], "b", "waiting first");
    let by = |id: &str| lay.rows.iter().find(|r| r.task.id == id).unwrap();
    assert_eq!(
        by("b").detail,
        Some(AgentDetail::Asking("web · Allow rm?".into()))
    );
    assert_eq!(
        by("c").detail,
        Some(AgentDetail::Step("docs · Editing src/main.rs".into()))
    );
    assert_eq!(by("c").meta.as_deref(), Some("1 file · 84k context"));
    // Quiet agents say their project; no stats, no meta line.
    assert_eq!(by("a").detail, Some(AgentDetail::Step("api".into())));
    assert_eq!(by("a").meta, None);
    // The boxes tile the pane and the pane's height is what they take.
    let mut y = AGENT_PAD_H / 2.0;
    for r in &lay.rows {
        assert_eq!(r.y, y);
        y += r.h;
    }
    assert_eq!(lay.height, (y + AGENT_PAD_H / 2.0) as f64);
    assert_eq!(
        m.size(Scene::ExpAgents).1,
        lay.height + m.tabs() as u8 as f64 * TAB_H as f64
    );
    assert_eq!(lay.hidden, 0);
}

#[test]
fn more_agents_than_rows_fold_into_a_count() {
    let t0 = Instant::now();
    let mut m = no_extra_panes();
    m.now = Some(t0);
    for i in 0..6 {
        m.apply_task(
            agent_in(&format!("a{i}"), "p", TaskEvent::Progress, None, t0),
            t0,
        );
    }
    let lay = m.agents_layout();
    assert_eq!((lay.rows.len(), lay.hidden), (4, 2));
}

#[test]
fn a_project_is_the_last_folder_of_the_path() {
    let t = |cwd: Option<&str>| {
        let now = Instant::now();
        TaskView {
            cwd: cwd.map(Into::into),
            ..TaskView {
                id: "t".into(),
                label: "Claude Code".into(),
                event: TaskEvent::Start,
                progress: None,
                duration_ms: None,
                exit_code: None,
                stderr: vec![],
                pid: None,
                attention: None,
                line: None,
                source: "agent".into(),
                cwd: None,
                agent: Default::default(),
                started: now,
                changed: now,
                visible_after: now,
            }
        }
    };
    assert_eq!(
        project_of(&t(Some(r"C:\src\hytte\"))).as_deref(),
        Some("hytte")
    );
    assert_eq!(project_of(&t(Some("/home/me/app"))).as_deref(), Some("app"));
    assert_eq!(project_of(&t(Some(r"C:\"))), None);
    assert_eq!(project_of(&t(None)), None);
}

#[test]
fn an_agents_turn_is_timed_from_its_prompt_and_starts_over_with_the_next_one() {
    let t0 = Instant::now();
    let mut m = Model::default();
    m.finish_peek = Duration::from_secs(6);
    let at = |s: u64| t0 + Duration::from_secs(s);
    m.apply_task(
        agent_update(TaskEvent::Resumed, None, Default::default(), at(0)),
        at(0),
    );
    // Work, then a question: answering it does not restart the clock.
    m.apply_task(
        agent_update(TaskEvent::Progress, None, Default::default(), at(60)),
        at(60),
    );
    m.apply_task(
        agent_update(
            TaskEvent::NeedsInput,
            Some("ok?"),
            Default::default(),
            at(120),
        ),
        at(120),
    );
    m.apply_task(
        agent_update(TaskEvent::Resumed, None, Default::default(), at(180)),
        at(180),
    );
    assert_eq!(m.tasks[0].started, at(0));
    // It finishes: the row keeps how long it ran, and the card says so.
    m.apply_task(
        agent_update(
            TaskEvent::Done,
            Some("All done."),
            Default::default(),
            at(500),
        ),
        at(500),
    );
    assert_eq!(m.tasks[0].duration_ms, Some(500_000));
    let note = m.chip.as_ref().unwrap().note.clone().unwrap();
    assert!(note.starts_with("Ran 8 min"), "{note}");
    // The row is still there (you were away): the next prompt is a new turn with its own clock.
    m.apply_task(
        agent_update(TaskEvent::Resumed, None, Default::default(), at(2000)),
        at(2000),
    );
    assert_eq!(m.tasks[0].started, at(2000));
    assert_eq!(m.tasks[0].event, TaskEvent::Resumed);
}

#[test]
fn a_working_agent_goes_quiet_after_a_while_and_any_news_clears_it() {
    let t0 = Instant::now();
    let after = Duration::from_secs(300);
    let mut m = Model::default();
    m.apply_task(
        agent_update(TaskEvent::Resumed, None, Default::default(), t0),
        t0,
    );
    let quiet = |m: &Model, s: u64| m.tasks[0].quiet_for(t0 + Duration::from_secs(s), after);
    assert_eq!(quiet(&m, 299), None);
    assert_eq!(quiet(&m, 360), Some(Duration::from_secs(360)));
    // Any hook report is news.
    let t1 = t0 + Duration::from_secs(400);
    m.apply_task(
        agent_update(TaskEvent::Progress, None, Default::default(), t1),
        t1,
    );
    assert_eq!(
        m.tasks[0].quiet_for(t1 + Duration::from_secs(10), after),
        None
    );
    // Off at zero; not for a waiting agent; not for one with no process.
    let far = t1 + Duration::from_secs(9000);
    assert_eq!(m.tasks[0].quiet_for(far, Duration::ZERO), None);
    m.tasks[0].pid = None;
    assert_eq!(m.tasks[0].quiet_for(far, after), None);
    m.tasks[0].pid = Some(7);
    m.tasks[0].event = TaskEvent::NeedsInput;
    assert_eq!(m.tasks[0].quiet_for(far, after), None);
    assert_eq!(
        crate::ui_state::quiet_text(Duration::from_secs(360)),
        "quiet 6m"
    );
    assert_eq!(
        crate::ui_state::quiet_text(Duration::from_secs(7300)),
        "quiet 2h"
    );
}
