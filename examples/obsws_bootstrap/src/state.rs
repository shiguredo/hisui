use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::Duration;

use shiguredo_webrtc::{
    AudioTrackSink, DataChannel, DataChannelObserver, PeerConnection, PeerConnectionObserver,
    RtpTransceiver, VideoSink, VideoSinkWants,
};
use tokio::sync::mpsc;

use crate::adm::BootstrapAudioDeviceModuleState;
use crate::event::{AudioFrameData, IceObserverEvent, VideoFrameData};
use crate::observer::{AudioRecordHandler, FrameRecordHandler};
use crate::sdp::GatheredIceCandidate;

pub struct RetainedState {
    pub _pc_observer: PeerConnectionObserver,
    pub dummy_dc: DataChannel,
    pub obsws_dc: Option<DataChannel>,
    pub signaling_dc: Option<DataChannel>,
    pub signaling_dc_observer: Option<DataChannelObserver>,
    pub obsws_dc_observer: Option<DataChannelObserver>,
    pub video_sinks: Vec<RetainedVideoSink>,
    pub audio_sinks: Vec<RetainedAudioSink>,
    pub track_transceivers: Vec<RtpTransceiver>,
    pub ice_rx: mpsc::UnboundedReceiver<IceObserverEvent>,
    pub ice_candidates: Vec<GatheredIceCandidate>,
}

pub struct RetainedVideoSink {
    pub track_id: String,
    pub track: shiguredo_webrtc::VideoTrack,
    pub sink: VideoSink,
}

pub struct RetainedAudioSink {
    pub track_id: String,
    pub track: shiguredo_webrtc::AudioTrack,
    pub sink: AudioTrackSink,
}

pub struct VideoSinkAttachState<'a> {
    pub video_frames: &'a Arc<AtomicUsize>,
    pub first_video_frame_logged: &'a Arc<AtomicBool>,
    pub frame_tx: &'a std::sync::mpsc::SyncSender<VideoFrameData>,
}

pub fn attach_video_sink(
    retained: &mut RetainedState,
    track_id: &str,
    video_track: shiguredo_webrtc::VideoTrack,
    state: &VideoSinkAttachState<'_>,
) {
    if retained
        .video_sinks
        .iter()
        .any(|retained_sink| retained_sink.track_id == track_id)
    {
        return;
    }
    let mut video_track = video_track;
    let sink = VideoSink::new_with_handler(Box::new(FrameRecordHandler {
        track_id: track_id.to_owned(),
        frame_count: state.video_frames.clone(),
        first_frame_logged: state.first_video_frame_logged.clone(),
        frame_tx: state.frame_tx.clone(),
    }));
    let wants = VideoSinkWants::default();
    video_track.add_or_update_sink(&sink, &wants);
    retained.video_sinks.push(RetainedVideoSink {
        track_id: track_id.to_owned(),
        track: video_track,
        sink,
    });
}

pub fn attach_audio_sink(
    retained: &mut RetainedState,
    track_id: &str,
    audio_track: shiguredo_webrtc::AudioTrack,
    audio_frames: &Arc<AtomicUsize>,
    audio_tx: &std::sync::mpsc::SyncSender<AudioFrameData>,
) {
    if retained
        .audio_sinks
        .iter()
        .any(|retained_sink| retained_sink.track_id == track_id)
    {
        return;
    }
    let mut audio_track = audio_track;
    let sink = AudioTrackSink::new_with_handler(Box::new(AudioRecordHandler {
        track_id: track_id.to_owned(),
        audio_frame_count: audio_frames.clone(),
        audio_tx: audio_tx.clone(),
    }));
    audio_track.add_sink(&sink);
    retained.audio_sinks.push(RetainedAudioSink {
        track_id: track_id.to_owned(),
        track: audio_track,
        sink,
    });
}

pub fn should_write_video_frame(
    subscribe_program_tracks: bool,
    track_id: &str,
    program_video_track_id: Option<&str>,
) -> bool {
    if subscribe_program_tracks {
        program_video_track_id == Some(track_id)
    } else {
        true
    }
}

pub fn should_write_audio_frame(
    subscribe_program_tracks: bool,
    track_id: &str,
    program_audio_track_id: Option<&str>,
) -> bool {
    if subscribe_program_tracks {
        program_audio_track_id == Some(track_id)
    } else {
        true
    }
}

pub async fn teardown_client(
    pc: &PeerConnection,
    retained: &mut RetainedState,
    audio_state: &BootstrapAudioDeviceModuleState,
) {
    for retained_sink in &mut retained.video_sinks {
        retained_sink.track.remove_sink(&retained_sink.sink);
    }
    for retained_sink in &mut retained.audio_sinks {
        retained_sink.track.remove_sink(&retained_sink.sink);
    }

    if let Some(dc) = retained.obsws_dc.as_ref() {
        dc.unregister_observer();
        dc.close();
    }
    if let Some(dc) = retained.signaling_dc.as_ref() {
        dc.unregister_observer();
        dc.close();
    }
    retained.dummy_dc.close();

    pc.close();
    audio_state.shutdown();

    // close 後の非同期コールバックが収束するまで少し待つ。
    tokio::time::sleep(Duration::from_millis(100)).await;
}
