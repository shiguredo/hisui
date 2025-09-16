use crate::json::JsonObject;
use crate::layout::DEFAULT_LAYOUT_JSON;

pub fn parse_vp8_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_libvpx::EncoderConfig, nojson::JsonParseError> {
    let mut config = shiguredo_libvpx::EncoderConfig::default();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("libvpx_vp8_encode_params")?
            .required()?,
    )?;
    update_vp8_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_vp8_encode_params(params, &mut config)?;

    Ok(config)
}

pub fn parse_vp9_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_libvpx::EncoderConfig, nojson::JsonParseError> {
    let mut config = shiguredo_libvpx::EncoderConfig::default();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("libvpx_vp9_encode_params")?
            .required()?,
    )?;
    update_vp9_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_vp9_encode_params(params, &mut config)?;

    Ok(config)
}

fn update_vp8_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut shiguredo_libvpx::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    // 基本的なエンコーダーパラメーター
    config.min_quantizer = params.get("min_quantizer")?.unwrap_or(config.min_quantizer);
    config.max_quantizer = params.get("max_quantizer")?.unwrap_or(config.max_quantizer);
    config.cq_level = params.get("cq_level")?.unwrap_or(config.cq_level);

    if let Some(cpu_used) = params.get("cpu_used")? {
        config.cpu_used = cpu_used;
    }

    // エンコード期限設定
    config.deadline = params
        .get_with("deadline", |v| match v.to_unquoted_string_str()?.as_ref() {
            "best" => Ok(shiguredo_libvpx::EncodingDeadline::Best),
            "good" => Ok(shiguredo_libvpx::EncodingDeadline::Good),
            "realtime" => Ok(shiguredo_libvpx::EncodingDeadline::Realtime),
            _ => Err(v.invalid("unknown 'deadline' value")),
        })?
        .unwrap_or(config.deadline.clone());

    // レート制御モード
    config.rate_control = params
        .get_with("rate_control", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "vbr" => Ok(shiguredo_libvpx::RateControlMode::Vbr),
                "cbr" => Ok(shiguredo_libvpx::RateControlMode::Cbr),
                "cq" => Ok(shiguredo_libvpx::RateControlMode::Cq),
                _ => Err(v.invalid("unknown 'rate_control' value")),
            }
        })?
        .unwrap_or(config.rate_control.clone());

    // 先読みフレーム数
    if let Some(lag_in_frames) = params.get("lag_in_frames")? {
        config.lag_in_frames = lag_in_frames;
    }

    // スレッド数
    if let Some(threads) = params.get("threads")? {
        config.threads = threads;
    }

    // エラー耐性モード
    config.error_resilient = params
        .get("error_resilient")?
        .unwrap_or(config.error_resilient);

    // キーフレーム間隔
    if let Some(keyframe_interval) = params.get("keyframe_interval")? {
        config.keyframe_interval = keyframe_interval;
    }

    // フレームドロップ閾値
    if let Some(frame_drop_threshold) = params.get("frame_drop_threshold")? {
        config.frame_drop_threshold = frame_drop_threshold;
    }

    // 以降はVP8固有の設定
    let mut vp8_config = config
        .vp8_config
        .take()
        .unwrap_or_else(|| shiguredo_libvpx::Vp8Config {
            noise_sensitivity: None,
            static_threshold: None,
            token_partitions: None,
            max_intra_bitrate_pct: None,
            arnr_config: None,
        });

    if let Some(noise_sensitivity) = params.get("noise_sensitivity")? {
        vp8_config.noise_sensitivity = Some(noise_sensitivity);
    }
    if let Some(static_threshold) = params.get("static_threshold")? {
        vp8_config.static_threshold = Some(static_threshold);
    }
    if let Some(token_partitions) = params.get("token_partitions")? {
        vp8_config.token_partitions = Some(token_partitions);
    }
    if let Some(max_intra_bitrate_pct) = params.get("max_intra_bitrate_pct")? {
        vp8_config.max_intra_bitrate_pct = Some(max_intra_bitrate_pct);
    }

    // ARNR設定
    if let Some(arnr_config) = params.get_with("arnr_config", JsonObject::new)? {
        let mut arnr = vp8_config.arnr_config.unwrap_or_default();
        arnr.max_frames = arnr_config.get("max_frames")?.unwrap_or(arnr.max_frames);
        arnr.strength = arnr_config.get("strength")?.unwrap_or(arnr.strength);
        arnr.filter_type = arnr_config.get("filter_type")?.unwrap_or(arnr.filter_type);
        vp8_config.arnr_config = Some(arnr);
    }

    config.vp8_config = Some(vp8_config);
    Ok(())
}

