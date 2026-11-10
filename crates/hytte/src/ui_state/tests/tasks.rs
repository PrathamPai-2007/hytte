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
    assert_eq!(
        m.chip.as_ref().unwrap().summary,
        "Claude Code: All done.\nEdited 1 file · 84k context"
    );
    // Nothing known: the card is as it was.
    let mut m = Model {
        finish_peek: Duration::from_secs(8),
        ..Default::default()
    };
    m.apply_task(
        agent_update(TaskEvent::Done, Some("All done."), Default::default(), t0),
        t0,
    );
    assert_eq!(m.chip.as_ref().unwrap().summary, "Claude Code: All done.");
}
