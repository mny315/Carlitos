mod events;
mod transport;

use crate::library::Millis;
use crate::player::Phase;
use anyhow::{Context, Result};
use gst::prelude::*;

#[derive(Clone, Debug)]
pub enum EventKind {
    AsyncDone(gst::Seqnum),
    Eos(gst::Seqnum),
    Error(String),
    State,
    Duration,
}
#[derive(Clone, Debug)]
pub struct Event {
    pub generation: u64,
    pub kind: EventKind,
}
pub struct Player {
    pipeline: Option<gst::Element>,
    watch: Option<gst::bus::BusWatchGuard>,
    sender: async_channel::Sender<Event>,
    pub generation: u64,
    pub phase: Phase,
    pub playing: bool,
    pub ended: bool,
    pub position: Millis,
    pub duration: Option<Millis>,
    pub seekable: bool,
    requested_seek: Option<Millis>,
    inflight_seek: Option<Millis>,
    seek_seqnum: Option<gst::Seqnum>,
    volume: f64,
    muted: bool,
    rate: f64,
    skip_silence: bool,
    silence: Option<crate::silence::Silence>,
    fake_sink: bool,
}
impl Player {
    pub fn new(sender: async_channel::Sender<Event>, fake_sink: bool) -> Result<Self> {
        gst::init()?;
        // Detect incomplete installations before playbin starts an error/retry loop.
        for required in [
            "playbin3",
            "filesrc",
            "typefind",
            "queue",
            "audioconvert",
            "audioresample",
            "volume",
            "scaletempo",
        ] {
            gst::ElementFactory::find(required)
                .with_context(|| tformat!("Не установлен элемент GStreamer: {}", required))?;
        }
        Ok(Self {
            pipeline: None,
            watch: None,
            sender,
            generation: 0,
            phase: Phase::Empty,
            playing: false,
            ended: false,
            position: 0,
            duration: None,
            seekable: false,
            requested_seek: None,
            inflight_seek: None,
            seek_seqnum: None,
            volume: 0.7,
            muted: false,
            rate: 1.0,
            skip_silence: false,
            silence: None,
            fake_sink,
        })
    }
    pub fn load(&mut self, uri: &str, position: Millis, playing: bool) -> Result<()> {
        self.stop();
        self.generation = self.generation.wrapping_add(1);
        // Even an early construction failure must be retryable from the UI.
        self.phase = Phase::Error;
        self.position = position;
        let pipeline = gst::ElementFactory::make("playbin3")
            .build()
            .context(crate::i18n::tr("Не установлен GStreamer playbin3"))?;
        pipeline.set_property("uri", uri);
        pipeline.set_property("volume", self.volume);
        pipeline.set_property("mute", self.muted);
        let (filter, silence) = crate::silence::Silence::filter(self.skip_silence)?;
        pipeline.set_property("audio-filter", &filter);
        self.silence = Some(silence);
        // Audio only: avoid video windows for containers that also contain cover art/video tracks.
        pipeline.set_property_from_str("flags", "audio+soft-volume+buffering");
        if self.fake_sink {
            let sink = gst::ElementFactory::make("fakesink")
                .property("sync", true)
                .build()?;
            pipeline.set_property("audio-sink", &sink);
        }
        let generation = self.generation;
        let sender = self.sender.clone();
        self.watch = Some(
            pipeline
                .bus()
                .context(crate::i18n::tr("Аудиодвижок не создал шину событий"))?
                .add_watch_local(move |_, message| {
                    let kind = match message.view() {
                        gst::MessageView::AsyncDone(_) => {
                            Some(EventKind::AsyncDone(message.seqnum()))
                        }
                        gst::MessageView::Eos(_) => Some(EventKind::Eos(message.seqnum())),
                        gst::MessageView::Error(e) => {
                            Some(EventKind::Error(format!("{}", e.error())))
                        }
                        gst::MessageView::StateChanged(_) => Some(EventKind::State),
                        gst::MessageView::DurationChanged(_) => Some(EventKind::Duration),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        let _ = sender.try_send(Event { generation, kind });
                    }
                    gst::glib::ControlFlow::Continue
                })?,
        );
        self.phase = Phase::Loading;
        self.position = position;
        self.requested_seek = Some(position);
        self.playing = playing;
        self.pipeline = Some(pipeline.clone());
        if let Err(e) = pipeline.set_state(gst::State::Paused) {
            self.phase = Phase::Error;
            self.playing = false;
            return Err(e.into());
        }
        Ok(())
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests;
