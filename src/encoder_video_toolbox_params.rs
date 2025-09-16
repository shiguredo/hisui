use crate::json::JsonObject;
use crate::layout::DEFAULT_LAYOUT_JSON;
use shiguredo_video_toolbox::{EncoderConfig, H264EntropyMode, ProfileLevel};
use std::time::Duration;

pub fn parse_h264_encode_params(
    value: nojson::RawJsonValue<'_, '_>,
) -> Result<EncoderConfig, nojson::JsonParseError> {
    let mut config = EncoderConfig::default();

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
    let mut config = EncoderConfig {
        // H.265用のデフォルト設定
        profile_level: ProfileLevel::H265Main,
        ..Default::default()
    };

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
    // - target_bitrate

    // 速度と品質のバランス設定
    config.prioritize_speed_over_quality = params
        .get("prioritize_speed_over_quality")?
        .unwrap_or(config.prioritize_speed_over_quality);

    config.real_time = params
        .get("real_time")?
        .unwrap_or(config.real_time);

    config.maximize_power_efficiency = params
        .get("maximize_power_efficiency")?
        .unwrap_or(config.maximize_power_efficiency);

    // フレーム構造設定
    config.allow_open_gop = params
        .get("allow_open_gop")?
        .unwrap_or(config.allow_open_gop);

    config.allow_temporal_compression = params
        .get("allow_temporal_compression")?
        .unwrap_or(config.allow_temporal_compression);

    // キーフレーム間隔設定（フレーム数）
    if let Some(max_key_frame_interval) = params.get("max_key_frame_interval")? {
        config.max_key_frame_interval = Some(max_key_frame_interval);
    }

    // キーフレーム間隔設定（秒数）
    if let Some(duration) = params.get_with("max_key_frame_interval_duration", |v| {
        Ok(Duration::from_secs_f64(v.try_into()?))
    })? {
        config.max_key_frame_interval_duration = Some(duration);
    }

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
    if let Some(max_frame_delay_count) = params.get("max_frame_delay_count")? {
        config.max_frame_delay_count = Some(max_frame_delay_count);
    }

    // 並列処理設定
    config.use_parallelization = params
        .get("use_parallelization")?
        .unwrap_or(config.use_parallelization);

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
    // - target_bitrate

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
    config.real_time = params
        .get("real_time")?
        .unwrap_or(config.real_time);

    config.maximize_power_efficiency = params
        .get("maximize_power_efficiency")?
        .unwrap_or(config.maximize_power_efficiency);

    // H.265では特別な処理が必要
    if let Some(prioritize_speed) = params.get("prioritize_speed_over_quality")? {
        if prioritize_speed {
            config.prioritize_speed_over_quality = true;
        }
        // H.265 ではこれが false だとエラーになるため、true以外の場合はデフォルト値を保持
    } else {
        // パラメータが指定されていない場合は true に設定
        config.prioritize_speed_over_quality = true;
    }

    // フレーム構造設定
    config.allow_open_gop = params
        .get("allow_open_gop")?
        .unwrap_or(config.allow_open_gop);

    config.allow_temporal_compression = params
        .get("allow_temporal_compression")?
        .unwrap_or(config.allow_temporal_compression);

    // キーフレーム間隔設定（フレーム数）
    if let Some(max_key_frame_interval) = params.get("max_key_frame_interval")? {
        config.max_key_frame_interval = Some(max_key_frame_interval);
    }

    // キーフレーム間隔設定（秒数）
    if let Some(duration) = params.get_with("max_key_frame_interval_duration", |v| {
        Ok(Duration::from_secs_f64(v.try_into()?))
    })? {
        config.max_key_frame_interval_duration = Some(duration);
    }

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
    if let Some(max_frame_delay_count) = params.get("max_frame_delay_count")? {
        config.max_frame_delay_count = Some(max_frame_delay_count);
    }

    // 並列処理設定
    config.use_parallelization = params
        .get("use_parallelization")?
        .unwrap_or(config.use_parallelization);

    Ok(())
}

