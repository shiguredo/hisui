//! Output (HLS / DASH) 系のテスト。
//!
//! 旧 `src/obsws/session/tests.rs` の line 2109 / 2239 から物理移動した 2 件を集約する。
//! テスト名に `scene_item` / `scene` を含むが、検証対象は HLS / DASH 出力経路 (program mixer の挙動)。

use crate::obsws::message::RequestMessage;
use crate::obsws::session::ObswsSession;
use crate::obsws::state::{ObswsInput, ObswsSessionState};
use std::time::Duration;

use super::common::*;

#[tokio::test]
async fn hls_output_uses_program_mixers_after_scene_item_change() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsSessionState::new(
        crate::types::EvenUsize::new(1920).expect("canvas width must be valid"),
        crate::types::EvenUsize::new(1080).expect("canvas height must be valid"),
        crate::video::FrameRate::FPS_30,
        None,
        None,
    );
    let input = ObswsInput::from_kind_and_settings(
        "mp4_file_source",
        nojson::RawJsonOwned::parse(
            r#"{"path":"testdata/red-320x320-h264-aac.mp4","loop_playback":true}"#,
        )
        .expect("requestData must be valid json")
        .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "video-file", input, true)
        .expect("input creation must succeed");

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle = create_coordinator_handle_with_pipeline(registry, pipeline_handle.clone());
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;
    create_output(&mut session, "hls", "hls_output").await;

    let hls_output_dir = temp_dir.path().join("hls-output");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-hls-output".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"outputName":"hls","outputSettings":{{"destination":{{"type":"filesystem","directory":"{}"}},"variants":[{{"video_bitrate":2000000,"audio_bitrate":128000}},{{"video_bitrate":1000000,"audio_bitrate":64000,"width":1280,"height":720}}]}}}}"#,
                    hls_output_dir.display()
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-hls-output".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"hls"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:hls:v1_scaler:0", true).await?;
    wait_for_processor_presence(&pipeline_handle, "output:hls:v0_hls_writer:0", true).await?;
    wait_for_processor_presence(&pipeline_handle, "output:hls:video_mixer:0", false).await?;

    let get_scene_item_id_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-get-hls-scene-item-id".to_owned()),
            request_type: Some("GetSceneItemId".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene","sourceName":"video-file"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(get_scene_item_id_action);
    let scene_item_id = parse_response_scene_item_id(&text);

    let set_scene_item_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-disable-hls-scene-item".to_owned()),
            request_type: Some("SetSceneItemEnabled".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"sceneName":"Scene","sceneItemId":{},"sceneItemEnabled":false}}"#,
                    scene_item_id
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_scene_item_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:hls:v0_hls_writer:0", true).await?;

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-hls-output".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"hls"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:hls:v0_hls_writer:0", false).await?;

    pipeline_task.abort();

    Ok(())
}

#[tokio::test]
async fn dash_output_uses_program_mixers_after_scene_change() -> crate::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut registry = ObswsSessionState::new(
        crate::types::EvenUsize::new(1920).expect("canvas width must be valid"),
        crate::types::EvenUsize::new(1080).expect("canvas height must be valid"),
        crate::video::FrameRate::FPS_30,
        None,
        None,
    );
    registry
        .create_scene("Scene B")
        .expect("second scene must be created");
    let input = ObswsInput::from_kind_and_settings(
        "mp4_file_source",
        nojson::RawJsonOwned::parse(
            r#"{"path":"testdata/red-320x320-h264-aac.mp4","loop_playback":true}"#,
        )
        .expect("requestData must be valid json")
        .value(),
    )
    .expect("input settings must be valid");
    registry
        .create_input("Scene", "video-file", input, true)
        .expect("input creation must succeed");

    let pipeline = crate::MediaPipeline::new(Default::default(), Default::default())?;
    let pipeline_handle = pipeline.handle();
    let pipeline_task = tokio::spawn(pipeline.run());
    let started = pipeline_handle
        .trigger_start()
        .await
        .map_err(|_| crate::Error::new("failed to trigger start: pipeline has terminated"))?;
    assert!(started);

    let handle =
        create_initialized_coordinator_handle_with_pipeline(registry, pipeline_handle.clone())
            .await?;
    let mut session = ObswsSession::new(None, handle);
    identify_session(&mut session).await;
    create_output(&mut session, "mpeg_dash", "mpeg_dash_output").await;

    let dash_output_dir = temp_dir.path().join("dash-output");
    let set_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-dash-output".to_owned()),
            request_type: Some("SetOutputSettings".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(format!(
                    r#"{{"outputName":"mpeg_dash","outputSettings":{{"destination":{{"type":"filesystem","directory":"{}"}},"video_codec":"VP9","audio_codec":"OPUS","variants":[{{"video_bitrate":2000000,"audio_bitrate":128000}},{{"video_bitrate":1000000,"audio_bitrate":64000,"width":1280,"height":720}}]}}}}"#,
                    dash_output_dir.display()
                ))
                .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    let start_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-start-dash-output".to_owned()),
            request_type: Some("StartOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"mpeg_dash"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(start_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:v1_scaler:0", true).await?;
    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:v0_dash_writer:0", true)
        .await?;
    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:video_mixer:0", false).await?;

    // ABR 結合 MPD は SampleEntry からコーデック文字列が確定してから書き出される。
    // manifest.mpd の出現を待ち、codecs 属性が実際の SampleEntry と一致することを検証する。
    // VP9 + Opus は libvpx / opus が全環境で利用可能なため、エンコーダー不在で失敗しない。
    let manifest_path = dash_output_dir.join("manifest.mpd");
    for _ in 0..60 {
        if manifest_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        manifest_path.exists(),
        "ABR combined manifest.mpd must be written after codec resolution"
    );
    let mpd_xml = std::fs::read_to_string(&manifest_path).expect("manifest.mpd must be readable");
    let mpd = shiguredo_mpd::parse(&mpd_xml).expect("manifest.mpd must be valid MPD XML");
    let adaptation_set = &mpd.periods[0].adaptation_sets[0];
    let codecs = adaptation_set
        .codecs
        .as_ref()
        .expect("AdaptationSet.codecs must be present");
    // VP9 + Opus を指定しているので codecs は vp09 と opus を含むこと
    assert!(
        codecs.contains("vp09."),
        "codecs must contain vp09 prefix from actual SampleEntry, got: {codecs}"
    );
    assert!(
        codecs.contains("opus"),
        "codecs must contain opus from actual SampleEntry, got: {codecs}"
    );

    let set_scene_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-set-program-scene-dash".to_owned()),
            request_type: Some("SetCurrentProgramScene".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"sceneName":"Scene B"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(set_scene_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:v0_dash_writer:0", true)
        .await?;

    let stop_action = session
        .handle_request(RequestMessage {
            request_id: Some("req-stop-dash-output".to_owned()),
            request_type: Some("StopOutput".to_owned()),
            request_data: Some(
                nojson::RawJsonOwned::parse(r#"{"outputName":"mpeg_dash"}"#)
                    .expect("requestData must be valid json"),
            ),
        })
        .await;
    let text = unwrap_send_text(stop_action);
    let (result, code) = parse_request_status(&text);
    assert!(result);
    assert_eq!(code, 100);

    wait_for_processor_presence(&pipeline_handle, "output:mpeg_dash:v0_dash_writer:0", false)
        .await?;

    pipeline_task.abort();

    Ok(())
}
