use crate::TrackId;

use super::{BuildObswsRecordSourcePlanError, ObswsRecordSourcePlan};

/// webrtc_source 用のソースプランを構築する。
///
/// webrtc_source は WebRTC remote track からフレームを受け取るため、
/// 自律的な source processor は生成しない。video_track_id のみ確保して、
/// 実際のフレーム publish は p2p_session 側の HisuiAttachWebRtcVideoTrack で行う。
pub fn build_record_source_plan(
    source_key: &str,
) -> Result<ObswsRecordSourcePlan, BuildObswsRecordSourcePlanError> {
    let video_track_id = TrackId::new(format!("webrtc_source:video:{source_key}"));

    Ok(ObswsRecordSourcePlan {
        source_processor_ids: vec![],
        source_video_track_id: Some(video_track_id),
        source_audio_track_id: None,
        requests: vec![],
    })
}

/// I420 フレームに chroma key を適用して I420A フレームを生成する。
///
/// key_u, key_v は背景色の U, V 成分。tolerance は UV 色差の許容値。
/// UV 色差がしきい値以下のピクセルを透明 (alpha=0) にする。
pub fn apply_chroma_key(
    i420_data: &[u8],
    width: usize,
    height: usize,
    key_u: u8,
    key_v: u8,
    tolerance: i32,
) -> Vec<u8> {
    let y_size = width * height;
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let uv_size = uv_width * uv_height;

    // I420A = I420 + alpha plane (Y と同じサイズ)
    let mut i420a_data = Vec::with_capacity(i420_data.len() + y_size);
    i420a_data.extend_from_slice(i420_data);

    let u_plane = &i420_data[y_size..y_size + uv_size];
    let v_plane = &i420_data[y_size + uv_size..y_size + 2 * uv_size];

    let tolerance_sq = tolerance as i64 * tolerance as i64;

    // alpha plane を生成（各ピクセルが属する UV ブロックの色差で判定）
    let mut alpha_plane = vec![255u8; y_size];
    for y in 0..height {
        let uv_y = y / 2;
        for x in 0..width {
            let uv_x = x / 2;
            let uv_idx = uv_y * uv_width + uv_x;
            let u = u_plane[uv_idx] as i64;
            let v = v_plane[uv_idx] as i64;
            let du = u - key_u as i64;
            let dv = v - key_v as i64;
            let dist_sq = du * du + dv * dv;
            if dist_sq <= tolerance_sq {
                alpha_plane[y * width + x] = 0;
            }
        }
    }

    i420a_data.extend_from_slice(&alpha_plane);
    i420a_data
}

/// #RRGGBB 形式の色文字列を RGB に変換する。
pub fn parse_hex_color(color: &str) -> Option<(u8, u8, u8)> {
    let color = color.strip_prefix('#')?;
    if color.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&color[0..2], 16).ok()?;
    let g = u8::from_str_radix(&color[2..4], 16).ok()?;
    let b = u8::from_str_radix(&color[4..6], 16).ok()?;
    Some((r, g, b))
}

/// RGB を BT.601 で YUV に変換し、U と V を返す。
pub fn rgb_to_uv_bt601(r: u8, g: u8, b: u8) -> (u8, u8) {
    let r = r as f64;
    let g = g as f64;
    let b = b as f64;
    let u = (-0.1687 * r - 0.3313 * g + 0.5 * b + 128.0).clamp(0.0, 255.0) as u8;
    let v = (0.5 * r - 0.4187 * g - 0.0813 * b + 128.0).clamp(0.0, 255.0) as u8;
    (u, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#00FF00"), Some((0, 255, 0)));
        assert_eq!(parse_hex_color("#FF0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_color("#000000"), Some((0, 0, 0)));
        assert_eq!(parse_hex_color("#FFFFFF"), Some((255, 255, 255)));
        assert_eq!(parse_hex_color("00FF00"), None); // # なし
        assert_eq!(parse_hex_color("#0FF"), None); // 短すぎ
    }

    #[test]
    fn test_rgb_to_uv_bt601_green() {
        // 純緑は U≈43, V≈21 付近
        let (u, v) = rgb_to_uv_bt601(0, 255, 0);
        assert!((u as i32 - 43).unsigned_abs() < 3);
        assert!((v as i32 - 21).unsigned_abs() < 3);
    }

    #[test]
    fn test_apply_chroma_key_all_transparent() {
        // 2x2 の均一色フレーム（key color と完全一致）
        let width = 2;
        let height = 2;
        let key_u = 128u8;
        let key_v = 128u8;
        // Y plane: 4 bytes, U plane: 1 byte, V plane: 1 byte
        let mut i420 = vec![128u8; 4]; // Y
        i420.push(key_u); // U
        i420.push(key_v); // V

        let result = apply_chroma_key(&i420, width, height, key_u, key_v, 10);
        // I420A = I420 (6 bytes) + alpha (4 bytes) = 10 bytes
        assert_eq!(result.len(), 10);
        // alpha plane は全て 0（透明）
        assert_eq!(&result[6..], &[0, 0, 0, 0]);
    }

    #[test]
    fn test_apply_chroma_key_all_opaque() {
        // 2x2 の均一色フレーム（key color と大きく異なる）
        let width = 2;
        let height = 2;
        let key_u = 0u8;
        let key_v = 0u8;
        let mut i420 = vec![128u8; 4]; // Y
        i420.push(200); // U (key から遠い)
        i420.push(200); // V (key から遠い)

        let result = apply_chroma_key(&i420, width, height, key_u, key_v, 10);
        // alpha plane は全て 255（不透明）
        assert_eq!(&result[6..], &[255, 255, 255, 255]);
    }
}
