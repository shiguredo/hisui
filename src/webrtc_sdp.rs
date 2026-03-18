use std::time::Duration;

use shiguredo_webrtc::{
    CreateSessionDescriptionObserver, CreateSessionDescriptionObserverHandler, PeerConnection,
    PeerConnectionOfferAnswerOptions, RtcError, SdpType, SessionDescription,
    SetLocalDescriptionObserver, SetLocalDescriptionObserverHandler, SetRemoteDescriptionObserver,
    SetRemoteDescriptionObserverHandler,
};

const SDP_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn create_offer_sdp(pc: &PeerConnection) -> crate::Result<String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    options.set_offer_to_receive_audio(0);
    options.set_offer_to_receive_video(0);
    create_sdp(pc, &mut options, true)
}

pub(crate) fn create_answer_sdp(pc: &PeerConnection) -> crate::Result<String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    create_sdp(pc, &mut options, false)
}

struct CreateSdpHandler {
    tx: std::sync::mpsc::Sender<crate::Result<String>>,
    is_offer: bool,
}

impl CreateSessionDescriptionObserverHandler for CreateSdpHandler {
    fn on_success(&mut self, desc: SessionDescription) {
        let sdp = desc
            .to_string()
            .map_err(|e| crate::Error::new(format!("failed to convert SDP to string: {e}")));
        let _ = self.tx.send(sdp);
    }

    fn on_failure(&mut self, error: RtcError) {
        let message = error.message().unwrap_or_else(|_| "unknown".to_owned());
        let _ = self.tx.send(Err(crate::Error::new(format!(
            "{} failed: {message}",
            if self.is_offer {
                "create_offer"
            } else {
                "create_answer"
            }
        ))));
    }
}

fn create_sdp(
    pc: &PeerConnection,
    options: &mut PeerConnectionOfferAnswerOptions,
    is_offer: bool,
) -> crate::Result<String> {
    let (tx, rx) = std::sync::mpsc::channel::<crate::Result<String>>();
    let mut observer =
        CreateSessionDescriptionObserver::new_with_handler(Box::new(CreateSdpHandler {
            tx,
            is_offer,
        }));

    if is_offer {
        pc.create_offer(&mut observer, options);
    } else {
        pc.create_answer(&mut observer, options);
    }

    rx.recv_timeout(SDP_TIMEOUT).map_err(|_| {
        crate::Error::new(format!(
            "{} timed out",
            if is_offer {
                "create_offer"
            } else {
                "create_answer"
            }
        ))
    })?
}

pub(crate) fn set_local_offer(pc: &PeerConnection, offer_sdp: &str) -> crate::Result<()> {
    set_local_description(pc, SdpType::Offer, offer_sdp, "offer")
}

pub(crate) fn set_local_answer(pc: &PeerConnection, answer_sdp: &str) -> crate::Result<()> {
    set_local_description(pc, SdpType::Answer, answer_sdp, "answer")
}

pub(crate) fn set_remote_offer(pc: &PeerConnection, offer_sdp: &str) -> crate::Result<()> {
    set_remote_description(pc, SdpType::Offer, offer_sdp, "offer")
}

pub(crate) fn set_remote_answer(pc: &PeerConnection, answer_sdp: &str) -> crate::Result<()> {
    set_remote_description(pc, SdpType::Answer, answer_sdp, "answer")
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

fn set_local_description(
    pc: &PeerConnection,
    sdp_type: SdpType,
    sdp: &str,
    kind: &str,
) -> crate::Result<()> {
    let description = SessionDescription::new(sdp_type, sdp)
        .map_err(|e| crate::Error::new(format!("failed to parse local {kind} SDP: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let observer =
        SetLocalDescriptionObserver::new_with_handler(Box::new(SetLocalSdpHandler { tx }));
    pc.set_local_description(description, &observer);

    let result = rx
        .recv_timeout(SDP_TIMEOUT)
        .map_err(|_| crate::Error::new("set_local_description timed out"))?;
    if let Some(message) = result {
        return Err(crate::Error::new(format!(
            "set_local_description failed: {message}"
        )));
    }
    Ok(())
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

fn set_remote_description(
    pc: &PeerConnection,
    sdp_type: SdpType,
    sdp: &str,
    kind: &str,
) -> crate::Result<()> {
    let description = SessionDescription::new(sdp_type, sdp)
        .map_err(|e| crate::Error::new(format!("failed to parse remote {kind} SDP: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let observer =
        SetRemoteDescriptionObserver::new_with_handler(Box::new(SetRemoteSdpHandler { tx }));
    pc.set_remote_description(description, &observer);

    let result = rx
        .recv_timeout(SDP_TIMEOUT)
        .map_err(|_| crate::Error::new("set_remote_description timed out"))?;
    if let Some(message) = result {
        return Err(crate::Error::new(format!(
            "set_remote_description failed: {message}"
        )));
    }
    Ok(())
}
