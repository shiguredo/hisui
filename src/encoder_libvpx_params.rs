use crate::json::JsonObject;

// エンコードパラメーターのデフォルト値
const DEFAULT_CQ_LEVEL: usize = 30;
const DEFAULT_MIN_Q: usize = 10;
const DEFAULT_MAX_Q: usize = 50;

pub fn parse_libvpx_vp8_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_libvpx::EncoderConfig, nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate
    let params = JsonObject::new(value)?;
    let mut config = shiguredo_libvpx::EncoderConfig::default();

    // 基本的なエンコーダーパラメーター
    config.min_quantizer = params.get("min_quantizer")?.unwrap_or(DEFAULT_MIN_Q);
    config.max_quantizer = params.get("max_quantizer")?.unwrap_or(DEFAULT_MAX_Q);
    config.cq_level = params.get("cq_level")?.unwrap_or(DEFAULT_CQ_LEVEL);
    config.cpu_used = params.get("cpu_used")?;

    // エンコード期限設定
    if let Some(deadline) =
        params.get_with("deadline", |v| match v.to_unquoted_string_str()?.as_ref() {
            "best" => Ok(shiguredo_libvpx::EncodingDeadline::Best),
            "good" => Ok(shiguredo_libvpx::EncodingDeadline::Good),
            "realtime" => Ok(shiguredo_libvpx::EncodingDeadline::Realtime),
            _ => Err(v.invalid("")),
        })?
    {
        config.deadline = deadline;
    }

    // // レート制御モード
    // if let Some(rate_control) = params.get("rate_control") {
    //     config.rate_control = match rate_control.to_str()? {
    //         "vbr" => shiguredo_libvpx::RateControlMode::Vbr,
    //         "cbr" => shiguredo_libvpx::RateControlMode::Cbr,
    //         "cq" => shiguredo_libvpx::RateControlMode::Cq,
    //         _ => return Err(nojson::JsonParseError::InvalidValue),
    //     };
    // }

    // // 先読みフレーム数
    // if let Some(lag_in_frames) = params.get("lag_in_frames") {
    //     if let Ok(lag) = std::num::NonZeroUsize::new(lag_in_frames.to_u64()? as usize) {
    //         config.lag_in_frames = Some(lag);
    //     }
    // }

    // // スレッド数
    // if let Some(threads) = params.get("threads") {
    //     if let Ok(thread_count) = std::num::NonZeroUsize::new(threads.to_u64()? as usize) {
    //         config.threads = Some(thread_count);
    //     }
    // }

    // // エラー耐性モード
    // if let Some(error_resilient) = params.get("error_resilient") {
    //     config.error_resilient = error_resilient.to_bool()?;
    // }

    // // キーフレーム間隔
    // if let Some(keyframe_interval) = params.get("keyframe_interval") {
    //     if let Ok(interval) = std::num::NonZeroUsize::new(keyframe_interval.to_u64()? as usize) {
    //         config.keyframe_interval = Some(interval);
    //     }
    // }

    // // フレームドロップ閾値
    // if let Some(frame_drop_threshold) = params.get("frame_drop_threshold") {
    //     config.frame_drop_threshold = Some(frame_drop_threshold.to_u64()? as usize);
    // }

    // // 以降はVP8固有の設定
    // let mut vp8_config = shiguredo_libvpx::Vp8Config {
    //     noise_sensitivity: None,
    //     static_threshold: None,
    //     token_partitions: None,
    //     max_intra_bitrate_pct: None,
    //     arnr_config: None,
    // };

    // if let Some(noise_sensitivity) = params.get("noise_sensitivity") {
    //     vp8_config.noise_sensitivity = Some(noise_sensitivity.to_i64()? as i32);
    // }

    // if let Some(static_threshold) = params.get("static_threshold") {
    //     vp8_config.static_threshold = Some(static_threshold.to_i64()? as i32);
    // }

    // if let Some(token_partitions) = params.get("token_partitions") {
    //     vp8_config.token_partitions = Some(token_partitions.to_i64()? as i32);
    // }

    // if let Some(max_intra_bitrate_pct) = params.get("max_intra_bitrate_pct") {
    //     vp8_config.max_intra_bitrate_pct = Some(max_intra_bitrate_pct.to_i64()? as i32);
    // }

    // // ARNR設定
    // if let Some(arnr_params) = params.get("arnr_config") {
    //     let arnr_obj = arnr_params.to_object()?;
    //     let arnr_config = shiguredo_libvpx::ArnrConfig {
    //         max_frames: arnr_obj
    //             .get("max_frames")
    //             .map(|v| v.to_i64())
    //             .transpose()?
    //             .unwrap_or(0) as i32,
    //         strength: arnr_obj
    //             .get("strength")
    //             .map(|v| v.to_i64())
    //             .transpose()?
    //             .unwrap_or(3) as i32,
    //         filter_type: arnr_obj
    //             .get("filter_type")
    //             .map(|v| v.to_i64())
    //             .transpose()?
    //             .unwrap_or(1) as i32,
    //     };
    //     vp8_config.arnr_config = Some(arnr_config);
    // }

    // config.vp8_config = Some(vp8_config);

    Ok(config)
}

pub fn parse_libvpx_vp9_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_libvpx::EncoderConfig, nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate
    let params = value.to_object()?;
    todo!()
}
