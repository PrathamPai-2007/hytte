//! Media cockpit via GSMTC.
//! Event-driven: WinRT callbacks only poke a channel; one thread re-reads
//! state, so handlers stay trivial. A 30 s timeout is a safety net, not a
//! poll loop. Album art is decoded once per track to 64x64 premultiplied BGRA.

use crate::ui_state::UiEvent;
use crossbeam_channel::Sender;

#[derive(Debug, Clone, Copy)]
pub enum Cmd {
    Prev,
    Toggle,
    Next,
}

pub fn spawn_watcher(ui_tx: Sender<UiEvent>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        imp::run(ui_tx);
        #[cfg(not(windows))]
        {
            let _ = ui_tx;
        }
    });
}

/// Fire-and-forget transport command (runs off the UI thread).
pub fn control(cmd: Cmd) {
    #[cfg(windows)]
    std::thread::spawn(move || imp::control(cmd));
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Resize + premultiply decoded cover art for Direct2D (BGRA).
pub fn art_from_bytes(bytes: &[u8]) -> Option<crate::ui_state::ArtBitmap> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = img
        .resize_to_fill(64, 64, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let mut bgra = Vec::with_capacity(64 * 64 * 4);
    for p in img.pixels() {
        let a = p[3] as u32;
        let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        bgra.extend_from_slice(&[m(p[2]), m(p[1]), m(p[0]), p[3]]);
    }
    Some(crate::ui_state::ArtBitmap { w: 64, h: 64, bgra })
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::ui_state::MediaInfo;
    use crate::winrt::block_on;
    use crossbeam_channel::unbounded;
    use std::sync::Arc;
    use std::time::Duration;
    use windows::Foundation::TypedEventHandler;
    use windows::Media::Control::{
        CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
        MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs,
        TimelinePropertiesChangedEventArgs,
    };
    use windows::Storage::Streams::{DataReader, IInputStream};
    use windows_core::Interface;

    const T: Duration = Duration::from_secs(3);

    fn manager() -> Option<Manager> {
        block_on(Manager::RequestAsync().ok()?, T)?.ok()
    }

    struct Sub {
        session: Session,
        tokens: [i64; 3],
    }

    impl Sub {
        fn new(session: Session, poke: &Sender<()>) -> Option<Self> {
            let p1 = poke.clone();
            let p2 = poke.clone();
            let p3 = poke.clone();
            let a = session
                .MediaPropertiesChanged(&TypedEventHandler::<
                    Session,
                    MediaPropertiesChangedEventArgs,
                >::new(move |_, _| {
                    let _ = p1.send(());
                    Ok(())
                }))
                .ok()?;
            let b = session
                .PlaybackInfoChanged(
                    &TypedEventHandler::<Session, PlaybackInfoChangedEventArgs>::new(
                        move |_, _| {
                            let _ = p2.send(());
                            Ok(())
                        },
                    ),
                )
                .ok()?;
            let c = session
                .TimelinePropertiesChanged(&TypedEventHandler::<
                    Session,
                    TimelinePropertiesChangedEventArgs,
                >::new(move |_, _| {
                    let _ = p3.send(());
                    Ok(())
                }))
                .ok()?;
            Some(Self {
                session,
                tokens: [a, b, c],
            })
        }
    }

    impl Drop for Sub {
        fn drop(&mut self) {
            let _ = self.session.RemoveMediaPropertiesChanged(self.tokens[0]);
            let _ = self.session.RemovePlaybackInfoChanged(self.tokens[1]);
            let _ = self.session.RemoveTimelinePropertiesChanged(self.tokens[2]);
        }
    }

    pub fn run(ui_tx: Sender<UiEvent>) {
        let (poke, wake) = unbounded::<()>();
        let mgr = loop {
            if let Some(m) = manager() {
                break m;
            }
            std::thread::sleep(Duration::from_secs(5));
        };
        let p = poke.clone();
        let _ = mgr.CurrentSessionChanged(&TypedEventHandler::<
            Manager,
            CurrentSessionChangedEventArgs,
        >::new(move |_, _| {
            let _ = p.send(());
            Ok(())
        }));
        let mut sub: Option<Sub> = None;
        let mut sub_id = String::new();
        let mut art_key = String::new();
        let mut last: Option<Option<MediaInfo>> = None;
        loop {
            match mgr.GetCurrentSession() {
                Ok(s) => {
                    let id = s
                        .SourceAppUserModelId()
                        .map(|h| h.to_string())
                        .unwrap_or_default();
                    if sub.is_none() || id != sub_id {
                        sub = Sub::new(s.clone(), &poke);
                        sub_id = id;
                    }
                    if let Some(info) = read(&s) {
                        let key = format!("{}\u{1}{}\u{1}{}", sub_id, info.0.title, info.0.artist);
                        if key != art_key {
                            art_key = key;
                            let art = thumbnail(&info.1)
                                .and_then(|b| art_from_bytes(&b))
                                .map(Arc::new);
                            let _ = ui_tx.send(UiEvent::MediaArt(art));
                        }
                        let m = Some(info.0);
                        if last.as_ref() != Some(&m) {
                            last = Some(m.clone());
                            let _ = ui_tx.send(UiEvent::Media(m));
                        }
                    }
                }
                Err(_) => {
                    sub = None;
                    art_key.clear();
                    if last.as_ref() != Some(&None) {
                        last = Some(None);
                        let _ = ui_tx.send(UiEvent::Media(None));
                    }
                }
            }
            // Block until a WinRT event pokes us; coalesce bursts.
            let _ = wake.recv_timeout(Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(60));
            while wake.try_recv().is_ok() {}
        }
    }

    type Props = windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties;

    fn read(s: &Session) -> Option<(MediaInfo, Props)> {
        let playing = s.GetPlaybackInfo().ok()?.PlaybackStatus().ok()? == Status::Playing;
        let props = block_on(s.TryGetMediaPropertiesAsync().ok()?, T)?.ok()?;
        let title = props
            .Title()
            .ok()
            .map(|h| h.to_string())
            .unwrap_or_default();
        let artist = props
            .Artist()
            .ok()
            .map(|h| h.to_string())
            .unwrap_or_default();
        let (mut pos_ms, mut dur_ms) = (0u64, 0u64);
        if let Ok(tl) = s.GetTimelineProperties() {
            let ms = |t: windows::Foundation::TimeSpan| (t.Duration.max(0) / 10_000) as u64;
            let start = tl.StartTime().map(ms).unwrap_or(0);
            dur_ms = tl.EndTime().map(ms).unwrap_or(0).saturating_sub(start);
            pos_ms = tl.Position().map(ms).unwrap_or(0).saturating_sub(start);
        }
        let app = s
            .SourceAppUserModelId()
            .map(|h| h.to_string())
            .unwrap_or_default();
        Some((
            MediaInfo {
                title,
                artist,
                playing,
                app,
                pos_ms,
                dur_ms,
            },
            props,
        ))
    }

    fn thumbnail(props: &Props) -> Option<Vec<u8>> {
        let r = props.Thumbnail().ok()?;
        let stream = block_on(r.OpenReadAsync().ok()?, T)?.ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > 8 * 1024 * 1024 {
            return None;
        }
        let input: IInputStream = stream.cast().ok()?;
        let reader = DataReader::CreateDataReader(&input).ok()?;
        block_on(reader.LoadAsync(size as u32).ok()?, T)?.ok()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf).ok()?;
        Some(buf)
    }

    pub fn control(cmd: Cmd) {
        let Some(mgr) = manager() else { return };
        let Ok(s) = mgr.GetCurrentSession() else {
            return;
        };
        match cmd {
            Cmd::Prev => {
                if let Ok(op) = s.TrySkipPreviousAsync() {
                    let _ = block_on(op, T);
                }
            }
            Cmd::Toggle => {
                if let Ok(op) = s.TryTogglePlayPauseAsync() {
                    let _ = block_on(op, T);
                }
            }
            Cmd::Next => {
                if let Ok(op) = s.TrySkipNextAsync() {
                    let _ = block_on(op, T);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn art_decodes_and_premultiplies() {
        let mut png = vec![];
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            10,
            20,
            image::Rgba([200, 100, 50, 128]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
        let a = super::art_from_bytes(&png).unwrap();
        assert_eq!((a.w, a.h, a.bgra.len()), (64, 64, 64 * 64 * 4));
        assert_eq!(a.bgra[3], 128);
        assert_eq!(a.bgra[0], 25); // 50 * 128/255
    }
}
