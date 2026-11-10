use super::*;

mod cards;
mod media;
mod panes;
mod scene;
mod scripts;
mod tasks;

pub(super) fn st(id: &str, ev: TaskEvent, now: Instant) -> TaskUpdate {
    TaskUpdate::Upsert(TaskState {
        task_id: id.into(),
        label: "build".into(),
        progress: None,
        event: ev,
        duration_ms: Some(10),
        stderr_tail: Some(
            "boom
bang"
                .into(),
        ),
        source: "run".into(),
        pid: None,
        delay_ms: 0,
        message: None,
        exit_code: None,
        line: None,
        cwd: None,
        agent: Default::default(),
        updated: now,
    })
}

pub(super) fn shell(id: &str, label: &str, ev: TaskEvent, delay: u32, now: Instant) -> TaskUpdate {
    TaskUpdate::Upsert(TaskState {
        task_id: id.into(),
        label: label.into(),
        progress: None,
        event: ev,
        duration_ms: None,
        stderr_tail: None,
        source: "shell".into(),
        pid: Some(1),
        delay_ms: delay,
        message: None,
        exit_code: None,
        line: None,
        cwd: None,
        agent: Default::default(),
        updated: now,
    })
}

pub(super) fn song(title: &str, playing: bool, pos: u64) -> MediaInfo {
    MediaInfo {
        title: title.into(),
        artist: "Band".into(),
        album: "Album".into(),
        playing,
        app: "Spotify.exe".into(),
        pos_ms: pos,
        dur_ms: 200_000,
    }
}

pub(super) fn focus_timer() -> crate::timer::Timer {
    crate::timer::Timer::start(crate::timer::TimerKind::Focus, 25, crate::timer::now_ms())
}

/// A model showing only the panes that exist without Notes, To-do and Weather.
fn no_extra_panes() -> Model {
    let mut m = Model::default();
    m.panes = crate::config::Panes {
        notes: false,
        todo: false,
        weather: false,
        ..Default::default()
    };
    m
}

pub(super) fn battery(plugged: bool) -> crate::power::Battery {
    crate::power::Battery {
        pct: 60,
        plugged,
        watts: None,
        mode: crate::power::Mode::Balanced,
        mins_left: None,
    }
}

pub(super) fn pane(name: &str, command: &str) -> crate::config::ScriptPane {
    crate::config::ScriptPane {
        name: name.into(),
        command: command.into(),
        every: 60,
        ..Default::default()
    }
}

pub(super) fn card(id: &str) -> hytte_proto::Cmd {
    hytte_proto::Cmd {
        id: Some(id.into()),
        ..hytte_proto::Cmd::card()
    }
}
