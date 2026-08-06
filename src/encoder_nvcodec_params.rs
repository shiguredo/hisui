use crate::json::JsonObject;
use crate::layout::DEFAULT_LAYOUT_JSON;

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
    update_h264_encode_params(&params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_h264_encode_params(&params, &mut config)?;

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
    update_h265_encode_params(&params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_h265_encode_params(&params, &mut config)?;

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
    update_av1_encode_params(&params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_av1_encode_params(&params, &mut config)?;

    Ok(config)
}

fn update_h264_encode_params(
    params: &JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(params, config)?;

    // 2026.2.0 で CodecConfig::H264(H264EncoderConfig) にネストされた
    let shiguredo_nvcodec::CodecConfig::H264(codec) = &mut config.codec else {
        // default_h264_encoder_config で H264 として初期化しているので、この分岐は起きない
        unreachable!("nvcodec encoder config is not H.264");
    };

    if let Some(v) = params.get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
        "baseline" => Ok(shiguredo_nvcodec::H264Profile::Baseline),
        "main" => Ok(shiguredo_nvcodec::H264Profile::Main),
        "high" => Ok(shiguredo_nvcodec::H264Profile::High),
        "high_10" => Ok(shiguredo_nvcodec::H264Profile::High10),
        "high_422" => Ok(shiguredo_nvcodec::H264Profile::High422),
        "high_444" => Ok(shiguredo_nvcodec::H264Profile::High444),
        _ => Err(v.invalid("unknown 'profile' value for H.264")),
    })? {
        codec.profile = Some(v);
    }
    if let Some(v) = params.get::<u32>("idr_period")? {
        codec.idr_period = Some(v);
    }

    Ok(())
}

fn update_h265_encode_params(
    params: &JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(params, config)?;

    // 2026.2.0 で CodecConfig::Hevc(HevcEncoderConfig) にネストされた
    let shiguredo_nvcodec::CodecConfig::Hevc(codec) = &mut config.codec else {
        // default_h265_encoder_config で Hevc として初期化しているので、この分岐は起きない
        unreachable!("nvcodec encoder config is not H.265");
    };

    if let Some(v) = params.get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
        "main" => Ok(shiguredo_nvcodec::HevcProfile::Main),
        "main10" => Ok(shiguredo_nvcodec::HevcProfile::Main10),
        "frext" => Ok(shiguredo_nvcodec::HevcProfile::Frext),
        _ => Err(v.invalid("unknown 'profile' value for H.265")),
    })? {
        codec.profile = Some(v);
    }
    if let Some(v) = params.get::<u32>("idr_period")? {
        codec.idr_period = Some(v);
    }

    Ok(())
}

fn update_av1_encode_params(
    params: &JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - framerate_num
    // - framerate_den
    // - average_bitrate

    update_common_encode_params(params, config)?;

    // 2026.2.0 で CodecConfig::Av1(Av1EncoderConfig) にネストされた
    let shiguredo_nvcodec::CodecConfig::Av1(codec) = &mut config.codec else {
        // default_av1_encoder_config で Av1 として初期化しているので、この分岐は起きない
        unreachable!("nvcodec encoder config is not AV1");
    };

    if let Some(v) = params.get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
        "main" => Ok(shiguredo_nvcodec::Av1Profile::Main),
        _ => Err(v.invalid("unknown 'profile' value for AV1")),
    })? {
        codec.profile = Some(v);
    }
    if let Some(v) = params.get::<u32>("idr_period")? {
        codec.idr_period = Some(v);
    }

    Ok(())
}

fn update_common_encode_params(
    params: &JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // プリセット設定
    if let Some(v) = params.get_with("preset", |v| match v.to_unquoted_string_str()?.as_ref() {
        "p1" => Ok(shiguredo_nvcodec::Preset::P1),
        "p2" => Ok(shiguredo_nvcodec::Preset::P2),
        "p3" => Ok(shiguredo_nvcodec::Preset::P3),
        "p4" => Ok(shiguredo_nvcodec::Preset::P4),
        "p5" => Ok(shiguredo_nvcodec::Preset::P5),
        "p6" => Ok(shiguredo_nvcodec::Preset::P6),
        "p7" => Ok(shiguredo_nvcodec::Preset::P7),
        _ => Err(v.invalid("unknown 'preset' value")),
    })? {
        config.preset = v;
    }

    // チューニング情報
    if let Some(v) = params.get_with("tuning_info", |v| {
        match v.to_unquoted_string_str()?.as_ref() {
            "high_quality" => Ok(shiguredo_nvcodec::TuningInfo::HIGH_QUALITY),
            "low_latency" => Ok(shiguredo_nvcodec::TuningInfo::LOW_LATENCY),
            "ultra_low_latency" => Ok(shiguredo_nvcodec::TuningInfo::ULTRA_LOW_LATENCY),
            "lossless" => Ok(shiguredo_nvcodec::TuningInfo::LOSSLESS),
            _ => Err(v.invalid("unknown 'tuning_info' value")),
        }
    })? {
        config.tuning_info = v;
    }

    // レート制御モード
    if let Some(v) = params.get_with("rate_control_mode", |v| {
        match v.to_unquoted_string_str()?.as_ref() {
            "const_qp" => Ok(shiguredo_nvcodec::RateControlMode::ConstQp),
            "vbr" => Ok(shiguredo_nvcodec::RateControlMode::Vbr),
            "cbr" => Ok(shiguredo_nvcodec::RateControlMode::Cbr),
            _ => Err(v.invalid("unknown 'rate_control_mode' value")),
        }
    })? {
        config.rate_control_mode = v;
    }

    // GOP 設定
    if let Some(v) = params.get::<u32>("gop_length")? {
        config.gop_length = Some(v);
    }
    // 2026.2.0 で idr_period は CodecConfig 側にネストされたので、ここでは扱わない

    // デバイス ID
    if let Some(v) = params.get::<i32>("device_id")? {
        config.device_id = v;
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
    // width / height / framerate / average_bitrate は encoder_nvcodec.rs 側で実値に上書きする
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
