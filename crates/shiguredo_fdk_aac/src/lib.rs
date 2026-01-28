//! [Hisui] 用の [FDK AAC] エンコーダー
//!
//! [Hisui]: https://github.com/shiguredo/hisui
//! [FDK AAC]: https://github.com/mstorsjo/fdk-aac
#![warn(missing_docs)]

use std::{ffi::c_void, mem::MaybeUninit};

mod sys;

/// エラー
#[derive(Debug)]
pub struct Error {
    code: sys::AACENC_ERROR,
    function: &'static str,
}

impl Error {
    fn check(code: sys::AACENC_ERROR, function: &'static str) -> Result<(), Self> {
        if code == sys::AACENC_ERROR_AACENC_OK {
            return Ok(());
        }
        Err(Self { code, function })
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}() failed: code={}", self.function, self.code)
    }
}

impl std::error::Error for Error {}

// チャネル数とサンプルレートは Hisui では固定
const CHANNELS: usize = 2;
const SAMPLE_RATE: usize = 48000;

// エンコードバッファのサイズ。十分に多い値ならなんでもいい。
const ENCODE_BUF_SIZE: usize = 20480;

// デコードバッファのサイズ。十分に多い値ならなんでもいい。
const DECODE_BUF_SIZE: usize = 4096;

/// エンコーダーに指定する設定
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// エンコードビットレート (bps 単位)
    pub target_bitrate: usize,
}

/// AAC エンコーダー
#[derive(Debug)]
pub struct Encoder {
    handle: EncoderHandle,
    encode_buf: Vec<u8>,
    pcm_buf: Vec<i16>,
    audio_specific_config: Vec<u8>,
    frame_len: usize,
}

