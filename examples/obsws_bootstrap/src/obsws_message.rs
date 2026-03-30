pub const CREATE_INPUT_REQUEST_ID: &str = "req-create-input";
pub const GET_WEBRTC_STATS_REQUEST_ID: &str = "req-get-webrtc-stats";
pub const SUBSCRIBE_PROGRAM_TRACKS_REQUEST_ID: &str = "req-subscribe-program-tracks";

// --- signaling DC メッセージパーサ ---

pub fn parse_signaling_type(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?;
    let json = nojson::RawJson::parse(text).ok()?;
    let msg_type: String = json
        .value()
        .to_member("type")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    Some(msg_type)
}

pub fn parse_signaling_sdp(data: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(data).ok()?;
    let json = nojson::RawJson::parse(text).ok()?;
    let sdp: String = json
        .value()
        .to_member("sdp")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    Some(sdp)
}

pub fn make_answer_json(sdp: &str) -> String {
    nojson::object(|f| {
        f.member("type", "answer")?;
        f.member("sdp", sdp)
    })
    .to_string()
}

pub fn make_create_mp4_input_request(input_path: &str) -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateInput")?;
                f.member("requestId", CREATE_INPUT_REQUEST_ID)?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
                        f.member("sceneName", "Scene")?;
                        f.member("inputName", "obsws-bootstrap-input")?;
                        f.member("inputKind", "mp4_file_source")?;
                        f.member(
                            "inputSettings",
                            nojson::object(|f| {
                                f.member("path", input_path)?;
                                f.member("loopPlayback", true)
                            }),
                        )?;
                        f.member("sceneItemEnabled", true)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn make_get_webrtc_stats_request() -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "GetWebRtcStats")?;
                f.member("requestId", GET_WEBRTC_STATS_REQUEST_ID)
            }),
        )
    })
    .to_string()
}

pub fn make_subscribe_program_tracks_request() -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "SubscribeProgramTracks")?;
                f.member("requestId", SUBSCRIBE_PROGRAM_TRACKS_REQUEST_ID)
            }),
        )
    })
    .to_string()
}

pub fn parse_obsws_request_response(text: &str) -> Option<Result<(), String>> {
    let json = nojson::RawJson::parse(text).ok()?;
    let root = json.value();
    let op: i64 = root
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if op != 7 {
        return None;
    }

    let d = root.to_member("d").ok()?.required().ok()?;
    let request_id: String = d
        .to_member("requestId")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if request_id != CREATE_INPUT_REQUEST_ID {
        return None;
    }

    let request_status = d.to_member("requestStatus").ok()?.required().ok()?;
    let result: bool = request_status
        .to_member("result")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if result {
        return Some(Ok(()));
    }

    let comment: Option<String> =
        if let Some(v) = request_status.to_member("comment").ok()?.optional() {
            v.try_into().ok()
        } else {
            None
        };
    Some(Err(
        comment.unwrap_or_else(|| "CreateInput request failed".to_owned())
    ))
}

pub struct ProgramTrackIds {
    pub video_track_id: String,
    pub audio_track_id: String,
}

pub fn parse_subscribe_program_tracks_response(
    text: &str,
) -> Option<Result<ProgramTrackIds, String>> {
    let json = nojson::RawJson::parse(text).ok()?;
    let root = json.value();
    let op: i64 = root
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if op != 7 {
        return None;
    }

    let d = root.to_member("d").ok()?.required().ok()?;
    let request_id: String = d
        .to_member("requestId")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if request_id != SUBSCRIBE_PROGRAM_TRACKS_REQUEST_ID {
        return None;
    }

    let request_status = d.to_member("requestStatus").ok()?.required().ok()?;
    let result: bool = request_status
        .to_member("result")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if result {
        let response_data = d.to_member("responseData").ok()?.required().ok()?;
        let video_track_id: String = response_data
            .to_member("videoTrackId")
            .and_then(|v| v.required()?.try_into())
            .ok()?;
        let audio_track_id: String = response_data
            .to_member("audioTrackId")
            .and_then(|v| v.required()?.try_into())
            .ok()?;
        return Some(Ok(ProgramTrackIds {
            video_track_id,
            audio_track_id,
        }));
    }

    let comment: Option<String> =
        if let Some(v) = request_status.to_member("comment").ok()?.optional() {
            v.try_into().ok()
        } else {
            None
        };
    Some(Err(comment.unwrap_or_else(|| {
        "SubscribeProgramTracks request failed".to_owned()
    })))
}

pub fn parse_obsws_server_webrtc_stats_response(text: &str) -> Option<Result<String, String>> {
    let json = nojson::RawJson::parse(text).ok()?;
    let root = json.value();
    let op: i64 = root
        .to_member("op")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if op != 7 {
        return None;
    }

    let d = root.to_member("d").ok()?.required().ok()?;
    let request_id: String = d
        .to_member("requestId")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if request_id != GET_WEBRTC_STATS_REQUEST_ID {
        return None;
    }

    let request_status = d.to_member("requestStatus").ok()?.required().ok()?;
    let result: bool = request_status
        .to_member("result")
        .and_then(|v| v.required()?.try_into())
        .ok()?;
    if !result {
        let comment: Option<String> =
            if let Some(v) = request_status.to_member("comment").ok()?.optional() {
                v.try_into().ok()
            } else {
                None
            };
        return Some(Err(
            comment.unwrap_or_else(|| "GetWebRtcStats request failed".to_owned())
        ));
    }

    let response_data = d.to_member("responseData").ok()?.required().ok()?;
    let stats = response_data.to_member("stats").ok()?.required().ok()?;
    Some(Ok(stats.as_raw_str().to_owned()))
}
