use std::time::Duration;

use shiguredo_webrtc::{
    CreateSessionDescriptionObserver, CreateSessionDescriptionObserverHandler, PeerConnection,
    PeerConnectionOfferAnswerOptions, RtcError, RtpTransceiver, SdpType, SessionDescription,
    SetLocalDescriptionObserver, SetLocalDescriptionObserverHandler, SetRemoteDescriptionObserver,
    SetRemoteDescriptionObserverHandler,
};
use tokio::sync::mpsc;

use crate::event::IceObserverEvent;

pub const SDP_TIMEOUT: Duration = Duration::from_secs(5);

struct CreateSdpHandler {
    tx: std::sync::mpsc::Sender<Result<String, String>>,
    is_offer: bool,
}

impl CreateSessionDescriptionObserverHandler for CreateSdpHandler {
    fn on_success(&mut self, desc: SessionDescription) {
        let sdp = desc
            .to_string()
            .map_err(|e| format!("failed to convert SDP to string: {e}"));
        let _ = self.tx.send(sdp);
    }

    fn on_failure(&mut self, error: RtcError) {
        let message = error.message().unwrap_or_else(|_| "unknown".to_owned());
        let kind = if self.is_offer { "offer" } else { "answer" };
        let _ = self
            .tx
            .send(Err(format!("create_{kind} failed: {message}")));
    }
}

pub fn create_offer_sdp(pc: &PeerConnection) -> Result<String, String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    options.set_offer_to_receive_audio(1);
    options.set_offer_to_receive_video(1);
    let (tx, rx) = std::sync::mpsc::channel();
    let mut observer =
        CreateSessionDescriptionObserver::new_with_handler(Box::new(CreateSdpHandler {
            tx,
            is_offer: true,
        }));
    pc.create_offer(&mut observer, &mut options);
    rx.recv_timeout(SDP_TIMEOUT)
        .map_err(|_| "create_offer timed out".to_owned())?
}

pub fn create_answer_sdp(pc: &PeerConnection) -> Result<String, String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut observer =
        CreateSessionDescriptionObserver::new_with_handler(Box::new(CreateSdpHandler {
            tx,
            is_offer: false,
        }));
    pc.create_answer(&mut observer, &mut options);
    rx.recv_timeout(SDP_TIMEOUT)
        .map_err(|_| "create_answer timed out".to_owned())?
}

struct SetLocalSdpHandler {
    tx: std::sync::mpsc::Sender<Option<String>>,
}

impl SetLocalDescriptionObserverHandler for SetLocalSdpHandler {
    fn on_set_local_description_complete(&mut self, error: RtcError) {
        let message = if error.ok() {
            None
        } else {
            Some(error.message().unwrap_or_else(|_| "unknown".to_owned()))
        };
        let _ = self.tx.send(message);
    }
}

struct SetRemoteSdpHandler {
    tx: std::sync::mpsc::Sender<Option<String>>,
}

impl SetRemoteDescriptionObserverHandler for SetRemoteSdpHandler {
    fn on_set_remote_description_complete(&mut self, error: RtcError) {
        let message = if error.ok() {
            None
        } else {
            Some(error.message().unwrap_or_else(|_| "unknown".to_owned()))
        };
        let _ = self.tx.send(message);
    }
}