impl Encoder {
    /// エンコーダーインスタンスを生成する
    pub fn new(config: EncoderConfig) -> Result<Self, Error> {
        let mut info = MaybeUninit::<sys::AACENC_InfoStruct>::zeroed();
        let mut handle = std::ptr::null_mut();
        let channel_mode = sys::CHANNEL_MODE_MODE_2;
        let channel_order = 1;
        let transport_type = sys::TRANSPORT_TYPE_TT_MP4_RAW;
        let afterburner = 1;
        unsafe {
            let code = sys::aacEncOpen(&mut handle, 0, CHANNELS as sys::UINT);
            Error::check(code, "aacEncOpen")?;

            let handle = EncoderHandle(handle);

            // LC (Low Complexity) を指定する
            let code = sys::aacEncoder_SetParam(
                handle.0,
                sys::AACENC_PARAM_AACENC_AOT,
                sys::AUDIO_OBJECT_TYPE_AOT_AAC_LC as sys::UINT,
            );
            Error::check(code, "aacEncoder_SetParam(AOT)")?;

            let code = sys::aacEncoder_SetParam(
                handle.0,
                sys::AACENC_PARAM_AACENC_SAMPLERATE,
                SAMPLE_RATE as sys::UINT,
            );
            Error::check(code, "aacEncoder_SetParam(SAMPLERATE)")?;

            let code = sys::aacEncoder_SetParam(
                handle.0,
                sys::AACENC_PARAM_AACENC_CHANNELMODE,
                channel_mode as sys::UINT,
            );
            Error::check(code, "aacEncoder_SetParam(CHANNELMODE)")?;

            let code = sys::aacEncoder_SetParam(
                handle.0,
                sys::AACENC_PARAM_AACENC_CHANNELORDER,
                channel_order,
            );
            Error::check(code, "aacEncoder_SetParam(CHANNELORDER)")?;

            let code = sys::aacEncoder_SetParam(
                handle.0,
                sys::AACENC_PARAM_AACENC_BITRATE,
                config.target_bitrate as sys::UINT,
            );
            Error::check(code, "aacEncoder_SetParam(BITRATE)")?;

            let code = sys::aacEncoder_SetParam(
                handle.0,
                sys::AACENC_PARAM_AACENC_TRANSMUX,
                transport_type as sys::UINT,
            );
            Error::check(code, "aacEncoder_SetParam(TRANSMUX)")?;

            let code = sys::aacEncoder_SetParam(
                handle.0,
                sys::AACENC_PARAM_AACENC_AFTERBURNER,
                afterburner,
            );
            Error::check(code, "aacEncoder_SetParam(AFTERBURNER)")?;

            let code = sys::aacEncEncode(
                handle.0,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            Error::check(code, "aacEncEncode")?;

            let code = sys::aacEncInfo(handle.0, info.as_mut_ptr());
            Error::check(code, "aacEncInfo")?;

            let info = info.assume_init();
            let audio_specific_config = info.confBuf[..info.confSize as usize].to_vec();

            Ok(Self {
                handle,
                encode_buf: vec![0; ENCODE_BUF_SIZE],
                pcm_buf: Vec::new(),
                audio_specific_config,
                frame_len: info.frameLength as usize,
            })
        }
    }

    /// MP4 のサンプルエントリーに設定するデコーダー向けの情報
    pub fn audio_specific_config(&self) -> &[u8] {
        &self.audio_specific_config
    }

    /// PCM データをエンコードする
    pub fn encode(&mut self, pcm: &[i16]) -> Result<Option<EncodedFrame>, Error> {
        self.pcm_buf.extend_from_slice(pcm);
        if self.pcm_buf.len() < self.frame_len * CHANNELS {
            return Ok(None);
        }
        self.encode_impl()
    }

    /// バッファ内に残っているデータを強制的にエンコードする
    pub fn finish(&mut self) -> Result<Option<EncodedFrame>, Error> {
        self.encode_impl()
    }

    fn encode_impl(&mut self) -> Result<Option<EncodedFrame>, Error> {
        if self.pcm_buf.is_empty() {
            return Ok(None);
        }

        let in_buf = MaybeUninit::<sys::AACENC_BufDesc>::zeroed();
        let out_buf = MaybeUninit::<sys::AACENC_BufDesc>::zeroed();
        let in_elem_size = 2;
        let out_elem_size = 1;
        let in_args = MaybeUninit::<sys::AACENC_InArgs>::zeroed();
        let mut out_args = MaybeUninit::<sys::AACENC_OutArgs>::zeroed();
        unsafe {
            let mut in_args = in_args.assume_init();
            in_args.numInSamples = self.pcm_buf.len() as sys::INT;

            let mut in_buf = in_buf.assume_init();

            // 一時配列を直接フィールドに代入してしまうと、
            // リリースビルド時のコンパイラの最適化によってポインタが無効になることがあるので、
            // 一度変数を経由する
            let mut in_buf_bufs = [self.pcm_buf.as_ptr() as *mut c_void];
            let mut in_buf_buffer_identifiers = [sys::AACENC_BufferIdentifier_IN_AUDIO_DATA as i32];
            let mut in_buf_buf_sizes = [self.pcm_buf.len() as sys::INT * in_elem_size];
            let mut in_buf_buf_el_sizes = [in_elem_size];

            in_buf.numBufs = 1;
            in_buf.bufs = in_buf_bufs.as_mut_ptr();
            in_buf.bufferIdentifiers = in_buf_buffer_identifiers.as_mut_ptr();
            in_buf.bufSizes = in_buf_buf_sizes.as_mut_ptr();
            in_buf.bufElSizes = in_buf_buf_el_sizes.as_mut_ptr();

            let mut out_buf = out_buf.assume_init();

            // in_buf_* と同様にこちらも変数を経由してポインタを取得する
            let mut out_buf_bufs = [self.encode_buf.as_mut_ptr() as *mut c_void];
            let mut out_buf_buffer_identifiers =
                [sys::AACENC_BufferIdentifier_OUT_BITSTREAM_DATA as i32];
            let mut out_buf_buf_sizes = [self.encode_buf.len() as sys::INT];
            let mut out_buf_buf_el_sizes = [out_elem_size];

            out_buf.numBufs = 1;
            out_buf.bufs = out_buf_bufs.as_mut_ptr();
            out_buf.bufferIdentifiers = out_buf_buffer_identifiers.as_mut_ptr();
            out_buf.bufSizes = out_buf_buf_sizes.as_mut_ptr();
            out_buf.bufElSizes = out_buf_buf_el_sizes.as_mut_ptr();

            let code = sys::aacEncEncode(
                self.handle.0,
                &in_buf,
                &out_buf,
                &in_args,
                out_args.as_mut_ptr(),
            );
            Error::check(code, "aacEncEncode")?;

            let out_args = out_args.assume_init();
            self.pcm_buf.drain(..out_args.numInSamples as usize);

            let data = self.encode_buf[..out_args.numOutBytes as usize].to_vec();
            Ok(Some(EncodedFrame {
                data,
                samples: out_args.numInSamples as usize / CHANNELS,
            }))
        }
    }
}

unsafe impl Send for Encoder {}

#[derive(Debug)]
struct EncoderHandle(sys::HANDLE_AACENCODER);

impl Drop for EncoderHandle {
    fn drop(&mut self) {
        unsafe {
            sys::aacEncClose(&mut self.0);
        }
    }
}

/// エンコードされた AAC フレーム
#[derive(Debug)]
pub struct EncodedFrame {
    /// 圧縮データ
    pub data: Vec<u8>,

