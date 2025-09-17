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
    fn new(code: i32, function: &'static str, reason: Option<&'static str>) -> Self {
        Self {
            code,
            function,
            reason,
        }
    }

    fn with_reason(code: i32, function: &'static str, reason: &'static str) -> Self {
        Self::new(code, function, Some(reason))
    }

    fn check(code: i32, function: &'static str) -> Result<(), Self> {
        if code == 0 {
            Ok(())
        } else {
            Err(Self::new(code, function, None))
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
            FilterMode::None => sys::FilterMode_kFilterNone,
            FilterMode::Linear => sys::FilterMode_kFilterLinear,
            FilterMode::Bilinear => sys::FilterMode_kFilterBilinear,
            FilterMode::Box => sys::FilterMode_kFilterBox,
        }
    }
}

/// I420 画像の各プレーン情報
#[derive(Debug)]
pub struct I420Planes<'a> {
    /// Y プレーンデータ
    pub y: &'a [u8],
    /// Y プレーンのストライド（行あたりのバイト数）
    pub y_stride: usize,
    /// U プレーンデータ
    pub u: &'a [u8],
    /// U プレーンのストライド
    pub u_stride: usize,
    /// V プレーンデータ
    pub v: &'a [u8],
    /// V プレーンのストライド
    pub v_stride: usize,
}

/// I420 画像の各プレーン情報（可変）
#[derive(Debug)]
pub struct I420PlanesMut<'a> {
    /// Y プレーンデータ
    pub y: &'a mut [u8],
    /// Y プレーンのストライド（行あたりのバイト数）
    pub y_stride: usize,
    /// U プレーンデータ
    pub u: &'a mut [u8],
    /// U プレーンのストライド
    pub u_stride: usize,
    /// V プレーンデータ
    pub v: &'a mut [u8],
    /// V プレーンのストライド
    pub v_stride: usize,
}

/// 画像の幅と高さ
#[derive(Debug, Clone, Copy)]
pub struct ImageSize {
    /// 画像の幅
    pub width: usize,
    /// 画像の高さ
    pub height: usize,
}

impl ImageSize {
    /// 新しい画像サイズを作成
    pub const fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }
}

/// RGB24 画像情報
#[derive(Debug)]
pub struct Rgb24Image<'a> {
    /// RGB24 データ (R, G, B の順)
    pub data: &'a [u8],
    /// RGB24 のストライド
    pub stride: usize,
}

/// RGB24 画像情報（可変）
#[derive(Debug)]
pub struct Rgb24ImageMut<'a> {
    /// RGB24 データ (R, G, B の順)
    pub data: &'a mut [u8],
    /// RGB24 のストライド
    pub stride: usize,
}

/// I420 形式の YUV データをリサイズする
pub fn i420_scale(
    src: &I420Planes<'_>,
    src_size: ImageSize,
    dst: &mut I420PlanesMut<'_>,
    dst_size: ImageSize,
    filtering: FilterMode,
) -> Result<(), Error> {
    // バッファサイズの検証
    let src_y_size = src.y_stride * src_size.height;
    let src_u_size = src.u_stride * src_size.height.div_ceil(2);
    let src_v_size = src.v_stride * src_size.height.div_ceil(2);
    let dst_y_size = dst.y_stride * dst_size.height;
    let dst_u_size = dst.u_stride * dst_size.height.div_ceil(2);
    let dst_v_size = dst.v_stride * dst_size.height.div_ceil(2);

    if src.y.len() < src_y_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "source Y buffer too small",
        ));
    }
    if src.u.len() < src_u_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "source U buffer too small",
        ));
    }
    if src.v.len() < src_v_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "source V buffer too small",
        ));
    }
    if dst.y.len() < dst_y_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "destination Y buffer too small",
        ));
    }
    if dst.u.len() < dst_u_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "destination U buffer too small",
        ));
    }
    if dst.v.len() < dst_v_size {
        return Err(Error::with_reason(
            -1,
            "I420Scale",
            "destination V buffer too small",
        ));
    }

    let result = unsafe {
        sys::I420Scale(
            src.y.as_ptr(),
            src.y_stride as c_int,
            src.u.as_ptr(),
            src.u_stride as c_int,
            src.v.as_ptr(),
            src.v_stride as c_int,
            src_size.width as c_int,
            src_size.height as c_int,
            dst.y.as_mut_ptr(),
            dst.y_stride as c_int,
            dst.u.as_mut_ptr(),
            dst.u_stride as c_int,
            dst.v.as_mut_ptr(),
            dst.v_stride as c_int,
            dst_size.width as c_int,
            dst_size.height as c_int,
            filtering.to_libyuv_filter_mode(),
        )
    };

    Error::check(result, "I420Scale")
}

/// RGB24 から I420 への変換
pub fn rgb24_to_i420(
    src: &Rgb24Image<'_>,
    dst: &mut I420PlanesMut<'_>,
    size: ImageSize,
) -> Result<(), Error> {
    let result = unsafe {
        sys::RGB24ToI420(
            src.data.as_ptr(),
            src.stride as c_int,
            dst.y.as_mut_ptr(),
            dst.y_stride as c_int,
            dst.u.as_mut_ptr(),
            dst.u_stride as c_int,
            dst.v.as_mut_ptr(),
            dst.v_stride as c_int,
            size.width as c_int,
            size.height as c_int,
        )
    };

    Error::check(result, "RGB24ToI420")
}

/// I420 から RGB24 への変換
pub fn i420_to_rgb24(
    src: &I420Planes<'_>,
    dst: &mut Rgb24ImageMut<'_>,
    size: ImageSize,
) -> Result<(), Error> {
    let result = unsafe {
        sys::I420ToRGB24(
            src.y.as_ptr(),
            src.y_stride as c_int,
            src.u.as_ptr(),
            src.u_stride as c_int,
            src.v.as_ptr(),
            src.v_stride as c_int,
            dst.data.as_mut_ptr(),
            dst.stride as c_int,
            size.width as c_int,
            size.height as c_int,
        )
    };

    Error::check(result, "I420ToRGB24")
}

/// プレーンのコピー
pub fn copy_plane(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    dst_stride: usize,
    size: ImageSize,
) -> Result<(), Error> {
    // バッファサイズの検証
    let src_size = src_stride * size.height;
    let dst_size = dst_stride * size.height;

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
            size.width as c_int,
            size.height as c_int,
        )
    };

    Ok(())
}

/// プレーンを指定値で塗りつぶし
pub fn set_plane(
    dst: &mut [u8],
    dst_stride: usize,
    size: ImageSize,
    value: u8,
) -> Result<(), Error> {
    let dst_size = dst_stride * size.height;

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
            size.width as c_int,
            size.height as c_int,
            value as u32,
        )
    };

    Ok(())
}
