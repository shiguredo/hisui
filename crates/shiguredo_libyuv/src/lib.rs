//! [Hisui] 用の [libyuv] 画像変換・処理ライブラリ
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [libyuv]: https://chromium.googlesource.com/libyuv/libyuv/
#![warn(missing_docs)]

use std::ffi::{c_int, c_uint};

mod sys;

/// ビルド時に参照したリポジトリ URL
pub const BUILD_REPOSITORY: &str = sys::BUILD_METADATA_REPOSITORY;

/// ビルド時に参照したリポジトリのバージョン（タグ）
pub const BUILD_VERSION: &str = sys::BUILD_METADATA_VERSION;

/// エラー
#[derive(Debug)]
pub struct Error {
    code: i32,
    function: &'static str,
    reason: Option<&'static str>,
}

impl Error {
    fn check(code: i32, function: &'static str) -> Result<(), Self> {
        if code == 0 {
            Ok(())
        } else {
            Err(Self {
                code,
                function,
                reason: None,
            })
        }
    }

    fn with_reason(code: i32, function: &'static str, reason: &'static str) -> Self {
        Self {
            code,
            function,
            reason: Some(reason),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(reason) = self.reason {
            write!(
                f,
                "{}() failed: code={}, reason={reason}",
                self.function, self.code
            )
        } else {
            write!(f, "{}() failed: code={}", self.function, self.code)
        }
    }
}

impl std::error::Error for Error {}

/// スケール品質フィルタ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    /// なし（最も高速だが品質は最低）
    None,
    /// 線形フィルタ（高速で適度な品質）
    Linear,
    /// バイリニア（中程度の速度と品質）
    Bilinear,
    /// ボックスフィルタ（中程度の速度、ダウンスケール時に有効）
    Box,
}

impl FilterMode {
    fn to_libyuv_filter_mode(self) -> c_uint {
        match self {
            FilterMode::None => 0,     // kFilterNone
            FilterMode::Linear => 1,   // kFilterLinear
            FilterMode::Bilinear => 2, // kFilterBilinear
            FilterMode::Box => 3,      // kFilterBox
        }
    }
}

/// I420 形式の YUV データをリサイズする
///
/// # 引数
/// * `src_y` - ソース Y プレーンデータ
/// * `src_stride_y` - ソース Y プレーンのストライド（行あたりのバイト数）
/// * `src_u` - ソース U プレーンデータ
/// * `src_stride_u` - ソース U プレーンのストライド
/// * `src_v` - ソース V プレーンデータ
/// * `src_stride_v` - ソース V プレーンのストライド
/// * `src_width` - ソース画像の幅
/// * `src_height` - ソース画像の高さ
/// * `dst_y` - 出力 Y プレーンバッファ
/// * `dst_stride_y` - 出力 Y プレーンのストライド
/// * `dst_u` - 出力 U プレーンバッファ
/// * `dst_stride_u` - 出力 U プレーンのストライド
/// * `dst_v` - 出力 V プレーンバッファ
/// * `dst_stride_v` - 出力 V プレーンのストライド
/// * `dst_width` - 出力画像の幅
/// * `dst_height` - 出力画像の高さ
/// * `filtering` - フィルタリングモード
pub fn i420_scale(
    src_y: &[u8],
    src_stride_y: usize,
    src_u: &[u8],
    src_stride_u: usize,
    src_v: &[u8],
    src_stride_v: usize,
    src_width: usize,
    src_height: usize,
    dst_y: &mut [u8],
    dst_stride_y: usize,
    dst_u: &mut [u8],
    dst_stride_u: usize,
    dst_v: &mut [u8],
    dst_stride_v: usize,
    dst_width: usize,
    dst_height: usize,
    filtering: FilterMode,
) -> Result<(), Error> {
    // バッファサイズの検証
    let src_y_size = src_stride_y * src_height;
    let src_u_size = src_stride_u * src_height.div_ceil(2);
    let src_v_size = src_stride_v * src_height.div_ceil(2);
    let dst_y_size = dst_stride_y * dst_height;
    let dst_u_size = dst_stride_u * dst_height.div_ceil(2);
    let dst_v_size = dst_stride_v * dst_height.div_ceil(2);

    if src_y.len() < src_y_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "source Y buffer too small",
        ));
    }
    if src_u.len() < src_u_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "source U buffer too small",
        ));
    }
    if src_v.len() < src_v_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "source V buffer too small",
        ));
    }
    if dst_y.len() < dst_y_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "destination Y buffer too small",
        ));
    }
    if dst_u.len() < dst_u_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "destination U buffer too small",
        ));
    }
    if dst_v.len() < dst_v_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "destination V buffer too small",
        ));
    }

    let result = unsafe {
        sys::I420Scale(
            src_y.as_ptr(),
            src_stride_y as c_int,
            src_u.as_ptr(),
            src_stride_u as c_int,
            src_v.as_ptr(),
            src_stride_v as c_int,
            src_width as c_int,
            src_height as c_int,
            dst_y.as_mut_ptr(),
            dst_stride_y as c_int,
            dst_u.as_mut_ptr(),
            dst_stride_u as c_int,
            dst_v.as_mut_ptr(),
            dst_stride_v as c_int,
            dst_width as c_int,
            dst_height as c_int,
            filtering.to_libyuv_filter_mode(),
        )
    };

    Error::check(result, "I420Scale")
}

