use std::time::Duration;

use shiguredo_webrtc::{DataChannelState, PeerConnection};
use tokio::sync::{mpsc, oneshot};

use crate::event::ClientEvent;
use crate::obsws_message::{
    make_get_webrtc_stats_request, parse_obsws_server_webrtc_stats_response,
};
use crate::state::RetainedState;

pub struct Stats {
    pub video_tracks: usize,
    pub audio_tracks: usize,
    pub video_frames: usize,
    pub audio_frames: usize,
    pub video_width: usize,
    pub video_height: usize,
    pub video_codec: String,
    pub audio_codec: String,
    pub video_samples_written: usize,
    pub audio_samples_written: usize,
    pub connection_state: String,
    pub webrtc_stats_error: String,
    pub program_tracks_subscribed: bool,
}

pub async fn collect_webrtc_stats_json(pc: &PeerConnection) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();
    pc.get_stats(move |report| {
        let _ = tx.send(
            report
                .to_json()
                .map_err(|e| format!("failed to serialize WebRTC stats: {e}")),
        );
    });

    tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .map_err(|_| "timed out waiting for WebRTC stats".to_owned())?
        .map_err(|_| "WebRTC stats callback channel closed".to_owned())?
}

pub async fn request_server_webrtc_stats(
    retained: &RetainedState,
    event_rx: &mut mpsc::UnboundedReceiver<ClientEvent>,
) -> Result<String, String> {
    let Some(dc) = retained.obsws_dc.as_ref() else {
        return Err("obsws DataChannel is not available".to_owned());
    };
    if dc.state() != DataChannelState::Open {
        return Err("obsws DataChannel is not open".to_owned());
    }

    let request = make_get_webrtc_stats_request();
    if !dc.send(request.as_bytes(), false) {
        return Err("failed to send GetWebRtcStats request".to_owned());
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for GetWebRtcStats response".to_owned());
        }

        let event = tokio::time::timeout(remaining, event_rx.recv())
            .await
            .map_err(|_| "timed out waiting for GetWebRtcStats response".to_owned())?
            .ok_or_else(|| {
                "event channel closed while waiting for GetWebRtcStats response".to_owned()
            })?;

        if let ClientEvent::ObswsMessage { data } = event {
            let text = std::str::from_utf8(&data)
                .map_err(|e| format!("GetWebRtcStats response was not UTF-8: {e}"))?;
            if let Some(result) = parse_obsws_server_webrtc_stats_response(text) {
                return result;
            }
        }
    }
}

pub fn summarize_webrtc_stats_json(stats_json: &str) -> String {
    let Ok(json) = nojson::RawJson::parse(stats_json) else {
        return "failed to parse stats json".to_owned();
    };

    let mut inbound_audio = 0usize;
    let mut inbound_video = 0usize;
    let mut outbound_audio = 0usize;
    let mut outbound_video = 0usize;
    let mut remote_inbound_audio = 0usize;
    let mut remote_inbound_video = 0usize;
    let mut remote_outbound_audio = 0usize;
    let mut remote_outbound_video = 0usize;
    let mut audio_codecs = Vec::new();
    let mut video_codecs = Vec::new();

    let root = json.value();
    let Ok(stats_objects) = root.to_object() else {
        return "stats root is not an object".to_owned();
    };

    for (_, stats) in stats_objects {
        let Ok(stats_type) = stats
            .to_member("type")
            .and_then(|v| v.required())
            .and_then(|v| v.to_unquoted_string_str())
        else {
            continue;
        };

        let mut kind = None;
        if let Ok(kind_member) = stats.to_member("kind")
            && let Some(kind_value) = kind_member.optional()
            && let Ok(kind_str) = kind_value.to_unquoted_string_str()
        {
            kind = Some(kind_str.to_string());
        }

        match (stats_type.as_ref(), kind.as_deref()) {
            ("inbound-rtp", Some("audio")) => inbound_audio += 1,
            ("inbound-rtp", Some("video")) => inbound_video += 1,
            ("outbound-rtp", Some("audio")) => outbound_audio += 1,
            ("outbound-rtp", Some("video")) => outbound_video += 1,
            ("remote-inbound-rtp", Some("audio")) => remote_inbound_audio += 1,
            ("remote-inbound-rtp", Some("video")) => remote_inbound_video += 1,
            ("remote-outbound-rtp", Some("audio")) => remote_outbound_audio += 1,
            ("remote-outbound-rtp", Some("video")) => remote_outbound_video += 1,
            ("codec", _) => {
                let mut mime_type = None;
                if let Ok(mime_type_member) = stats.to_member("mimeType")
                    && let Some(mime_type_value) = mime_type_member.optional()
                    && let Ok(mime_type_str) = mime_type_value.to_unquoted_string_str()
                {
                    mime_type = Some(mime_type_str.to_string());
                }
                if let Some(mime_type) = mime_type {
                    if mime_type.starts_with("audio/") {
                        audio_codecs.push(mime_type);
                    } else if mime_type.starts_with("video/") {
                        video_codecs.push(mime_type);
                    }
                }
            }
            _ => {}
        }
    }

    audio_codecs.sort();
    audio_codecs.dedup();
    video_codecs.sort();
    video_codecs.dedup();

    format!(
        "inbound_audio={inbound_audio}, inbound_video={inbound_video}, outbound_audio={outbound_audio}, outbound_video={outbound_video}, remote_inbound_audio={remote_inbound_audio}, remote_inbound_video={remote_inbound_video}, remote_outbound_audio={remote_outbound_audio}, remote_outbound_video={remote_outbound_video}, audio_codecs=[{}], video_codecs=[{}]",
        audio_codecs.join(", "),
        video_codecs.join(", ")
    )
}
