use crate::json::JsonObject;

// エンコードパラメーターのデフォルト値
const DEFAULT_CQ_LEVEL: usize = 30;
const DEFAULT_MIN_Q: usize = 10;
const DEFAULT_MAX_Q: usize = 50;

pub fn parse_vp8_encode_params(
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
            _ => Err(v.invalid("unknown 'deadline' value")),
        })?
    {
        config.deadline = deadline;
    }

    // レート制御モード
    if let Some(rate_control) = params.get_with("rate_control", |v| {
        match v.to_unquoted_string_str()?.as_ref() {
            "vbr" => Ok(shiguredo_libvpx::RateControlMode::Vbr),
            "cbr" => Ok(shiguredo_libvpx::RateControlMode::Cbr),
            "cq" => Ok(shiguredo_libvpx::RateControlMode::Cq),
            _ => Err(v.invalid("unknown 'rate_control' value")),
        }
    })? {
        config.rate_control = rate_control;
    }

    // 先読みフレーム数
    config.lag_in_frames = params.get("lag_in_frames")?;

    // スレッド数
    config.threads = params.get("threads")?;

    // エラー耐性モード
    config.error_resilient = params.get("error_resilient")?.unwrap_or_default();

    // キーフレーム間隔
    config.keyframe_interval = params.get("keyframe_interval")?;

    // フレームドロップ閾値
    config.frame_drop_threshold = params.get("frame_drop_threshold")?;

    // 以降はVP8固有の設定
    let mut vp8_config = shiguredo_libvpx::Vp8Config {
        noise_sensitivity: None,
        static_threshold: None,
        token_partitions: None,
        max_intra_bitrate_pct: None,
        arnr_config: None,
    };
    vp8_config.noise_sensitivity = params.get("noise_sensitivity")?;
    vp8_config.static_threshold = params.get("static_threshold")?;
    vp8_config.token_partitions = params.get("token_partitions")?;
    vp8_config.max_intra_bitrate_pct = params.get("max_intra_bitrate_pct")?;

    // ARNR設定
    vp8_config.arnr_config = params.get_with("arnr_config", |v| {
        let arnr_obj = JsonObject::new(v)?;
        Ok(shiguredo_libvpx::ArnrConfig {
            max_frames: arnr_obj.get("max_frames")?.unwrap_or(0) as i32,
            strength: arnr_obj.get("strength")?.unwrap_or(3) as i32,
            filter_type: arnr_obj.get("filter_type")?.unwrap_or(1) as i32,
        })
    })?;

    config.vp8_config = Some(vp8_config);
    Ok(config)
}

pub fn parse_vp9_encode_params(
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
            _ => Err(v.invalid("unknown 'deadline' value")),
        })?
    {
        config.deadline = deadline;
    }

    // レート制御モード
    if let Some(rate_control) = params.get_with("rate_control", |v| {
        match v.to_unquoted_string_str()?.as_ref() {
            "vbr" => Ok(shiguredo_libvpx::RateControlMode::Vbr),
            "cbr" => Ok(shiguredo_libvpx::RateControlMode::Cbr),
            "cq" => Ok(shiguredo_libvpx::RateControlMode::Cq),
            _ => Err(v.invalid("unknown 'rate_control' value")),
        }
    })? {
        config.rate_control = rate_control;
    }

    // 先読みフレーム数
    config.lag_in_frames = params.get("lag_in_frames")?;

    // スレッド数
    config.threads = params.get("threads")?;

    // エラー耐性モード
    config.error_resilient = params.get("error_resilient")?.unwrap_or_default();

    // キーフレーム間隔
    config.keyframe_interval = params.get("keyframe_interval")?;

    // フレームドロップ閾値
    config.frame_drop_threshold = params.get("frame_drop_threshold")?;

    // 以降はVP9固有の設定
    let mut vp9_config = shiguredo_libvpx::Vp9Config {
        aq_mode: None,
        noise_sensitivity: None,
        tile_columns: None,
        tile_rows: None,
        row_mt: false,
        frame_parallel_decoding: false,
        tune_content: None,
    };

    // VP9固有パラメータの設定
    vp9_config.aq_mode = params.get("aq_mode")?;
    vp9_config.noise_sensitivity = params.get("noise_sensitivity")?;
    vp9_config.tile_columns = params.get("tile_columns")?;
    vp9_config.tile_rows = params.get("tile_rows")?;
    vp9_config.row_mt = params.get("row_mt")?.unwrap_or_default();
    vp9_config.frame_parallel_decoding = params.get("frame_parallel_decoding")?.unwrap_or_default();

    // コンテンツタイプ最適化
    vp9_config.tune_content = params.get_with("tune_content", |v| {
        match v.to_unquoted_string_str()?.as_ref() {
            "default" => Ok(shiguredo_libvpx::ContentType::Default),
            "screen" => Ok(shiguredo_libvpx::ContentType::Screen),
            _ => Err(v.invalid("unknown 'tune_content' value")),
        }
    })?;

    config.vp9_config = Some(vp9_config);
    Ok(config)
}
