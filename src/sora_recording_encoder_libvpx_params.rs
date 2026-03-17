use crate::sora_recording_layout::DEFAULT_LAYOUT_JSON;

pub fn parse_vp8_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_libvpx::EncoderConfig, nojson::JsonParseError> {
    let mut config = shiguredo_libvpx::EncoderConfig::new(
        2,
        2,
        shiguredo_libvpx::ImageFormat::I420,
        shiguredo_libvpx::CodecConfig::Vp8(shiguredo_libvpx::Vp8Config::default()),
    );

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("libvpx_vp8_encode_params")?
        .required()?;
    update_vp8_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_vp8_encode_params(value, &mut config)?;

    Ok(config)
}

pub fn parse_vp9_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_libvpx::EncoderConfig, nojson::JsonParseError> {
    let mut config = shiguredo_libvpx::EncoderConfig::new(
        2,
        2,
        shiguredo_libvpx::ImageFormat::I420,
        shiguredo_libvpx::CodecConfig::Vp9(shiguredo_libvpx::Vp9Config::default()),
    );

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("libvpx_vp9_encode_params")?
        .required()?;
    update_vp9_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_vp9_encode_params(value, &mut config)?;

    Ok(config)
}

fn update_vp8_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_libvpx::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    // 基本的なエンコーダーパラメーター
    if let Some(v) = params.to_member("min_quantizer")?.optional() {
        config.min_quantizer = v.try_into()?;
    }
    if let Some(v) = params.to_member("max_quantizer")?.optional() {
        config.max_quantizer = v.try_into()?;
    }
    if let Some(v) = params.to_member("cq_level")?.optional() {
        config.cq_level = v.try_into()?;
    }
    if let Some(v) = params.to_member("cpu_used")?.optional() {
        config.cpu_used = v.try_into()?;
    }

    // エンコード期限設定
    if let Some(v) = params.to_member("deadline")?.optional() {
        config.deadline = match v.to_unquoted_string_str()?.as_ref() {
            "best" => shiguredo_libvpx::EncodingDeadline::Best,
            "good" => shiguredo_libvpx::EncodingDeadline::Good,
            "realtime" => shiguredo_libvpx::EncodingDeadline::Realtime,
            _ => return Err(v.invalid("unknown 'deadline' value")),
        };
    }

    // レート制御モード
    if let Some(v) = params.to_member("rate_control")?.optional() {
        config.rate_control = match v.to_unquoted_string_str()?.as_ref() {
            "vbr" => shiguredo_libvpx::RateControlMode::Vbr,
            "cbr" => shiguredo_libvpx::RateControlMode::Cbr,
            "cq" => shiguredo_libvpx::RateControlMode::Cq,
            _ => return Err(v.invalid("unknown 'rate_control' value")),
        };
    }

    // 先読みフレーム数
    if let Some(v) = params.to_member("lag_in_frames")?.optional() {
        config.lag_in_frames = v.try_into()?;
    }

    // スレッド数
    if let Some(v) = params.to_member("threads")?.optional() {
        config.threads = v.try_into()?;
    }

    // エラー耐性モード
    if let Some(v) = params.to_member("error_resilient")?.optional() {
        config.error_resilient = v.try_into()?;
    }

    // キーフレーム間隔
    if let Some(v) = params.to_member("keyframe_interval")?.optional() {
        config.keyframe_interval = v.try_into()?;
    }

    // フレームドロップ閾値
    if let Some(v) = params.to_member("frame_drop_threshold")?.optional() {
        config.frame_drop_threshold = v.try_into()?;
    }

    // 以降はVP8固有の設定
    let shiguredo_libvpx::CodecConfig::Vp8(vp8_config) = &mut config.codec else {
        unreachable!();
    };

    if let Some(v) = params.to_member("noise_sensitivity")?.optional() {
        vp8_config.noise_sensitivity = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("static_threshold")?.optional() {
        vp8_config.static_threshold = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("token_partitions")?.optional() {
        vp8_config.token_partitions = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("max_intra_bitrate_pct")?.optional() {
        vp8_config.max_intra_bitrate_pct = Some(v.try_into()?);
    }

    // ARNR設定
    if let Some(v) = params.to_member("arnr_config")?.optional() {
        vp8_config.arnr_config = Some(shiguredo_libvpx::ArnrConfig {
            max_frames: v.to_member("max_frames")?.required()?.try_into()?,
            strength: v.to_member("strength")?.required()?.try_into()?,
            filter_type: v.to_member("filter_type")?.required()?.try_into()?,
        });
    }

    Ok(())
}

fn update_vp9_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_libvpx::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    // 基本的なエンコーダーパラメーター
    if let Some(v) = params.to_member("min_quantizer")?.optional() {
        config.min_quantizer = v.try_into()?;
    }
    if let Some(v) = params.to_member("max_quantizer")?.optional() {
        config.max_quantizer = v.try_into()?;
    }
    if let Some(v) = params.to_member("cq_level")?.optional() {
        config.cq_level = v.try_into()?;
    }
    if let Some(v) = params.to_member("cpu_used")?.optional() {
        config.cpu_used = v.try_into()?;
    }

    // エンコード期限設定
    if let Some(v) = params.to_member("deadline")?.optional() {
        config.deadline = match v.to_unquoted_string_str()?.as_ref() {
            "best" => shiguredo_libvpx::EncodingDeadline::Best,
            "good" => shiguredo_libvpx::EncodingDeadline::Good,
            "realtime" => shiguredo_libvpx::EncodingDeadline::Realtime,
            _ => return Err(v.invalid("unknown 'deadline' value")),
        };
    }

    // レート制御モード
    if let Some(v) = params.to_member("rate_control")?.optional() {
        config.rate_control = match v.to_unquoted_string_str()?.as_ref() {
            "vbr" => shiguredo_libvpx::RateControlMode::Vbr,
            "cbr" => shiguredo_libvpx::RateControlMode::Cbr,
            "cq" => shiguredo_libvpx::RateControlMode::Cq,
            _ => return Err(v.invalid("unknown 'rate_control' value")),
        };
    }

    // 先読みフレーム数
    if let Some(v) = params.to_member("lag_in_frames")?.optional() {
        config.lag_in_frames = v.try_into()?;
    }

    // スレッド数
    if let Some(v) = params.to_member("threads")?.optional() {
        config.threads = v.try_into()?;
    }

    // エラー耐性モード
    if let Some(v) = params.to_member("error_resilient")?.optional() {
        config.error_resilient = v.try_into()?;
    }

    // キーフレーム間隔
    if let Some(v) = params.to_member("keyframe_interval")?.optional() {
        config.keyframe_interval = v.try_into()?;
    }

    // フレームドロップ閾値
    if let Some(v) = params.to_member("frame_drop_threshold")?.optional() {
        config.frame_drop_threshold = v.try_into()?;
    }

    // 以降はVP9固有の設定
    let shiguredo_libvpx::CodecConfig::Vp9(vp9_config) = &mut config.codec else {
        unreachable!();
    };

    // VP9固有パラメータの設定
    if let Some(v) = params.to_member("aq_mode")?.optional() {
        vp9_config.aq_mode = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("noise_sensitivity")?.optional() {
        vp9_config.noise_sensitivity = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("tile_columns")?.optional() {
        vp9_config.tile_columns = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("tile_rows")?.optional() {
        vp9_config.tile_rows = Some(v.try_into()?);
    }
    if let Some(v) = params.to_member("row_mt")?.optional() {
        vp9_config.row_mt = v.try_into()?;
    }
    if let Some(v) = params.to_member("frame_parallel_decoding")?.optional() {
        vp9_config.frame_parallel_decoding = v.try_into()?;
    }

    // コンテンツタイプ最適化
    if let Some(v) = params.to_member("tune_content")?.optional() {
        vp9_config.tune_content = Some(match v.to_unquoted_string_str()?.as_ref() {
            "default" => shiguredo_libvpx::ContentType::Default,
            "screen" => shiguredo_libvpx::ContentType::Screen,
            _ => return Err(v.invalid("unknown 'tune_content' value")),
        });
    }

    Ok(())
}