pub fn set_local_description(
    pc: &PeerConnection,
    sdp_type: SdpType,
    sdp: &str,
) -> Result<(), String> {
    let description =
        SessionDescription::new(sdp_type, sdp).map_err(|e| format!("failed to parse SDP: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let observer =
        SetLocalDescriptionObserver::new_with_handler(Box::new(SetLocalSdpHandler { tx }));
    pc.set_local_description(description, &observer);
    let result = rx
        .recv_timeout(SDP_TIMEOUT)
        .map_err(|_| "set_local_description timed out".to_owned())?;
    if let Some(message) = result {
        return Err(format!("set_local_description failed: {message}"));
    }
    Ok(())
}

pub fn set_remote_description(
    pc: &PeerConnection,
    sdp_type: SdpType,
    sdp: &str,
) -> Result<(), String> {
    let description =
        SessionDescription::new(sdp_type, sdp).map_err(|e| format!("failed to parse SDP: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let observer =
        SetRemoteDescriptionObserver::new_with_handler(Box::new(SetRemoteSdpHandler { tx }));
    pc.set_remote_description(description, &observer);
    let result = rx
        .recv_timeout(SDP_TIMEOUT)
        .map_err(|_| "set_remote_description timed out".to_owned())?;
    if let Some(message) = result {
        return Err(format!("set_remote_description failed: {message}"));
    }
    Ok(())
}

// --- ICE candidates ---

#[derive(Clone)]
pub struct GatheredIceCandidate {
    pub sdp_mid: String,
    pub sdp_mline_index: i32,
    pub candidate: String,
}

pub async fn finalize_local_sdp(
    initial_sdp: String,
    ice_rx: &mut mpsc::UnboundedReceiver<IceObserverEvent>,
    cached_candidates: &mut Vec<GatheredIceCandidate>,
) -> Result<String, String> {
    if initial_sdp.contains("\r\na=candidate:") {
        return Ok(initial_sdp);
    }

    let mut candidates = Vec::new();
    let mut complete = false;
    // まずノンブロッキングで既に到着しているイベントを処理する
    while let Ok(event) = ice_rx.try_recv() {
        match event {
            IceObserverEvent::Candidate {
                sdp_mid,
                sdp_mline_index,
                candidate,
            } => {
                candidates.push(GatheredIceCandidate {
                    sdp_mid,
                    sdp_mline_index,
                    candidate,
                });
            }
            IceObserverEvent::Complete => {
                complete = true;
            }
        }
    }

    if !complete && candidates.is_empty() && !cached_candidates.is_empty() {
        return Ok(append_ice_candidates_to_sdp(
            &initial_sdp,
            cached_candidates,
        ));
    }

    // タイムアウト付きで ICE gathering 完了を待機する
    let deadline = tokio::time::Instant::now() + SDP_TIMEOUT;
    while !complete {
        match tokio::time::timeout_at(deadline, ice_rx.recv()).await {
            Ok(Some(IceObserverEvent::Candidate {
                sdp_mid,
                sdp_mline_index,
                candidate,
            })) => {
                candidates.push(GatheredIceCandidate {
                    sdp_mid,
                    sdp_mline_index,
                    candidate,
                });
            }
            Ok(Some(IceObserverEvent::Complete)) => {
                complete = true;
            }
            Ok(None) => {
                // チャネルが切断された
                return Err("ICE gathering channel closed".to_owned());
            }
            Err(_) => {
                // タイムアウト
                if !cached_candidates.is_empty() {
                    return Ok(append_ice_candidates_to_sdp(
                        &initial_sdp,
                        cached_candidates,
                    ));
                }
                return Err("ICE gathering timed out".to_owned());
            }
        }
    }

    if !candidates.is_empty() {
        *cached_candidates = candidates.clone();
    }

    Ok(append_ice_candidates_to_sdp(
        &initial_sdp,
        if candidates.is_empty() {
            cached_candidates
        } else {
            &candidates
        },
    ))
}

fn append_ice_candidates_to_sdp(sdp: &str, candidates: &[GatheredIceCandidate]) -> String {
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut current_section = Vec::new();

    for line in sdp.split("\r\n").filter(|line| !line.is_empty()) {
        if line.starts_with("m=") && !current_section.is_empty() {
            sections.push(current_section);
            current_section = Vec::new();
        }
        current_section.push(line.to_owned());
    }
    if !current_section.is_empty() {
        sections.push(current_section);
    }

    let mut output = Vec::new();
    for (index, section) in sections.into_iter().enumerate() {
        let is_media_section = section.first().is_some_and(|line| line.starts_with("m="));
        let sdp_mid = section
            .iter()
            .find_map(|line| line.strip_prefix("a=mid:"))
            .unwrap_or_default();

        for line in &section {
            output.push(line.clone());
        }

        if is_media_section {
            let section_candidates: Vec<&GatheredIceCandidate> = candidates
                .iter()
                .filter(|candidate| {
                    candidate.sdp_mid == sdp_mid || candidate.sdp_mline_index == index as i32 - 1
                })
                .collect();
            if !section_candidates.is_empty() {
                for candidate in section_candidates {
                    output.push(format!("a={}", candidate.candidate));
                }
                output.push("a=end-of-candidates".to_owned());
            }
        }
    }

    output.join("\r\n") + "\r\n"
}

// --- SDP ロギング ---

pub fn summarize_sdp_for_log(sdp: &str) -> String {
    let mut sections = Vec::new();
    let mut current = Vec::new();
    for line in sdp.split("\r\n").filter(|line| !line.is_empty()) {
        if line.starts_with("m=") && !current.is_empty() {
            sections.push(current);
            current = Vec::new();
        }
        current.push(line);
    }
    if !current.is_empty() {
        sections.push(current);
    }

    let mut summary = Vec::new();
    for section in sections {
        let Some(first_line) = section.first() else {
            continue;
        };
        if !(first_line.starts_with("m=audio")
            || first_line.starts_with("m=video")
            || first_line.starts_with("m=application"))
        {
            continue;
        }
        summary.push((*first_line).to_owned());
        for line in &section {
            if line.starts_with("a=mid:")
                || line == &"a=sendrecv"
                || line == &"a=sendonly"
                || line == &"a=recvonly"
                || line == &"a=inactive"
                || line.starts_with("a=rtpmap:")
                || line.starts_with("a=fmtp:")
            {
                summary.push(format!("  {line}"));
            }
        }
    }
    summary.join("\n")
}

pub fn log_sdp_summary(label: &str, sdp: &str) {
    tracing::info!("{label}:\n{}", summarize_sdp_for_log(sdp));
}

pub fn log_transceiver_receiver_state(label: &str, transceiver: &RtpTransceiver) {
    let receiver = transceiver.receiver();
    let track = receiver.track();
    let kind = track.kind().unwrap_or_default();
    let track_id = track.id().unwrap_or_default();
    tracing::info!("{label}: receiver_track_kind={kind}, receiver_track_id={track_id}");
}
