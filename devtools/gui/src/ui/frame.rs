//! I420 フレームを GPUI の [`RenderImage`] に変換する。

use std::sync::Arc;

use gpui::RenderImage;

use hisui_devtools_gui::p2p::OwnedVideoFrame;

/// I420 フレームを GPUI の RenderImage (BGRA バイト列) に変換する。
///
/// GPUI は BGRA バイト列 ([B][G][R][A] 順) を期待する。
/// libyuv のピクセル形式名は「ビッグエンディアンの 32 ビット値」であり、
/// リトルエンディアン環境ではメモリ順が逆になる。
/// `I420ToARGB` の出力はメモリ上 [B][G][R][A] となり、GPUI の期待と一致する。
pub(super) fn to_render_image(frame: &OwnedVideoFrame) -> Option<Arc<RenderImage>> {
    use shiguredo_libyuv::{ArgbImageMut, I420Image};

    let width = frame.width as usize;
    let height = frame.height as usize;
    if width == 0 || height == 0 {
        return None;
    }

    let src = I420Image {
        y: &frame.y,
        y_stride: frame.stride_y as usize,
        u: &frame.u,
        u_stride: frame.stride_u as usize,
        v: &frame.v,
        v_stride: frame.stride_v as usize,
    };
    let mut bgra = vec![0_u8; width * height * 4];
    let mut dst = ArgbImageMut {
        data: &mut bgra,
        stride: width * 4,
    };
    let size = shiguredo_libyuv::ImageSize { width, height };
    shiguredo_libyuv::i420_to_argb(&src, &mut dst, size).ok()?;

    let bgra_image = image::RgbaImage::from_raw(width as u32, height as u32, bgra)?;
    Some(Arc::new(RenderImage::new([image::Frame::new(bgra_image)])))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 単色の I420 フレームを作る。
    fn gray_i420(width: i32, height: i32) -> OwnedVideoFrame {
        let y_size = (width * height) as usize;
        let uv_width = (width + 1) / 2;
        let uv_height = (height + 1) / 2;
        let uv_size = (uv_width * uv_height) as usize;
        OwnedVideoFrame {
            track_id: "t".to_owned(),
            width,
            height,
            timestamp_us: 0,
            y: vec![128; y_size],
            u: vec![128; uv_size],
            v: vec![128; uv_size],
            stride_y: width,
            stride_u: uv_width,
            stride_v: uv_width,
        }
    }

    #[test]
    fn to_render_image_preserves_frame_size() {
        let frame = gray_i420(2, 2);
        let image = to_render_image(&frame).expect("変換に失敗しました");
        let size = image.size(0);
        assert_eq!(size.width.0, 2);
        assert_eq!(size.height.0, 2);
        assert_eq!(
            image.as_bytes(0).expect("画素データがありません").len(),
            2 * 2 * 4
        );
    }

    #[test]
    fn to_render_image_rejects_empty_frame() {
        let frame = gray_i420(0, 10);
        assert!(to_render_image(&frame).is_none());
    }
}
