use crate::json::JsonObject;
use shiguredo_video_toolbox::{EncoderConfig, H264EntropyMode, ProfileLevel};
use std::time::Duration;

pub fn parse_h264_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<EncoderConfig, nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate
    let params = JsonObject::new(value)?;
    let mut config = EncoderConfig::default();

    // 速度と品質のバランス設定
    if let Some(prioritize_speed) = params.get("prioritize_speed_over_quality")? {
        config.prioritize_speed_over_quality = prioritize_speed;
    }
    if let Some(real_time) = params.get("real_time")? {
        config.real_time = real_time;
    }
    if let Some(maximize_power) = params.get("maximize_power_efficiency")? {
        config.maximize_power_efficiency = maximize_power;
    }

    // フレーム構造設定
    if let Some(allow_reordering) = params.get("allow_frame_reordering")? {
        config.allow_frame_reordering = allow_reordering;
    }
    if let Some(allow_open_gop) = params.get("allow_open_gop")? {
        config.allow_open_gop = allow_open_gop;
    }
    if let Some(allow_temporal) = params.get("allow_temporal_compression")? {
        config.allow_temporal_compression = allow_temporal;
    }

    // キーフレーム間隔設定（フレーム数）
    config.max_key_frame_interval = params.get("max_key_frame_interval")?;

    // キーフレーム間隔設定（秒数）
    config.max_key_frame_interval_duration = params
        .get_with("max_key_frame_interval_duration", |v| {
            Ok(Duration::from_secs_f64(v.try_into()?))
        })?;

    // プロファイルレベル設定
    config.profile_level = params
        .get_with("profile_level", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "baseline" => Ok(ProfileLevel::H264Baseline),
                "main" => Ok(ProfileLevel::H264Main),
                "high" => Ok(ProfileLevel::H264High),
                _ => Err(v.invalid("unknown 'profile_level' value for H.264")),
            }
        })?
        .unwrap_or(config.profile_level);

    // H.264エントロピー符号化モード
    config.h264_entropy_mode = params
        .get_with("h264_entropy_mode", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "cavlc" => Ok(H264EntropyMode::Cavlc),
                "cabac" => Ok(H264EntropyMode::Cabac),
                _ => Err(v.invalid("unknown 'h264_entropy_mode' value")),
            }
        })?
        .unwrap_or(config.h264_entropy_mode);

    // フレーム遅延制限
    config.max_frame_delay_count = params.get("max_frame_delay_count")?;

    // 並列処理設定
    config.use_parallelization = params.get("use_parallelization")?.unwrap_or_default();

    Ok(config)
}

pub fn parse_h265_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<EncoderConfig, nojson::JsonParseError> {
    // [NOTE] 以下は後で別途設定するので、ここではパースしない:
    // - width
    // - height
    // - fps_numerator
    // - fps_denominator
    // - target_bitrate
    let params = JsonObject::new(value)?;
    let mut config = EncoderConfig::default();

    // H.265用のデフォルト設定
    config.profile_level = ProfileLevel::H265Main;

    // 基本的なエンコーダーパラメーター
    if let Some(bitrate) = params.get("target_bitrate")? {
        config.target_bitrate = bitrate;
    }
    if let Some(fps_num) = params.get("fps_numerator")? {
        config.fps_numerator = fps_num;
    }
    if let Some(fps_den) = params.get("fps_denominator")? {
        config.fps_denominator = fps_den;
    }

    // 速度と品質のバランス設定
    if let Some(prioritize_speed) = params.get("prioritize_speed_over_quality")? {
        config.prioritize_speed_over_quality = prioritize_speed;
    }
    if let Some(real_time) = params.get("real_time")? {
        config.real_time = real_time;
    }
    if let Some(maximize_power) = params.get("maximize_power_efficiency")? {
        config.maximize_power_efficiency = maximize_power;
    }

    // フレーム構造設定
    if let Some(allow_reordering) = params.get("allow_frame_reordering")? {
        config.allow_frame_reordering = allow_reordering;
    }
    if let Some(allow_open_gop) = params.get("allow_open_gop")? {
        config.allow_open_gop = allow_open_gop;
    }
    if let Some(allow_temporal) = params.get("allow_temporal_compression")? {
        config.allow_temporal_compression = allow_temporal;
    }

    // キーフレーム間隔設定（フレーム数）
    config.max_key_frame_interval = params.get("max_key_frame_interval")?;

    // キーフレーム間隔設定（秒数）
    config.max_key_frame_interval_duration = params
        .get_with("max_key_frame_interval_duration", |v| {
            Ok(Duration::from_secs_f64(v.try_into()?))
        })?;

    // プロファイルレベル設定（H.265用）
    config.profile_level = params
        .get_with("profile_level", |v| {
            match v.to_unquoted_string_str()?.as_ref() {
                "main" => Ok(ProfileLevel::H265Main),
                "main10" => Ok(ProfileLevel::H265Main10),
                _ => Err(v.invalid("unknown 'profile_level' value for H.265")),
            }
        })?
        .unwrap_or(config.profile_level);

    // フレーム遅延制限
    config.max_frame_delay_count = params.get("max_frame_delay_count")?;

    // 並列処理設定
    config.use_parallelization = params.get("use_parallelization")?.unwrap_or_default();

    Ok(config)
}
