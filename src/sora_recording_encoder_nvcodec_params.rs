use crate::json::JsonObject;
use crate::sora_recording_layout::DEFAULT_LAYOUT_JSON;

pub fn parse_h264_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::EncoderConfig, nojson::JsonParseError> {
    let mut config = default_h264_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("nvcodec_h264_encode_params")?
            .required()?,
    )?;
    update_h264_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_h264_encode_params(params, &mut config)?;

    Ok(config)
}

pub fn parse_h265_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::EncoderConfig, nojson::JsonParseError> {
    let mut config = default_h265_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("nvcodec_h265_encode_params")?
            .required()?,
    )?;
    update_h265_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_h265_encode_params(params, &mut config)?;

    Ok(config)
}

pub fn parse_av1_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::EncoderConfig, nojson::JsonParseError> {
    let mut config = default_av1_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("nvcodec_av1_encode_params")?
            .required()?,
    )?;
    update_av1_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_av1_encode_params(params, &mut config)?;

    Ok(config)
}

fn update_h264_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(&params, config)?;

    // H.264 固有の設定
    let profile = params.get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
        "baseline" => Ok(shiguredo_nvcodec::H264Profile::Baseline),
        "main" => Ok(shiguredo_nvcodec::H264Profile::Main),
        "high" => Ok(shiguredo_nvcodec::H264Profile::High),
        "high_10" => Ok(shiguredo_nvcodec::H264Profile::High10),
        "high_422" => Ok(shiguredo_nvcodec::H264Profile::High422),
        "high_444" => Ok(shiguredo_nvcodec::H264Profile::High444),
        _ => Err(v.invalid("unknown 'profile' value for H.264")),
    })?;
    let idr_period = params.get("idr_period")?;

    let shiguredo_nvcodec::CodecConfig::H264(codec) = &mut config.codec else {
        unreachable!();
    };
    codec.profile = profile.or(codec.profile);
    codec.idr_period = idr_period.or(codec.idr_period);

    Ok(())
}

fn update_h265_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(&params, config)?;

    // H.265 固有の設定
    let profile = params.get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
        "main" => Ok(shiguredo_nvcodec::HevcProfile::Main),
        "main10" => Ok(shiguredo_nvcodec::HevcProfile::Main10),
        "frext" => Ok(shiguredo_nvcodec::HevcProfile::Frext),
        _ => Err(v.invalid("unknown 'profile' value for H.265")),
    })?;
    let idr_period = params.get("idr_period")?;

    let shiguredo_nvcodec::CodecConfig::Hevc(codec) = &mut config.codec else {
        unreachable!();
    };
    codec.profile = profile.or(codec.profile);
    codec.idr_period = idr_period.or(codec.idr_period);

    Ok(())
}

fn update_av1_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(&params, config)?;

    // AV1 固有の設定
    let profile = params.get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
        "main" => Ok(shiguredo_nvcodec::Av1Profile::Main),
        _ => Err(v.invalid("unknown 'profile' value for AV1")),
    })?;
    let idr_period = params.get("idr_period")?;

    let shiguredo_nvcodec::CodecConfig::Av1(codec) = &mut config.codec else {
        unreachable!();
    };
    codec.profile = profile.or(codec.profile);
    codec.idr_period = idr_period.or(codec.idr_period);

    Ok(())
}

fn update_common_encode_params(
    params: &JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // プリセット設定
    config.preset = params
        .get_with("preset", |v| match v.to_unquoted_string_str()?.as_ref() {
            "p1" => Ok(shiguredo_nvcodec::Preset::P1),
            "p2" => Ok(shiguredo_nvcodec::Preset::P2),
            "p3" => Ok(shiguredo_nvcodec::Preset::P3),
            "p4" => Ok(shiguredo_nvcodec::Preset::P4),
            "p5" => Ok(shiguredo_nvcodec::Preset::P5),
            "p6" => Ok(shiguredo_nvcodec::Preset::P6),
            "p7" => Ok(shiguredo_nvcodec::Preset::P7),
            _ => Err(v.invalid("unknown 'preset' value")),
        })?
        .unwrap_or(config.preset);

    // チューニング情報
    config.tuning_info = params
        .get_with("tuning_info", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "high_quality" => Ok(shiguredo_nvcodec::TuningInfo::HIGH_QUALITY),
                "low_latency" => Ok(shiguredo_nvcodec::TuningInfo::LOW_LATENCY),
                "ultra_low_latency" => Ok(shiguredo_nvcodec::TuningInfo::ULTRA_LOW_LATENCY),
                "lossless" => Ok(shiguredo_nvcodec::TuningInfo::LOSSLESS),
                _ => Err(v.invalid("unknown 'tuning_info' value")),
            }
        })?
        .unwrap_or(config.tuning_info);

    // レート制御モード
    config.rate_control_mode = params
        .get_with("rate_control_mode", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "const_qp" => Ok(shiguredo_nvcodec::RateControlMode::ConstQp),
                "vbr" => Ok(shiguredo_nvcodec::RateControlMode::Vbr),
                "cbr" => Ok(shiguredo_nvcodec::RateControlMode::Cbr),
                _ => Err(v.invalid("unknown 'rate_control_mode' value")),
            }
        })?
        .unwrap_or(config.rate_control_mode);

    // GOP設定
    config.gop_length = params.get("gop_length")?.or(config.gop_length);

    // デバイスID
    config.device_id = params.get("device_id")?.unwrap_or(config.device_id);

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
