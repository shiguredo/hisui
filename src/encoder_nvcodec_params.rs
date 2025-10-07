use crate::json::JsonObject;
use crate::layout::DEFAULT_LAYOUT_JSON;

pub fn parse_h264_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<shiguredo_nvcodec::EncoderConfig, nojson::JsonParseError> {
    let mut config = shiguredo_nvcodec::EncoderConfig::default();

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
    let mut config = shiguredo_nvcodec::EncoderConfig::default();

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
    let mut config = shiguredo_nvcodec::EncoderConfig::default();

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
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    update_common_encode_params(&params, config)?;

    // H.264固有のプロファイル設定
    config.profile = params
        .get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
            "baseline" => Ok(shiguredo_nvcodec::Profile::H264_BASELINE),
            "main" => Ok(shiguredo_nvcodec::Profile::H264_MAIN),
            "high" => Ok(shiguredo_nvcodec::Profile::H264_HIGH),
            "high_10" => Ok(shiguredo_nvcodec::Profile::H264_HIGH_10),
            "high_422" => Ok(shiguredo_nvcodec::Profile::H264_HIGH_422),
            "high_444" => Ok(shiguredo_nvcodec::Profile::H264_HIGH_444),
            _ => Err(v.invalid("unknown 'profile' value for H.264")),
        })?
        .or(config.profile);

    Ok(())
}

fn update_h265_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    update_common_encode_params(&params, config)?;

    // H.265固有のプロファイル設定
    config.profile = params
        .get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
            "main" => Ok(shiguredo_nvcodec::Profile::HEVC_MAIN),
            "main10" => Ok(shiguredo_nvcodec::Profile::HEVC_MAIN10),
            "frext" => Ok(shiguredo_nvcodec::Profile::HEVC_FREXT),
            _ => Err(v.invalid("unknown 'profile' value for H.265")),
        })?
        .or(config.profile);

    Ok(())
}

fn update_av1_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate

    update_common_encode_params(&params, config)?;

    // AV1固有のプロファイル設定
    config.profile = params
        .get_with("profile", |v| match v.to_unquoted_string_str()?.as_ref() {
            "main" => Ok(shiguredo_nvcodec::Profile::AV1_MAIN),
            _ => Err(v.invalid("unknown 'profile' value for AV1")),
        })?
        .or(config.profile);

    Ok(())
}

fn update_common_encode_params(
    params: &JsonObject<'_, '_>,
    config: &mut shiguredo_nvcodec::EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // 最大エンコードサイズ（動的解像度変更用）
    config.max_encode_width = params.get("max_encode_width")?.or(config.max_encode_width);
    config.max_encode_height = params
        .get("max_encode_height")?
        .or(config.max_encode_height);

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
    config.idr_period = params.get("idr_period")?.or(config.idr_period);
    config.frame_interval_p = params
        .get("frame_interval_p")?
        .unwrap_or(config.frame_interval_p);

    // デバイスID
    config.device_id = params.get("device_id")?.unwrap_or(config.device_id);

    Ok(())
}
