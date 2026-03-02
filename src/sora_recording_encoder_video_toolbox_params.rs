use crate::json::JsonObject;
use crate::sora_recording_layout::DEFAULT_LAYOUT_JSON;
use shiguredo_video_toolbox::{
    CodecConfig, EncoderConfig, H264EncoderConfig, H264EntropyMode, H264Profile, HevcEncoderConfig,
    HevcProfile, PixelFormat,
};
use std::time::Duration;

pub fn parse_h264_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<EncoderConfig, nojson::JsonParseError> {
    let mut config = default_h264_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("video_toolbox_h264_encode_params")?
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
) -> Result<EncoderConfig, nojson::JsonParseError> {
    let mut config = default_h265_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = JsonObject::new(
        default
            .value()
            .to_member("video_toolbox_h265_encode_params")?
            .required()?,
    )?;
    update_h265_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    let params = JsonObject::new(value)?;
    update_h265_encode_params(params, &mut config)?;

    Ok(config)
}

fn update_h264_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - average_bitrate

    config.prioritize_encoding_speed_over_quality = params
        .get("prioritize_speed_over_quality")?
        .unwrap_or(config.prioritize_encoding_speed_over_quality);

    config.real_time = params.get("real_time")?.unwrap_or(config.real_time);

    config.maximize_power_efficiency = params
        .get("maximize_power_efficiency")?
        .unwrap_or(config.maximize_power_efficiency);

    config.allow_temporal_compression = params
        .get("allow_temporal_compression")?
        .unwrap_or(config.allow_temporal_compression);

    if let Some(max_key_frame_interval) = params.get("max_key_frame_interval")? {
        config.max_key_frame_interval = Some(max_key_frame_interval);
    }

    if let Some(duration) = params.get_with("max_key_frame_interval_duration", |v| {
        Ok(Duration::from_secs_f64(v.try_into()?))
    })? {
        config.max_key_frame_interval_duration = Some(duration);
    }

    if let Some(max_frame_delay_count) = params.get("max_frame_delay_count")? {
        config.max_frame_delay_count = Some(max_frame_delay_count);
    }

    let CodecConfig::H264(codec) = &mut config.codec else {
        unreachable!();
    };

    codec.profile = params
        .get_with("profile_level", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "baseline" => Ok(H264Profile::Baseline),
                "main" => Ok(H264Profile::Main),
                "high" => Ok(H264Profile::High),
                _ => Err(v.invalid("unknown 'profile_level' value for H.264")),
            }
        })?
        .unwrap_or(codec.profile);

    codec.entropy_mode = params
        .get_with("h264_entropy_mode", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "cavlc" => Ok(H264EntropyMode::Cavlc),
                "cabac" => Ok(H264EntropyMode::Cabac),
                _ => Err(v.invalid("unknown 'h264_entropy_mode' value")),
            }
        })?
        .unwrap_or(codec.entropy_mode);

    Ok(())
}

fn update_h265_encode_params(
    params: JsonObject<'_, '_>,
    config: &mut EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - average_bitrate

    config.prioritize_encoding_speed_over_quality = true;
    config.real_time = params.get("real_time")?.unwrap_or(config.real_time);

    config.maximize_power_efficiency = params
        .get("maximize_power_efficiency")?
        .unwrap_or(config.maximize_power_efficiency);

    config.allow_temporal_compression = params
        .get("allow_temporal_compression")?
        .unwrap_or(config.allow_temporal_compression);

    if let Some(max_key_frame_interval) = params.get("max_key_frame_interval")? {
        config.max_key_frame_interval = Some(max_key_frame_interval);
    }

    if let Some(duration) = params.get_with("max_key_frame_interval_duration", |v| {
        Ok(Duration::from_secs_f64(v.try_into()?))
    })? {
        config.max_key_frame_interval_duration = Some(duration);
    }

    if let Some(max_frame_delay_count) = params.get("max_frame_delay_count")? {
        config.max_frame_delay_count = Some(max_frame_delay_count);
    }

    let CodecConfig::Hevc(codec) = &mut config.codec else {
        unreachable!();
    };

    codec.allow_open_gop = params
        .get("allow_open_gop")?
        .unwrap_or(codec.allow_open_gop);
    codec.profile = params
        .get_with("profile_level", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "main" => Ok(HevcProfile::Main),
                "main10" => Ok(HevcProfile::Main10),
                _ => Err(v.invalid("unknown 'profile_level' value for H.265")),
            }
        })?
        .unwrap_or(codec.profile);

    Ok(())
}

fn default_h264_encoder_config() -> EncoderConfig {
    default_encoder_config(CodecConfig::H264(H264EncoderConfig {
        profile: H264Profile::Main,
        entropy_mode: H264EntropyMode::Cabac,
    }))
}

fn default_h265_encoder_config() -> EncoderConfig {
    default_encoder_config(CodecConfig::Hevc(HevcEncoderConfig {
        profile: HevcProfile::Main,
        allow_open_gop: true,
    }))
}

fn default_encoder_config(codec: CodecConfig) -> EncoderConfig {
    EncoderConfig {
        width: 640,
        height: 480,
        codec,
        pixel_format: PixelFormat::I420,
        average_bitrate: Some(5_000_000),
        fps_numerator: 30,
        fps_denominator: 1,
        prioritize_encoding_speed_over_quality: false,
        real_time: false,
        maximize_power_efficiency: false,
        allow_frame_reordering: false,
        allow_temporal_compression: true,
        max_key_frame_interval: None,
        max_key_frame_interval_duration: None,
        max_frame_delay_count: None,
    }
}
