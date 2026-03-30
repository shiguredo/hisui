use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use shiguredo_mp4::boxes::SampleEntry;
use shiguredo_webrtc::{
    AudioDecoderFactory, AudioDeviceModule, AudioEncoderFactory, AudioProcessingBuilder,
    DataChannelInit, DataChannelState, PeerConnection, PeerConnectionDependencies,
    PeerConnectionFactory, PeerConnectionFactoryDependencies, PeerConnectionObserver,
    PeerConnectionState, RtcEventLogFactory, SdpType, Thread, VideoDecoderFactory,
    VideoEncoderFactory,
};
use tokio::sync::mpsc;

use crate::adm::{BootstrapAudioDeviceModuleHandler, BootstrapAudioDeviceModuleState};
use crate::encode::{encode_and_write_audio_frame, encode_and_write_frame};
use crate::event::{AudioFrameData, ClientEvent, IceObserverEvent, VideoFrameData};
use crate::http::http_bootstrap;
use crate::mp4::SimpleMp4Writer;
use crate::observer::ClientPcObserver;
use crate::obsws_message::{
    make_create_mp4_input_request, make_subscribe_program_tracks_request,
    parse_obsws_request_response, parse_subscribe_program_tracks_response,
};
use crate::sdp::{
    create_answer_sdp, create_offer_sdp, finalize_local_sdp, log_sdp_summary,
    log_transceiver_receiver_state, set_local_description, set_remote_description,
};
use crate::state::{
    RetainedState, VideoSinkAttachState, attach_audio_sink, attach_video_sink,
    should_write_audio_frame, should_write_video_frame, teardown_client,
};
use crate::stats::{
    Stats, collect_webrtc_stats_json, request_server_webrtc_stats, summarize_webrtc_stats_json,
};

const MAX_FRAMES_PER_POLL: usize = 8;
const INITIAL_VIDEO_FRAME_GRACE: Duration = Duration::from_secs(2);