fn update_vp9_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut shiguredo_libvpx::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    // 基本的なエンコーダーパラメーター
    config.min_quantizer = params.get("min_quantizer")?.unwrap_or(config.min_quantizer);
    config.max_quantizer = params.get("max_quantizer")?.unwrap_or(config.max_quantizer);
    config.cq_level = params.get("cq_level")?.unwrap_or(config.cq_level);

    if let Some(cpu_used) = params.get("cpu_used")? {
        config.cpu_used = cpu_used;
    }

    // エンコード期限設定
    config.deadline = params
        .get_with("deadline", |v| match v.to_unquoted_string_str()?.as_ref() {
            "best" => Ok(shiguredo_libvpx::EncodingDeadline::Best),
            "good" => Ok(shiguredo_libvpx::EncodingDeadline::Good),
            "realtime" => Ok(shiguredo_libvpx::EncodingDeadline::Realtime),
            _ => Err(v.invalid("unknown 'deadline' value")),
        })?
        .unwrap_or(config.deadline.clone());

    // レート制御モード
    config.rate_control = params
        .get_with("rate_control", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "vbr" => Ok(shiguredo_libvpx::RateControlMode::Vbr),
                "cbr" => Ok(shiguredo_libvpx::RateControlMode::Cbr),
                "cq" => Ok(shiguredo_libvpx::RateControlMode::Cq),
                _ => Err(v.invalid("unknown 'rate_control' value")),
            }
        })?
        .unwrap_or(config.rate_control.clone());

    // 先読みフレーム数
    if let Some(lag_in_frames) = params.get("lag_in_frames")? {
        config.lag_in_frames = lag_in_frames;
    }

    // スレッド数
    if let Some(threads) = params.get("threads")? {
        config.threads = threads;
    }

    // エラー耐性モード
    config.error_resilient = params
        .get("error_resilient")?
        .unwrap_or(config.error_resilient);

    // キーフレーム間隔
    if let Some(keyframe_interval) = params.get("keyframe_interval")? {
        config.keyframe_interval = keyframe_interval;
    }

    // フレームドロップ閾値
    if let Some(frame_drop_threshold) = params.get("frame_drop_threshold")? {
        config.frame_drop_threshold = frame_drop_threshold;
    }

    // 以降はVP9固有の設定
    let mut vp9_config = config
        .vp9_config
        .take()
        .unwrap_or_else(|| shiguredo_libvpx::Vp9Config {
            aq_mode: None,
            noise_sensitivity: None,
            tile_columns: None,
            tile_rows: None,
            row_mt: false,
            frame_parallel_decoding: false,
            tune_content: None,
        });

    // VP9固有パラメータの設定
    if let Some(aq_mode) = params.get("aq_mode")? {
        vp9_config.aq_mode = Some(aq_mode);
    }
    if let Some(noise_sensitivity) = params.get("noise_sensitivity")? {
        vp9_config.noise_sensitivity = Some(noise_sensitivity);
    }
    if let Some(tile_columns) = params.get("tile_columns")? {
        vp9_config.tile_columns = Some(tile_columns);
    }
    if let Some(tile_rows) = params.get("tile_rows")? {
        vp9_config.tile_rows = Some(tile_rows);
    }
    vp9_config.row_mt = params.get("row_mt")?.unwrap_or(vp9_config.row_mt);
    vp9_config.frame_parallel_decoding = params
        .get("frame_parallel_decoding")?
        .unwrap_or(vp9_config.frame_parallel_decoding);

    // コンテンツタイプ最適化
    vp9_config.tune_content = params
        .get_with("tune_content", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "default" => Ok(shiguredo_libvpx::ContentType::Default),
                "screen" => Ok(shiguredo_libvpx::ContentType::Screen),
                _ => Err(v.invalid("unknown 'tune_content' value")),
            }
        })?
        .or(vp9_config.tune_content);

    config.vp9_config = Some(vp9_config);
    Ok(())
}
