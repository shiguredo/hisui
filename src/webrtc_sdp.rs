use std::time::Duration;

use shiguredo_webrtc::{
    CreateSessionDescriptionObserver, PeerConnection, PeerConnectionOfferAnswerOptions, SdpType,
    SessionDescription, SetLocalDescriptionObserver, SetRemoteDescriptionObserver,
};

const SDP_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn create_offer_sdp(pc: &PeerConnection) -> crate::Result<String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    options.set_offer_to_receive_audio(0);
    options.set_offer_to_receive_video(0);
    create_sdp(pc, &mut options, true)
}

pub(crate) fn create_offer_sdp_recvonly(pc: &PeerConnection) -> crate::Result<String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    options.set_offer_to_receive_audio(1);
    options.set_offer_to_receive_video(1);
    create_sdp(pc, &mut options, true)
}

pub(crate) fn create_answer_sdp(pc: &PeerConnection) -> crate::Result<String> {
    let mut options = PeerConnectionOfferAnswerOptions::new();
    create_sdp(pc, &mut options, false)
}

fn create_sdp(
    pc: &PeerConnection,
    options: &mut PeerConnectionOfferAnswerOptions,
    is_offer: bool,
) -> crate::Result<String> {
    let (tx, rx) = std::sync::mpsc::channel::<crate::Result<String>>();
    let tx_ok = tx.clone();
    let mut observer = CreateSessionDescriptionObserver::new(
        move |description| {
            let sdp = description
                .to_string()
                .map_err(|e| crate::Error::new(format!("failed to convert SDP to string: {e}")));
            let _ = tx_ok.send(sdp);
        },
        move |error| {
            let message = error.message().unwrap_or_else(|_| "unknown".to_owned());
            let _ = tx.send(Err(crate::Error::new(format!(
                "{} failed: {message}",
                if is_offer {
                    "create_offer"
                } else {
                    "create_answer"
                }
            ))));
        },
    );

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

fn set_local_description(
    pc: &PeerConnection,
    sdp_type: SdpType,
    sdp: &str,
    kind: &str,
) -> crate::Result<()> {
    let description = SessionDescription::new(sdp_type, sdp)
        .map_err(|e| crate::Error::new(format!("failed to parse local {kind} SDP: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let observer = SetLocalDescriptionObserver::new(move |error| {
        let message = if error.ok() {
            None
        } else {
            Some(error.message().unwrap_or_else(|_| "unknown".to_owned()))
        };
        let _ = tx.send(message);
    });
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

fn set_remote_description(
    pc: &PeerConnection,
    sdp_type: SdpType,
    sdp: &str,
    kind: &str,
) -> crate::Result<()> {
    let description = SessionDescription::new(sdp_type, sdp)
        .map_err(|e| crate::Error::new(format!("failed to parse remote {kind} SDP: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let observer = SetRemoteDescriptionObserver::new(move |error| {
        let message = if error.ok() {
            None
        } else {
            Some(error.message().unwrap_or_else(|_| "unknown".to_owned()))
        };
        let _ = tx.send(message);
    });
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