pub async fn run_client(
    host: &str,
    port: u16,
    duration_secs: u64,
    output_path: &str,
    input_mp4_path: &str,
    subscribe_program_tracks: bool,
) -> Result<Stats, String> {
    // WebRTC ファクトリを初期化する
    let mut network = Thread::new_with_socket_server();
    let mut worker = Thread::new();
    let mut signaling = Thread::new();
    network.start();
    worker.start();
    signaling.start();

    let mut deps = PeerConnectionFactoryDependencies::new();
    deps.set_network_thread(&network);
    deps.set_worker_thread(&worker);
    deps.set_signaling_thread(&signaling);
    deps.set_event_log_factory(RtcEventLogFactory::new());

    let audio_state = Arc::new(BootstrapAudioDeviceModuleState::new());
    let adm = AudioDeviceModule::new_with_handler(Box::new(
        BootstrapAudioDeviceModuleHandler::new(audio_state.clone()),
    ));
    deps.set_audio_device_module(&adm);
    deps.set_audio_encoder_factory(&AudioEncoderFactory::builtin());
    deps.set_audio_decoder_factory(&AudioDecoderFactory::builtin());
    deps.set_video_encoder_factory(VideoEncoderFactory::builtin());
    deps.set_video_decoder_factory(VideoDecoderFactory::builtin());
    deps.set_audio_processing_builder(AudioProcessingBuilder::new_builtin());
    deps.enable_media();

    let factory = Arc::new(
        PeerConnectionFactory::create_modular(&mut deps)
            .map_err(|e| format!("failed to create PeerConnectionFactory: {e}"))?,
    );

    // PeerConnection を作成する
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ClientEvent>();
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<IceObserverEvent>();
    let pc_observer = PeerConnectionObserver::new_with_handler(Box::new(ClientPcObserver {
        event_tx: event_tx.clone(),
        ice_tx,
    }));
    let mut pc_deps = PeerConnectionDependencies::new(&pc_observer);
    let mut config = shiguredo_webrtc::PeerConnectionRtcConfiguration::new();

    let pc = PeerConnection::create(factory.as_ref(), &mut config, &mut pc_deps)
        .map_err(|e| format!("failed to create PeerConnection: {e}"))?;
    // server 側の signaling / obsws DataChannel を初回 offer に載せるための
    // m=application 用ダミー DataChannel
    let mut dc_init = DataChannelInit::new();
    dc_init.set_ordered(true);
    let dummy_dc = pc
        .create_data_channel("dummy", &mut dc_init)
        .map_err(|e| format!("failed to create dummy DataChannel: {e}"))?;
    // offer SDP を生成する
    let offer_sdp = create_offer_sdp(&pc)?;
    log_sdp_summary("initial local offer SDP summary", &offer_sdp);
    set_local_description(&pc, SdpType::Offer, &offer_sdp)?;
    let mut initial_ice_candidates = Vec::new();
    let offer_sdp = finalize_local_sdp(offer_sdp, &mut ice_rx, &mut initial_ice_candidates).await?;
    log_sdp_summary("initial local offer with ICE SDP summary", &offer_sdp);

    // /bootstrap で answer SDP を取得する
    let answer_sdp = http_bootstrap(host, port, &offer_sdp).await?;
    log_sdp_summary("bootstrap remote answer SDP summary", &answer_sdp);
    set_remote_description(&pc, SdpType::Answer, &answer_sdp)?;

    // 統計カウンタ
    let video_tracks = Arc::new(AtomicUsize::new(0));
    let audio_tracks = Arc::new(AtomicUsize::new(0));
    let video_frames = Arc::new(AtomicUsize::new(0));
    let audio_frames = Arc::new(AtomicUsize::new(0));
    let first_video_frame_logged = Arc::new(AtomicBool::new(false));
    let connection_state = Arc::new(std::sync::Mutex::new("new".to_owned()));
    let mut output_video_width = 0;
    let mut output_video_height = 0;
    let mut program_video_track_id: Option<String> = None;
    let mut program_audio_track_id: Option<String> = None;

    // フレームデータ受信用チャネル
    let (frame_tx, frame_rx) = std::sync::mpsc::sync_channel::<VideoFrameData>(60);
    let (audio_tx, audio_rx) = std::sync::mpsc::sync_channel::<AudioFrameData>(120);

    let mut retained = RetainedState {
        _pc_observer: pc_observer,
        dummy_dc,
        obsws_dc: None,
        signaling_dc: None,
        signaling_dc_observer: None,
        obsws_dc_observer: None,
        video_sinks: Vec::new(),
        audio_sinks: Vec::new(),
        track_transceivers: Vec::new(),
        ice_rx,
        ice_candidates: initial_ice_candidates,
    };
    let video_sink_attach_state = VideoSinkAttachState {
        video_frames: &video_frames,
        first_video_frame_logged: &first_video_frame_logged,
        frame_tx: &frame_tx,
    };

    // VP9 エンコーダー（遅延初期化）
    let mut vp9_encoder: Option<shiguredo_libvpx::Encoder> = None;
    let mut vp9_sample_entry: Option<SampleEntry> = None;

    // Opus エンコーダー（遅延初期化）
    let mut opus_encoder: Option<shiguredo_opus::Encoder> = None;
    let mut opus_sample_entry: Option<SampleEntry> = None;
    let mut audio_pcm_buffer: Vec<i16> = Vec::new();
    let mut audio_channels: u8 = 0;

    // MP4 ライター
    let mut mp4_writer = SimpleMp4Writer::new(output_path)?;

    // イベントループ（duration 秒間）
    let deadline = tokio::time::Instant::now() + Duration::from_secs(duration_secs);
    let mut obsws_create_input_sent = false;
    let mut obsws_create_input_succeeded = false;
    let mut obsws_ready = false;
    let mut obsws_subscribe_program_sent = false;
    let mut obsws_subscribe_program_succeeded = false;
    let mut server_webrtc_stats_json = None;
    let mut playout_interval = tokio::time::interval(Duration::from_millis(10));
    playout_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    'event_loop: loop {
        audio_state.render_10ms_audio();

        // フレーム受信チャネルから溜まっているフレームを処理する。
        // 1 回のポーリングで処理するフレーム数を制限して、
        // deadline 判定とイベント処理に必ず戻れるようにする。
        let mut processed_frames = 0;
        while processed_frames < MAX_FRAMES_PER_POLL {
            if tokio::time::Instant::now() >= deadline {
                break 'event_loop;
            }
            let Ok(frame_data) = frame_rx.try_recv() else {
                break;
            };
            if !should_write_video_frame(
                subscribe_program_tracks,
                &frame_data.track_id,
                program_video_track_id.as_deref(),
            ) {
                continue;
            }
            encode_and_write_frame(
                &frame_data,
                &mut vp9_encoder,
                &mut vp9_sample_entry,
                &mut mp4_writer,
                &mut output_video_width,
                &mut output_video_height,
            )?;
            processed_frames += 1;
        }

        // 音声フレームを処理する
        let mut processed_audio = 0;
        while processed_audio < MAX_FRAMES_PER_POLL {
            if tokio::time::Instant::now() >= deadline {
                break 'event_loop;
            }
            let Ok(audio_data) = audio_rx.try_recv() else {
                break;
            };
            if !should_write_audio_frame(
                subscribe_program_tracks,
                &audio_data.track_id,
                program_audio_track_id.as_deref(),
            ) {
                continue;
            }
            audio_channels = audio_data.channels as u8;
            encode_and_write_audio_frame(
                &audio_data,
                &mut opus_encoder,
                &mut opus_sample_entry,
                &mut audio_pcm_buffer,
                &mut mp4_writer,
            )?;
            processed_audio += 1;
        }

        let connection_ready = connection_state.lock().unwrap().as_str() == "connected";
        let signaling_ready = retained
            .signaling_dc
            .as_ref()
            .is_some_and(|dc| dc.state() == DataChannelState::Open);
        if !obsws_create_input_sent
            && let Some(dc) = &retained.obsws_dc
            && connection_ready
            && signaling_ready
            && obsws_ready
            && dc.state() == DataChannelState::Open
        {
            let request = make_create_mp4_input_request(input_mp4_path);
            tracing::info!(
                "sending CreateInput request: input_mp4_path={input_mp4_path}, duration_secs={duration_secs}"
            );
            if !dc.send(request.as_bytes(), false) {
                return Err("failed to send CreateInput request on obsws DataChannel".to_owned());
            }
            obsws_create_input_sent = true;
        }

        let event = tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Some(e) => e,
                    None => break 'event_loop,
                }
            }
            _ = playout_interval.tick() => continue,
            _ = tokio::time::sleep_until(deadline) => break 'event_loop,
        };

        match event {
            ClientEvent::ConnectionChange(state) => {
                tracing::info!("peer connection state changed: {state:?}");
                let state_str = match state {
                    PeerConnectionState::New => "new",
                    PeerConnectionState::Connecting => "connecting",
                    PeerConnectionState::Connected => "connected",
                    PeerConnectionState::Disconnected => "disconnected",
                    PeerConnectionState::Failed => "failed",
                    PeerConnectionState::Closed => "closed",
                    PeerConnectionState::Unknown(_) => "unknown",
                };
                *connection_state.lock().unwrap() = state_str.to_owned();
            }
            ClientEvent::Track(transceiver) => {
                log_transceiver_receiver_state("onTrack transceiver", &transceiver);
                let receiver = transceiver.receiver();
                let track = receiver.track();
                let kind = track.kind().unwrap_or_default();
                let track_id = track.id().unwrap_or_default();
                match kind.as_str() {
                    "video" => {
                        video_tracks.fetch_add(1, Ordering::Relaxed);
                        tracing::info!("video track received: track_id={track_id}");
                        attach_video_sink(
                            &mut retained,
                            &track_id,
                            track.cast_to_video_track(),
                            &video_sink_attach_state,
                        );
                    }
                    "audio" => {
                        audio_tracks.fetch_add(1, Ordering::Relaxed);
                        tracing::info!("audio track received: track_id={track_id}");
                        attach_audio_sink(
                            &mut retained,
                            &track_id,
                            track.cast_to_audio_track(),
                            &audio_frames,
                            &audio_tx,
                        );
                    }
                    _ => {
                        tracing::warn!("unknown track kind: {kind}");
                    }
                }
                // transceiver を保持しないと、ラッパーの寿命次第で受信が不安定になる可能性がある。
                retained.track_transceivers.push(transceiver);
            }
            ClientEvent::DataChannel(dc, observer) => {
                let label = dc.label().unwrap_or_default();
                tracing::info!(
                    "data channel received: label={label}, state={:?}",
                    dc.state()
                );
                if label == "signaling" {
                    retained.signaling_dc = Some(dc);
                    retained.signaling_dc_observer = observer;
                } else if label == "obsws" {
                    obsws_ready = dc.state() == DataChannelState::Open;
                    retained.obsws_dc = Some(dc);
                    retained.obsws_dc_observer = observer;
                }
            }
            ClientEvent::SignalingMessage { data } => {
                let msg_type =
                    crate::obsws_message::parse_signaling_type(&data).unwrap_or_default();
                if msg_type == "offer" {
                    tracing::info!("renegotiation offer received from signaling data channel");
                    // renegotiation: サーバーからの offer に answer を返す
                    if let Some(sdp) = crate::obsws_message::parse_signaling_sdp(&data) {
                        log_sdp_summary("renegotiation remote offer SDP summary", &sdp);
                        if let Err(e) = set_remote_description(&pc, SdpType::Offer, &sdp) {
                            tracing::warn!("failed to set remote offer: {e}");
                            continue;
                        }
                        match create_answer_sdp(&pc) {
                            Ok(answer) => {
                                if let Err(e) = set_local_description(&pc, SdpType::Answer, &answer)
                                {
                                    tracing::warn!("failed to set local answer: {e}");
                                    continue;
                                }
                                let answer = match finalize_local_sdp(
                                    answer,
                                    &mut retained.ice_rx,
                                    &mut retained.ice_candidates,
                                )
                                .await
                                {
                                    Ok(answer) => answer,
                                    Err(e) => {
                                        tracing::warn!("failed to gather ICE candidates: {e}");
                                        continue;
                                    }
                                };
                                log_sdp_summary("renegotiation local answer SDP summary", &answer);
                                let answer_json = crate::obsws_message::make_answer_json(&answer);
                                if let Some(dc) = &retained.signaling_dc {
                                    tracing::info!(
                                        "sending renegotiation answer on signaling data channel"
                                    );
                                    dc.send(answer_json.as_bytes(), false);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("failed to create answer: {e}");
                            }
                        }
                    }
                }
            }
            ClientEvent::ObswsMessage { data } => {
                if let Ok(text) = std::str::from_utf8(&data) {
                    if let Some(result) = parse_obsws_request_response(text) {
                        match result {
                            Ok(()) => {
                                obsws_create_input_succeeded = true;
                                // CreateInput 成功後に SubscribeProgramTracks を送信する
                                if subscribe_program_tracks
                                    && !obsws_subscribe_program_sent
                                    && let Some(dc) = &retained.obsws_dc
                                    && dc.state() == DataChannelState::Open
                                {
                                    let request = make_subscribe_program_tracks_request();
                                    tracing::info!("sending SubscribeProgramTracks request");
                                    if !dc.send(request.as_bytes(), false) {
                                        return Err(
                                            "failed to send SubscribeProgramTracks request"
                                                .to_owned(),
                                        );
                                    }
                                    obsws_subscribe_program_sent = true;
                                }
                            }
                            Err(reason) => {
                                return Err(format!("CreateInput request failed: {reason}"));
                            }
                        }
                    }
                    if let Some(result) = parse_subscribe_program_tracks_response(text) {
                        match result {
                            Ok(track_ids) => {
                                obsws_subscribe_program_succeeded = true;
                                program_video_track_id = Some(track_ids.video_track_id);
                                program_audio_track_id = Some(track_ids.audio_track_id);
                                tracing::info!("SubscribeProgramTracks succeeded");
                            }
                            Err(reason) => {
                                return Err(format!(
                                    "SubscribeProgramTracks request failed: {reason}"
                                ));
                            }
                        }
                    }
                } else {
                    tracing::debug!("obsws message: <binary {} bytes>", data.len());
                }
            }
            ClientEvent::ObswsDataChannelStateChange => {
                if let Some(dc) = &retained.obsws_dc {
                    obsws_ready = dc.state() == DataChannelState::Open;
                }
            }
        }
    }

    if video_tracks.load(Ordering::Relaxed) > 0 && video_frames.load(Ordering::Relaxed) == 0 {
        tracing::warn!(
            "video track was received but no video frames arrived before deadline; waiting additional {:?}",
            INITIAL_VIDEO_FRAME_GRACE
        );
        let grace_deadline = tokio::time::Instant::now() + INITIAL_VIDEO_FRAME_GRACE;
        while tokio::time::Instant::now() < grace_deadline {
            while let Ok(frame_data) = frame_rx.try_recv() {
                if !should_write_video_frame(
                    subscribe_program_tracks,
                    &frame_data.track_id,
                    program_video_track_id.as_deref(),
                ) {
                    continue;
                }
                encode_and_write_frame(
                    &frame_data,
                    &mut vp9_encoder,
                    &mut vp9_sample_entry,
                    &mut mp4_writer,
                    &mut output_video_width,
                    &mut output_video_height,
                )?;
            }
            if video_frames.load(Ordering::Relaxed) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    if video_tracks.load(Ordering::Relaxed) > 0 && video_frames.load(Ordering::Relaxed) == 0 {
        match request_server_webrtc_stats(&retained, &mut event_rx).await {
            Ok(stats_json) => {
                tracing::warn!(
                    "server-side libwebrtc stats summary: {}",
                    summarize_webrtc_stats_json(&stats_json)
                );
                tracing::warn!("server-side libwebrtc stats raw: {stats_json}");
                server_webrtc_stats_json = Some(stats_json);
            }
            Err(e) => {
                tracing::warn!("failed to fetch server-side libwebrtc stats: {e}");
            }
        }
    }

    let webrtc_stats_json = collect_webrtc_stats_json(&pc).await;
    let webrtc_stats_error = match &webrtc_stats_json {
        Ok(_) => String::new(),
        Err(e) => e.clone(),
    };
    if video_frames.load(Ordering::Relaxed) == 0
        && let Ok(stats_json) = &webrtc_stats_json
    {
        tracing::warn!(
            "libwebrtc stats summary: {}",
            summarize_webrtc_stats_json(stats_json)
        );
        tracing::warn!("libwebrtc stats raw: {stats_json}");
    }
    if video_frames.load(Ordering::Relaxed) == 0
        && let Some(stats_json) = &server_webrtc_stats_json
    {
        tracing::debug!("server-side libwebrtc stats length={}", stats_json.len());
    }
    if !webrtc_stats_error.is_empty() {
        tracing::warn!("failed to collect libwebrtc stats: {}", webrtc_stats_error);
    }

    teardown_client(&pc, &mut retained, &audio_state).await;

    // 残りのフレームを処理する
    let drain_deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while let Ok(frame_data) = frame_rx.try_recv() {
        if !should_write_video_frame(
            subscribe_program_tracks,
            &frame_data.track_id,
            program_video_track_id.as_deref(),
        ) {
            continue;
        }
        encode_and_write_frame(
            &frame_data,
            &mut vp9_encoder,
            &mut vp9_sample_entry,
            &mut mp4_writer,
            &mut output_video_width,
            &mut output_video_height,
        )?;
        if tokio::time::Instant::now() >= drain_deadline {
            break;
        }
    }
    // 残りの音声フレームを処理する
    while let Ok(audio_data) = audio_rx.try_recv() {
        if !should_write_audio_frame(
            subscribe_program_tracks,
            &audio_data.track_id,
            program_audio_track_id.as_deref(),
        ) {
            continue;
        }
        audio_channels = audio_data.channels as u8;
        encode_and_write_audio_frame(
            &audio_data,
            &mut opus_encoder,
            &mut opus_sample_entry,
            &mut audio_pcm_buffer,
            &mut mp4_writer,
        )?;
        if tokio::time::Instant::now() >= drain_deadline {
            break;
        }
    }

    // エンコーダーの残りフレームをフラッシュする
    if let Some(encoder) = &mut vp9_encoder {
        encoder
            .finish()
            .map_err(|e| format!("failed to finish encoder: {e}"))?;
        while let Some(frame) = encoder.next_frame() {
            let se = vp9_sample_entry.take();
            mp4_writer.append_video(frame.data(), frame.is_keyframe(), se, 0)?;
        }
    }

    // バッファに残った PCM データが 1 フレーム分以上あればエンコードする
    if let Some(encoder) = &mut opus_encoder {
        let frame_samples = encoder.frame_samples();
        let total_per_frame = frame_samples * audio_channels as usize;
        if total_per_frame > 0 {
            while audio_pcm_buffer.len() >= total_per_frame {
                let pcm: Vec<i16> = audio_pcm_buffer.drain(..total_per_frame).collect();
                let opus_data = encoder
                    .encode(&pcm)
                    .map_err(|e| format!("Opus encode failed: {e}"))?;
                let sample_rate = encoder
                    .get_sample_rate()
                    .map_err(|e| format!("failed to get sample rate: {e}"))?;
                let duration_us = (frame_samples as u64 * 1_000_000 / sample_rate as u64) as u32;
                let se = opus_sample_entry.take();
                mp4_writer.append_audio(&opus_data, se, duration_us)?;
            }
        }
    }

    // MP4 ファイルをファイナライズする
    if mp4_writer.video_sample_count > 0 || mp4_writer.audio_sample_count > 0 {
        mp4_writer.finalize()?;
    }

    let video_codec = if mp4_writer.video_sample_count > 0 {
        "vp9".to_owned()
    } else {
        "none".to_owned()
    };
    let audio_codec = if mp4_writer.audio_sample_count > 0 {
        "opus".to_owned()
    } else {
        "none".to_owned()
    };

    if !obsws_create_input_succeeded {
        tracing::warn!("CreateInput request did not complete before deadline");
        return Err("CreateInput request did not complete".to_owned());
    }
    if subscribe_program_tracks && !obsws_subscribe_program_succeeded {
        tracing::warn!("SubscribeProgramTracks request did not complete before deadline");
        return Err("SubscribeProgramTracks request did not complete".to_owned());
    }
    let final_connection_state = connection_state
        .lock()
        .expect("connection_state mutex should not be poisoned")
        .clone();
    tracing::info!(
        "bootstrap finished: video_tracks={}, video_frames={}, audio_tracks={}, audio_frames={}, video_width={}, video_height={}, video_samples_written={}, audio_samples_written={}, connection_state={}, webrtc_stats_error={}, program_tracks_subscribed={}",
        video_tracks.load(Ordering::Relaxed),
        video_frames.load(Ordering::Relaxed),
        audio_tracks.load(Ordering::Relaxed),
        audio_frames.load(Ordering::Relaxed),
        output_video_width,
        output_video_height,
        mp4_writer.video_sample_count,
        mp4_writer.audio_sample_count,
        final_connection_state,
        webrtc_stats_error.as_str(),
        obsws_subscribe_program_succeeded,
    );
    Ok(Stats {
        video_tracks: video_tracks.load(Ordering::Relaxed),
        audio_tracks: audio_tracks.load(Ordering::Relaxed),
        video_frames: video_frames.load(Ordering::Relaxed),
        audio_frames: audio_frames.load(Ordering::Relaxed),
        video_width: output_video_width,
        video_height: output_video_height,
        video_codec,
        audio_codec,
        video_samples_written: mp4_writer.video_sample_count,
        audio_samples_written: mp4_writer.audio_sample_count,
        connection_state: connection_state.lock().unwrap().clone(),
        webrtc_stats_error,
        program_tracks_subscribed: obsws_subscribe_program_succeeded,
    })
}