/// RGB24 から I420 への変換
///
/// # 引数
/// * `src_rgb24` - ソース RGB24 データ (R, G, B の順)
/// * `src_stride_rgb24` - ソース RGB24 のストライド
/// * `dst_y` - 出力 Y プレーンバッファ
/// * `dst_stride_y` - 出力 Y プレーンのストライド
/// * `dst_u` - 出力 U プレーンバッファ
/// * `dst_stride_u` - 出力 U プレーンのストライド
/// * `dst_v` - 出力 V プレーンバッファ
/// * `dst_stride_v` - 出力 V プレーンのストライド
/// * `width` - 画像の幅
/// * `height` - 画像の高さ
pub fn rgb24_to_i420(
    src_rgb24: &[u8],
    src_stride_rgb24: usize,
    dst_y: &mut [u8],
    dst_stride_y: usize,
    dst_u: &mut [u8],
    dst_stride_u: usize,
    dst_v: &mut [u8],
    dst_stride_v: usize,
    width: usize,
    height: usize,
) -> Result<(), Error> {
    let result = unsafe {
        sys::RGB24ToI420(
            src_rgb24.as_ptr(),
            src_stride_rgb24 as c_int,
            dst_y.as_mut_ptr(),
            dst_stride_y as c_int,
            dst_u.as_mut_ptr(),
            dst_stride_u as c_int,
            dst_v.as_mut_ptr(),
            dst_stride_v as c_int,
            width as c_int,
            height as c_int,
        )
    };

    Error::check(result, "RGB24ToI420")
}

/// I420 から RGB24 への変換
///
/// # 引数
/// * `src_y` - ソース Y プレーンデータ
/// * `src_stride_y` - ソース Y プレーンのストライド
/// * `src_u` - ソース U プレーンデータ
/// * `src_stride_u` - ソース U プレーンのストライド
/// * `src_v` - ソース V プレーンデータ
/// * `src_stride_v` - ソース V プレーンのストライド
/// * `dst_rgb24` - 出力 RGB24 バッファ (R, G, B の順)
/// * `dst_stride_rgb24` - 出力 RGB24 のストライド
/// * `width` - 画像の幅
/// * `height` - 画像の高さ
pub fn i420_to_rgb24(
    src_y: &[u8],
    src_stride_y: usize,
    src_u: &[u8],
    src_stride_u: usize,
    src_v: &[u8],
    src_stride_v: usize,
    dst_rgb24: &mut [u8],
    dst_stride_rgb24: usize,
    width: usize,
    height: usize,
) -> Result<(), Error> {
    let result = unsafe {
        sys::I420ToRGB24(
            src_y.as_ptr(),
            src_stride_y as c_int,
            src_u.as_ptr(),
            src_stride_u as c_int,
            src_v.as_ptr(),
            src_stride_v as c_int,
            dst_rgb24.as_mut_ptr(),
            dst_stride_rgb24 as c_int,
            width as c_int,
            height as c_int,
        )
    };

    Error::check(result, "I420ToRGB24")
}

/// プレーンの高速コピー（libyuv の CopyPlane を使用）
pub fn copy_plane(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    width: usize,
    height: usize,
) -> Result<(), Error> {
    // バッファサイズの検証
    let src_size = src_stride * height;
    let dst_size = dst_stride * height;

    if src.len() < src_size {
        return Err(Error::with_reason(
            -1,
            "CopyPlane",
            "source buffer too small",
        ));
    }
    if dst.len() < dst_size {
        return Err(Error::with_reason(
            -1,
            "CopyPlane",
            "destination buffer too small",
        ));
    }

    unsafe {
        sys::CopyPlane(
            src.as_ptr(),
            src_stride as c_int,
            dst.as_mut_ptr(),
            dst_stride as c_int,
            width as c_int,
            height as c_int,
        )
    };

    Ok(())
}

/// プレーンを指定値で塗りつぶし（libyuv の SetPlane を使用）
pub fn set_plane(
    dst: &mut [u8],
    dst_stride: usize,
    width: usize,
    height: usize,
    value: u8,
) -> Result<(), Error> {
    let dst_size = dst_stride * height;

    if dst.len() < dst_size {
        return Err(Error::with_reason(
            -1,
            "SetPlane",
            "destination buffer too small",
        ));
    }

    unsafe {
        sys::SetPlane(
            dst.as_mut_ptr(),
            dst_stride as c_int,
            width as c_int,
            height as c_int,
            value.into(), // Convert u8 to u32
        )
    };

    Ok(())
}