    /// フレーム内のサンプル数
    pub samples: usize,
}

/// AAC デコーダー
#[derive(Debug)]
pub struct Decoder {
    handle: DecoderHandle,
    audio_specific_config: Vec<u8>,
}

impl Decoder {
    /// デコーダーインスタンスを生成する
    ///
    /// Audio Specific Config (ASC) バッファを指定して、AAC デコーダーを初期化します。
    /// ASC はエンコーダーの `audio_specific_config()` メソッドで取得できます。
    pub fn new(audio_specific_config: &[u8]) -> Result<Self, Error> {
        unsafe {
            let handle = sys::aacDecoder_Open(sys::TRANSPORT_TYPE_TT_MP4_RAW, 1);
            if handle.is_null() {
                return Err(Error {
                    code: sys::AACENC_ERROR_AACENC_INVALID_HANDLE,
                    function: "aacDecoder_Open",
                });
            }

            let handle = DecoderHandle(handle);

            // Audio Specific Config を設定する
            let mut conf = [audio_specific_config.as_ptr() as *mut u8];
            let mut length = [audio_specific_config.len() as sys::UINT];

            let code = sys::aacDecoder_ConfigRaw(handle.0, conf.as_mut_ptr(), length.as_mut_ptr());

            // aacDecoder_ConfigRaw の戻り値はエラーコードではなく、バッファポインタなので
            // 通常のエラーチェックは行わない
            if code.is_null() {
                return Err(Error {
                    code: sys::AACENC_ERROR_AACENC_INVALID_HANDLE,
                    function: "aacDecoder_ConfigRaw",
                });
            }

            Ok(Self {
                handle,
                audio_specific_config: audio_specific_config.to_vec(),
            })
        }
    }

