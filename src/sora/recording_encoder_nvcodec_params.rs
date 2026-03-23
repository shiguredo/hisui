use crate::sora::recording_layout::DEFAULT_LAYOUT_JSON;

pub fn parse_h264_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::EncoderConfig, nojson::JsonParseError> {
    let mut config = default_h264_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("nvcodec_h264_encode_params")?
        .required()?;
    update_h264_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_h264_encode_params(value, &mut config)?;

    Ok(config)
}

pub fn parse_h265_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::EncoderConfig, nojson::JsonParseError> {
    let mut config = default_h265_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("nvcodec_h265_encode_params")?
        .required()?;
    update_h265_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_h265_encode_params(value, &mut config)?;

    Ok(config)
}

pub fn parse_av1_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::EncoderConfig, nojson::JsonParseError> {
    let mut config = default_av1_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("nvcodec_av1_encode_params")?
        .required()?;
    update_av1_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_av1_encode_params(value, &mut config)?;

    Ok(config)
}

fn update_h264_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(params, config)?;

    // H.264 固有の設定
    let shiguredo_nvcodec::CodecConfig::H264(codec) = &mut config.codec else {
        unreachable!();
    };
    if let Some(v) = params.to_member("profile")?.optional() {
        codec.profile = Some(match v.to_unquoted_string_str()?.as_ref() {
            "baseline" => shiguredo_nvcodec::H264Profile::Baseline,
            "main" => shiguredo_nvcodec::H264Profile::Main,
            "high" => shiguredo_nvcodec::H264Profile::High,
            "high_10" => shiguredo_nvcodec::H264Profile::High10,
            "high_422" => shiguredo_nvcodec::H264Profile::High422,
            "high_444" => shiguredo_nvcodec::H264Profile::High444,
            _ => return Err(v.invalid("unknown 'profile' value for H.264")),
        });
    }
    if let Some(v) = params.to_member("idr_period")?.optional() {
        codec.idr_period = Some(v.try_into()?);
    }

    Ok(())
}

fn update_h265_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(params, config)?;

    // H.265 固有の設定
    let shiguredo_nvcodec::CodecConfig::Hevc(codec) = &mut config.codec else {
        unreachable!();
    };
    if let Some(v) = params.to_member("profile")?.optional() {
        codec.profile = Some(match v.to_unquoted_string_str()?.as_ref() {
            "main" => shiguredo_nvcodec::HevcProfile::Main,
            "main10" => shiguredo_nvcodec::HevcProfile::Main10,
            "frext" => shiguredo_nvcodec::HevcProfile::Frext,
            _ => return Err(v.invalid("unknown 'profile' value for H.265")),
        });
    }
    if let Some(v) = params.to_member("idr_period")?.optional() {
        codec.idr_period = Some(v.try_into()?);
    }

    Ok(())
}

fn update_av1_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(params, config)?;

    // AV1 固有の設定
    let shiguredo_nvcodec::CodecConfig::Av1(codec) = &mut config.codec else {
        unreachable!();
    };
    if let Some(v) = params.to_member("profile")?.optional() {
        codec.profile = Some(match v.to_unquoted_string_str()?.as_ref() {
            "main" => shiguredo_nvcodec::Av1Profile::Main,
            _ => return Err(v.invalid("unknown 'profile' value for AV1")),
        });
    }
    if let Some(v) = params.to_member("idr_period")?.optional() {
        codec.idr_period = Some(v.try_into()?);
    }

    Ok(())
}

fn update_common_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // プリセット設定
    if let Some(v) = params.to_member("preset")?.optional() {
        config.preset = match v.to_unquoted_string_str()?.as_ref() {
            "p1" => shiguredo_nvcodec::Preset::P1,
            "p2" => shiguredo_nvcodec::Preset::P2,
            "p3" => shiguredo_nvcodec::Preset::P3,
            "p4" => shiguredo_nvcodec::Preset::P4,
            "p5" => shiguredo_nvcodec::Preset::P5,
            "p6" => shiguredo_nvcodec::Preset::P6,
            "p7" => shiguredo_nvcodec::Preset::P7,
            _ => return Err(v.invalid("unknown 'preset' value")),
        };
    }

    // チューニング情報
    if let Some(v) = params.to_member("tuning_info")?.optional() {
        config.tuning_info = match v.to_unquoted_string_str()?.as_ref() {
            "high_quality" => shiguredo_nvcodec::TuningInfo::HIGH_QUALITY,
            "low_latency" => shiguredo_nvcodec::TuningInfo::LOW_LATENCY,
            "ultra_low_latency" => shiguredo_nvcodec::TuningInfo::ULTRA_LOW_LATENCY,
            "lossless" => shiguredo_nvcodec::TuningInfo::LOSSLESS,
            _ => return Err(v.invalid("unknown 'tuning_info' value")),
        };
    }

    // レート制御モード
    if let Some(v) = params.to_member("rate_control_mode")?.optional() {
        config.rate_control_mode = match v.to_unquoted_string_str()?.as_ref() {
            "const_qp" => shiguredo_nvcodec::RateControlMode::ConstQp,
            "vbr" => shiguredo_nvcodec::RateControlMode::Vbr,
            "cbr" => shiguredo_nvcodec::RateControlMode::Cbr,
            _ => return Err(v.invalid("unknown 'rate_control_mode' value")),
        };
    }

    // GOP設定
    if let Some(v) = params.to_member("gop_length")?.optional() {
        config.gop_length = Some(v.try_into()?);
    }

    // デバイスID
    if let Some(v) = params.to_member("device_id")?.optional() {
        config.device_id = v.try_into()?;
    }

    Ok(())
}

fn default_h264_encoder_config() -> shiguredo_nvcodec::EncoderConfig {
    default_encoder_config(shiguredo_nvcodec::CodecConfig::H264(
        shiguredo_nvcodec::H264EncoderConfig {
            profile: None,
            idr_period: None,
        },
    ))
}

fn default_h265_encoder_config() -> shiguredo_nvcodec::EncoderConfig {
    default_encoder_config(shiguredo_nvcodec::CodecConfig::Hevc(
        shiguredo_nvcodec::HevcEncoderConfig {
            profile: None,
            idr_period: None,
        },
    ))
}

fn default_av1_encoder_config() -> shiguredo_nvcodec::EncoderConfig {
    default_encoder_config(shiguredo_nvcodec::CodecConfig::Av1(
        shiguredo_nvcodec::Av1EncoderConfig {
            profile: None,
            idr_period: None,
        },
    ))
}

fn default_encoder_config(
    codec: shiguredo_nvcodec::CodecConfig,
) -> shiguredo_nvcodec::EncoderConfig {
    shiguredo_nvcodec::EncoderConfig {
        codec,
        width: 640,
        height: 480,
        max_encode_width: None,
        max_encode_height: None,
        framerate_num: 30,
        framerate_den: 1,
        average_bitrate: Some(5_000_000),
        preset: shiguredo_nvcodec::Preset::P4,
        tuning_info: shiguredo_nvcodec::TuningInfo::LOW_LATENCY,
        rate_control_mode: shiguredo_nvcodec::RateControlMode::Vbr,
        gop_length: None,
        frame_interval_p: 1,
        buffer_format: shiguredo_nvcodec::BufferFormat::Nv12,
        device_id: 0,
    }
}
