//! [Hisui] 用の [Opus] エンコーダーとデコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [Opus]: https://github.com/xiph/opus
#![warn(missing_docs)]

use std::ffi::{c_int, CStr};

mod sys;

/// エラー
#[derive(Debug)]
pub struct Error {
    code: c_int,
    function: &'static str,
}

impl Error {
    fn check(code: c_int, function: &'static str) -> Result<(), Self> {
        if code >= 0 {
            Ok(())
        } else {
            Err(Self { code, function })
        }
    }

    fn reason(&self) -> Option<&CStr> {
        let reason = unsafe { sys::opus_strerror(self.code) };
        if reason.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(reason) })
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(reason) = self.reason() {
            write!(
                f,
                "{}() failed: code={}, reason={}",
                self.function,
                self.code,
                reason.to_string_lossy()
            )
        } else {
            write!(f, "{}() failed: code={}", self.function, self.code)
        }
    }
}

impl std::error::Error for Error {}

/// Opus デコーダー
#[derive(Debug)]
pub struct Decoder {
    #[cfg(not(feature = "docs-rs"))]
    inner: *mut sys::OpusDecoder,
    channels: u8,
    decode_buf: Vec<i16>,
}

impl Decoder {
    /// デコーダーインスタンスを生成する
    pub fn new(sample_rate: u16, channels: u8) -> Result<Self, Error> {
        let mut error: c_int = 0;
        let inner =
            unsafe { sys::opus_decoder_create(sample_rate as i32, channels as c_int, &mut error) };
        Error::check(error, "opus_decoder_create")?;
        Ok(Self {
            inner,
            channels,
            decode_buf: Vec::new(),
        })
    }

    /// 圧縮音声データをデコードする
    pub fn decode(&mut self, data: &[u8]) -> Result<&[i16], Error> {
        // デコード後のサンプル群を保持できるだけのバッファを確保する
        let size = self.get_nb_samples(data)? * self.channels as usize;
        self.decode_buf.resize(size, 0);

        // デコードする
        unsafe {
            let code = sys::opus_decode(
                self.inner,
                data.as_ptr(),
                data.len() as c_int,
                self.decode_buf.as_mut_ptr(),
                (self.decode_buf.len() / self.channels as usize) as c_int,
                0, // fec
            );
            Error::check(code, "opus_decode")?;
        }

        Ok(&self.decode_buf)
    }

    fn get_nb_samples(&self, packet: &[u8]) -> Result<usize, Error> {
        unsafe {
            let samples = sys::opus_decoder_get_nb_samples(
                self.inner,
                packet.as_ptr(),
                packet.len() as c_int,
            );
            Error::check(samples, "opus_decoder_get_nb_samples")?;
            Ok(samples as usize)
        }
    }
}

unsafe impl Send for Decoder {}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            sys::opus_decoder_destroy(self.inner);
        }
    }
}

/// Opus エンコーダー
#[derive(Debug)]
pub struct Encoder {
    #[cfg(not(feature = "docs-rs"))]
    inner: *mut sys::OpusEncoder,
    channels: u8,
    encode_buf: Vec<u8>,
}

impl Encoder {
    /// エンコーダーインスタンスを生成する
    pub fn new(sample_rate: u16, channels: u8, bitrate: u32) -> Result<Self, Error> {
        let mut error = 0;
        let inner = unsafe {
            sys::opus_encoder_create(
                sample_rate as i32,
                channels as c_int,
                sys::OPUS_APPLICATION_AUDIO as i32,
                &mut error,
            )
        };
        Error::check(error, "opus_encoder_create")?;

        // エンコードビットレートを指定する
        unsafe {
            let code = sys::opus_encoder_ctl(
                inner,
                sys::OPUS_SET_BITRATE_REQUEST as i32,
                bitrate as c_int,
            );
            Error::check(code, "opus_encoder_ctl").inspect_err(|_| {
                sys::opus_encoder_destroy(inner);
            })?;
        }

        Ok(Self {
            inner,
            channels,
            encode_buf: Vec::new(),
        })
    }

    /// MP4 のサンプルエントリーで設定する preSkip の値を取得する
    pub fn get_lookahead(&self) -> Result<u16, Error> {
        let mut value = 0;
        unsafe {
            let code = sys::opus_encoder_ctl(
                self.inner,
                sys::OPUS_GET_LOOKAHEAD_REQUEST as i32,
                &mut value,
            );
            Error::check(code, "opus_encoder_ctl")?;
        }
        Ok(value as u16)
    }

    /// PCM データをエンコードする
    pub fn encode(&mut self, pcm: &[i16]) -> Result<&[u8], Error> {
        // エンコードによって PCM よりも大きなサイズになることはないと仮定して
        // エンコード後のデータを格納するためのバッファを割り当てておく。
        // ただし PCM が極端に小さい場合にはエンコード後の方が大きくなる可能性が
        // あるかもしれないので 1024 を最低にしておく。
        let buf_size = 1024.max(pcm.len() * self.channels as usize * 2);
        self.encode_buf.resize(buf_size, 0);

        unsafe {
            let size = sys::opus_encode(
                self.inner,
                pcm.as_ptr(),
                pcm.len() as c_int / self.channels as c_int,
                self.encode_buf.as_mut_ptr(),
                self.encode_buf.len() as c_int,
            );
            Error::check(size, "opus_encode")?;
            Ok(&self.encode_buf[..size as usize])
        }
    }
}

unsafe impl Send for Encoder {}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            sys::opus_encoder_destroy(self.inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_decoder() {
        // OK
        assert!(Decoder::new(48000, 2).is_ok());
        assert!(Decoder::new(48000, 1).is_ok());

        // NG
        assert!(Decoder::new(48000, 20).is_err());
        assert!(Decoder::new(48000, 0).is_err());
        assert!(Decoder::new(0, 2).is_err());
    }

    #[test]
    fn init_encoder() {
        // OK
        assert!(Encoder::new(48000, 2, 64_000).is_ok());
        assert!(Encoder::new(48000, 1, 64_000).is_ok());

        // NG
        assert!(Encoder::new(48000, 20, 64_000).is_err());
        assert!(Encoder::new(48000, 1, 0).is_err());
        assert!(Encoder::new(0, 1, 64_000).is_err());
    }

    #[test]
    fn decode_silent() {
        let mut decoder = Decoder::new(48000, 1).expect("failed to create decoder");
        let decoded = decoder
            .decode(&[0b1111_1000, 0xFF, 0xFE])
            .expect("failed to decode");
        assert_eq!(decoded.len(), 960);
        assert!(decoded.iter().all(|v| *v == 0));
    }

    #[test]
    fn encode_silent() {
        let mut encoder = Encoder::new(48000, 1, 64_000).expect("failed to create encoder");
        let encoded = encoder.encode(&[0; 960]).expect("failed to encode");
        assert_eq!(encoded, &[0b1111_1000, 0xFF, 0xFE]);
    }

    #[test]
    fn error_reason() {
        let e = Error::check(sys::OPUS_BAD_ARG, "test").expect_err("not an error");
        assert!(e.reason().is_some());
    }
}
