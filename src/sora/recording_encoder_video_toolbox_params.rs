use crate::sora::recording_layout::DEFAULT_LAYOUT_JSON;
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
    let params = default
        .value()
        .to_member("video_toolbox_h264_encode_params")?
        .required()?;
    update_h264_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_h264_encode_params(value, &mut config)?;

    Ok(config)
}

pub fn parse_h265_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<EncoderConfig, nojson::JsonParseError> {
    let mut config = default_h265_encoder_config();

    // デフォルトレイアウトの設定を反映
    let default = nojson::RawJson::parse_jsonc(DEFAULT_LAYOUT_JSON)?.0;
    let params = default
        .value()
        .to_member("video_toolbox_h265_encode_params")?
        .required()?;
    update_h265_encode_params(params, &mut config)?;

    // 実際のレイアウトの設定を反映
    update_h265_encode_params(value, &mut config)?;

    Ok(config)
}

fn update_h264_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - average_bitrate

    if let Some(v) = params
        .to_member("prioritize_speed_over_quality")?
        .optional()
    {
        config.prioritize_encoding_speed_over_quality = v.try_into()?;
    }
    if let Some(v) = params.to_member("real_time")?.optional() {
        config.real_time = v.try_into()?;
    }
    if let Some(v) = params.to_member("maximize_power_efficiency")?.optional() {
        config.maximize_power_efficiency = v.try_into()?;
    }
    if let Some(v) = params.to_member("allow_temporal_compression")?.optional() {
        config.allow_temporal_compression = v.try_into()?;
    }
    if let Some(v) = params.to_member("max_key_frame_interval")?.optional() {
        config.max_key_frame_interval = Some(v.try_into()?);
    }
    if let Some(v) = params
        .to_member("max_key_frame_interval_duration")?
        .optional()
    {
        config.max_key_frame_interval_duration = Some(Duration::from_secs_f64(v.try_into()?));
    }
    if let Some(v) = params.to_member("max_frame_delay_count")?.optional() {
        config.max_frame_delay_count = Some(v.try_into()?);
    }

    let CodecConfig::H264(codec) = &mut config.codec else {
        unreachable!();
    };

    if let Some(v) = params.to_member("profile_level")?.optional() {
        codec.profile = match v.to_unquoted_string_str()?.as_ref() {
            "baseline" => H264Profile::Baseline,
            "main" => H264Profile::Main,
            "high" => H264Profile::High,
            _ => return Err(v.invalid("unknown 'profile_level' value for H.264")),
        };
    }

    if let Some(v) = params.to_member("h264_entropy_mode")?.optional() {
        codec.entropy_mode = match v.to_unquoted_string_str()?.as_ref() {
            "cavlc" => H264EntropyMode::Cavlc,
            "cabac" => H264EntropyMode::Cabac,
            _ => return Err(v.invalid("unknown 'h264_entropy_mode' value")),
        };
    }

    Ok(())
}

fn update_h265_encode_params(
    params: nojson::RawJsonValue<'_, '_>,
    config: &mut EncoderConfig,
) -> Result<(), nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - average_bitrate

    // H.265 ではこれが false だとエラーになるため、常に true を指定する
    config.prioritize_encoding_speed_over_quality = true;
    if let Some(v) = params.to_member("real_time")?.optional() {
        config.real_time = v.try_into()?;
    }
    if let Some(v) = params.to_member("maximize_power_efficiency")?.optional() {
        config.maximize_power_efficiency = v.try_into()?;
    }
    if let Some(v) = params.to_member("allow_temporal_compression")?.optional() {
        config.allow_temporal_compression = v.try_into()?;
    }
    if let Some(v) = params.to_member("max_key_frame_interval")?.optional() {
        config.max_key_frame_interval = Some(v.try_into()?);
    }
    if let Some(v) = params
        .to_member("max_key_frame_interval_duration")?
        .optional()
    {
        config.max_key_frame_interval_duration = Some(Duration::from_secs_f64(v.try_into()?));
    }
    if let Some(v) = params.to_member("max_frame_delay_count")?.optional() {
        config.max_frame_delay_count = Some(v.try_into()?);
    }

    let CodecConfig::Hevc(codec) = &mut config.codec else {
        unreachable!();
    };

    if let Some(v) = params.to_member("allow_open_gop")?.optional() {
        codec.allow_open_gop = v.try_into()?;
    }
    if let Some(v) = params.to_member("profile_level")?.optional() {
        codec.profile = match v.to_unquoted_string_str()?.as_ref() {
            "main" => HevcProfile::Main,
            "main10" => HevcProfile::Main10,
            _ => return Err(v.invalid("unknown 'profile_level' value for H.265")),
        };
    }

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
