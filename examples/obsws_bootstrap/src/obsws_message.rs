pub const CREATE_INPUT_REQUEST_ID: &str = "req-create-input";
pub const GET_WEBRTC_STATS_REQUEST_ID: &str = "req-get-webrtc-stats";
pub const SUBSCRIBE_PROGRAM_TRACKS_REQUEST_ID: &str = "req-subscribe-program-tracks";
pub const CREATE_WEBRTC_SOURCE_REQUEST_ID: &str = "req-create-webrtc-source";
pub const LIST_WEBRTC_VIDEO_TRACKS_REQUEST_ID: &str = "req-list-webrtc-video-tracks";
pub const ATTACH_WEBRTC_VIDEO_TRACK_REQUEST_ID: &str = "req-attach-webrtc-video-track";

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
                f.member("requestType", "HisuiGetWebRtcStats")?;
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
                f.member("requestType", "HisuiSubscribeProgramTracks")?;
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
        "HisuiSubscribeProgramTracks request failed".to_owned()
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
            comment.unwrap_or_else(|| "HisuiGetWebRtcStats request failed".to_owned())
        ));
    }

    let response_data = d.to_member("responseData").ok()?.required().ok()?;
    let stats = response_data.to_member("stats").ok()?.required().ok()?;
    Some(Ok(stats.as_raw_str().to_owned()))
}

// --- signaling offer ---

pub fn make_offer_json(sdp: &str) -> String {
    nojson::object(|f| {
        f.member("type", "offer")?;
        f.member("sdp", sdp)
    })
    .to_string()
}

// --- webrtc_source 用 Request/Response ---

pub fn make_create_webrtc_source_request(input_name: &str) -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "CreateInput")?;
                f.member("requestId", CREATE_WEBRTC_SOURCE_REQUEST_ID)?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
                        f.member("sceneName", "Scene")?;
                        f.member("inputName", input_name)?;
                        f.member("inputKind", "webrtc_source")?;
                        f.member("inputSettings", nojson::object(|_f| Ok(())))?;
                        f.member("sceneItemEnabled", true)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn parse_create_webrtc_source_response(text: &str) -> Option<Result<(), String>> {
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
    if request_id != CREATE_WEBRTC_SOURCE_REQUEST_ID {
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
    Some(Err(comment.unwrap_or_else(|| {
        "CreateInput(webrtc_source) request failed".to_owned()
    })))
}

pub fn make_list_webrtc_video_tracks_request() -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "HisuiListWebRtcVideoTracks")?;
                f.member("requestId", LIST_WEBRTC_VIDEO_TRACKS_REQUEST_ID)
            }),
        )
    })
    .to_string()
}

pub struct WebRtcVideoTrackInfo {
    pub track_id: String,
    pub attached_input_name: Option<String>,
}

pub fn parse_list_webrtc_video_tracks_response(
    text: &str,
) -> Option<Result<Vec<WebRtcVideoTrackInfo>, String>> {
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
    if request_id != LIST_WEBRTC_VIDEO_TRACKS_REQUEST_ID {
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
        return Some(Err(comment.unwrap_or_else(|| {
            "HisuiListWebRtcVideoTracks request failed".to_owned()
        })));
    }

    let response_data = d.to_member("responseData").ok()?.required().ok()?;
    let tracks_array = response_data.to_member("tracks").ok()?.required().ok()?;
    let tracks_iter = tracks_array.to_array().ok()?;
    let mut tracks = Vec::new();
    for track in tracks_iter {
        let track_id: String = track
            .to_member("trackId")
            .and_then(|v| v.required()?.try_into())
            .ok()?;
        let attached_input_name: Option<String> =
            if let Some(v) = track.to_member("attachedInputName").ok()?.optional() {
                v.try_into().ok()
            } else {
                None
            };
        tracks.push(WebRtcVideoTrackInfo {
            track_id,
            attached_input_name,
        });
    }
    Some(Ok(tracks))
}

pub fn make_attach_webrtc_video_track_request(input_name: &str, track_id: &str) -> String {
    nojson::object(|f| {
        f.member("op", 6)?;
        f.member(
            "d",
            nojson::object(|f| {
                f.member("requestType", "HisuiAttachWebRtcVideoTrack")?;
                f.member("requestId", ATTACH_WEBRTC_VIDEO_TRACK_REQUEST_ID)?;
                f.member(
                    "requestData",
                    nojson::object(|f| {
                        f.member("inputName", input_name)?;
                        f.member("trackId", track_id)
                    }),
                )
            }),
        )
    })
    .to_string()
}

pub fn parse_attach_webrtc_video_track_response(text: &str) -> Option<Result<(), String>> {
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
    if request_id != ATTACH_WEBRTC_VIDEO_TRACK_REQUEST_ID {
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
    Some(Err(comment.unwrap_or_else(|| {
        "HisuiAttachWebRtcVideoTrack request failed".to_owned()
    })))
}