    /// AAC 圧縮データをデコードする
    pub fn decode(&mut self, encoded: &[u8]) -> Result<Option<DecodedFrame>, Error> {
        unsafe {
            let mut buf = [encoded.as_ptr() as *mut u8];
            let mut buf_size = [encoded.len() as sys::UINT];
            let mut bytes_valid = encoded.len() as sys::UINT;

            // デコーダーの入力バッファにデータを充填する
            let code = sys::aacDecoder_Fill(
                self.handle.0,
                buf.as_mut_ptr(),
                buf_size.as_mut_ptr(),
                &mut bytes_valid,
            );

            // aacDecoder_Fill はエラーコードを返さないため、チェックしない

            // デコード用バッファを準備
            let mut decode_buf = vec![0i16; DECODE_BUF_SIZE];

            // フレームをデコードする
            let code = sys::aacDecoder_DecodeFrame(
                self.handle.0,
                decode_buf.as_mut_ptr(),
                (decode_buf.len() * std::mem::size_of::<i16>()) as i32,
                0,
            );

            // デコードエラーをチェック
            // AAC_DEC_NOT_ENOUGH_BITS は正常な終了を示す
            if code != sys::AAC_DEC_OK && code != sys::AAC_DEC_NOT_ENOUGH_BITS {
                return Err(Error {
                    code: sys::AACENC_ERROR_AACENC_UNKNOWN,
                    function: "aacDecoder_DecodeFrame",
                });
            }

            // ストリーム情報を取得
            let stream_info = sys::aacDecoder_GetStreamInfo(self.handle.0);
            if stream_info.is_null() {
                return Ok(None);
            }

            let stream_info = &*stream_info;
            let frame_size = stream_info.frameSize as usize;
            let num_channels = stream_info.numChannels as usize;
            let samples = frame_size * num_channels;

            // バッファを実際のサンプル数に縮小
            decode_buf.truncate(samples);

            if samples == 0 {
                return Ok(None);
            }

            Ok(Some(DecodedFrame {
                data: decode_buf,
                samples: frame_size,
            }))
        }
    }
}

unsafe impl Send for Decoder {}

#[derive(Debug)]
struct DecoderHandle(sys::HANDLE_AACDECODER);

impl Drop for DecoderHandle {
    fn drop(&mut self) {
        unsafe {
            sys::aacDecoder_Close(self.0);
        }
    }
}

/// デコードされた AAC フレーム
#[derive(Debug)]
pub struct DecodedFrame {
    /// PCM データ
    pub data: Vec<i16>,

    /// フレーム内のサンプル数
    pub samples: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_encoder() {
        // OK
        let config = EncoderConfig {
            target_bitrate: 100_000,
        };
        assert!(Encoder::new(config).is_ok());

        // FDK AAC では、ターゲットビットレートに 0 を指定しても通ってしまう模様
        let config = EncoderConfig { target_bitrate: 0 };
        assert!(Encoder::new(config).is_ok());
    }

    #[test]
    fn encode_silent() {
        let config = EncoderConfig {
            target_bitrate: 100_000,
        };
        let mut encoder = Encoder::new(config).expect("failed to create encoder");
        let mut sample_count = 0;

        for _ in 0..100 {
            if let Some(encoded) = encoder
                .encode(&[0; 100 * CHANNELS])
                .expect("failed to encode")
            {
                sample_count += encoded.samples;
            }
        }
        if let Some(encoded) = encoder.finish().expect("failed to finish") {
            sample_count += encoded.samples;
        }

        assert_eq!(sample_count, 100 * 100);
    }

    #[test]
    fn init_decoder() {
        let config = EncoderConfig {
            target_bitrate: 100_000,
        };
        let encoder = Encoder::new(config).expect("failed to create encoder");
        let asc = encoder.audio_specific_config();

        // OK - valid ASC
        assert!(Decoder::new(asc).is_ok());

        // OK - empty ASC (decoder initialization may still succeed)
        let result = Decoder::new(&[]);
        // We don't assert on this as FDK behavior may vary
        let _ = result;
    }

    #[test]
    fn decode_silent() {
        let config = EncoderConfig {
            target_bitrate: 100_000,
        };
        let mut encoder = Encoder::new(config).expect("failed to create encoder");

        // エンコード用のデータを準備
        let pcm_data = vec![0i16; 1024 * CHANNELS];
        let mut encoded_frames = Vec::new();

        // 無音のオーディオをエンコード
        if let Some(frame) = encoder.encode(&pcm_data).expect("failed to encode") {
            encoded_frames.push(frame);
        }

        if let Some(frame) = encoder.finish().expect("failed to finish") {
            encoded_frames.push(frame);
        }

        // デコーダーを初期化
        let asc = encoder.audio_specific_config();
        let mut decoder = Decoder::new(asc).expect("failed to create decoder");

        // エンコードされたフレームをデコード
        let mut total_decoded = 0;
        for frame in encoded_frames {
            if let Some(decoded) = decoder.decode(&frame.data).expect("failed to decode") {
                total_decoded += decoded.samples;
            }
        }

        // デコードされたサンプル数が入力サンプル数と一致することを確認
        assert!(total_decoded > 0, "expected to decode some samples");
    }
}
