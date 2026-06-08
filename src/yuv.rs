use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use crate::{Error, MediaFrame, Message, ProcessorHandle, Result, TrackId, video::VideoFormat};

#[derive(Debug)]
pub struct YuvWriter {
    file: File,
}

impl YuvWriter {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .map_err(|e| Error::new(format!("{e}: {}", path.as_ref().display())))?;
        Ok(Self { file })
    }

    pub async fn run(mut self, handle: ProcessorHandle, input_track_id: TrackId) -> Result<()> {
        let mut input_rx = handle.subscribe_track(input_track_id.clone());
        handle.notify_ready();

        loop {
            match input_rx.recv().await {
                Message::Media(MediaFrame::Video(frame)) => {
                    if frame.format != VideoFormat::I420 {
                        return Err(Error::new(format!(
                            "expected I420 video sample on track {}, but got {}",
                            input_track_id.get(),
                            frame.format
                        )));
                    }
                    self.file.write_all(&frame.data)?;
                }
                Message::Media(MediaFrame::Audio(_)) => {
                    return Err(Error::new(format!(
                        "expected a video sample on track {}, but got an audio sample",
                        input_track_id.get()
                    )));
                }
                Message::Eos => break,
                Message::Syn(_) => {}
            }
        }

        Ok(())
    }
}

// I420 (YUV 4:2:0 8-bit) の生データファイルを 1 フレームずつ読み込むリーダー
//
// `YuvWriter` が書き出した連続バッファを、指定された解像度に基づいてフレーム単位に
// 区切りながら読み込む。VMAF 評価でフレームごとに参照・劣化画像を取り出すために使う。
#[derive(Debug)]
pub struct YuvReader {
    file: File,
    y_size: usize,
    chroma_size: usize,
}

// I420 の 1 フレーム分のデータと、その Y / U / V プレーンへの分割情報
#[derive(Debug)]
pub struct YuvFrame {
    data: Vec<u8>,
    y_size: usize,
    chroma_size: usize,
}

impl YuvFrame {
    pub fn y(&self) -> &[u8] {
        &self.data[..self.y_size]
    }

    pub fn u(&self) -> &[u8] {
        &self.data[self.y_size..self.y_size + self.chroma_size]
    }

    pub fn v(&self) -> &[u8] {
        &self.data[self.y_size + self.chroma_size..]
    }
}

impl YuvReader {
    pub fn new<P: AsRef<Path>>(path: P, width: usize, height: usize) -> Result<Self> {
        // I420 の各プレーンサイズ。輝度は width * height、色差は水平・垂直ともに半分。
        // 本パイプラインでは解像度は常に偶数だが、念のため切り上げで計算する
        let y_size = width * height;
        let chroma_size = width.div_ceil(2) * height.div_ceil(2);
        let file = File::open(&path)
            .map_err(|e| Error::new(format!("{e}: {}", path.as_ref().display())))?;
        Ok(Self {
            file,
            y_size,
            chroma_size,
        })
    }

    fn frame_size(&self) -> usize {
        self.y_size + self.chroma_size * 2
    }

    // 次の 1 フレームを読み込む。ファイル終端に達していれば `None` を返す
    //
    // フレーム境界の途中でファイルが終わっている場合はエラーとする。
    pub fn read_frame(&mut self) -> Result<Option<YuvFrame>> {
        let frame_size = self.frame_size();
        // フレームサイズは解像度から決まる固定値であり、入力データ由来のサイズ値ではないため
        // 事前確保しても破損データによるメモリ暴走のリスクはない
        let mut data = vec![0u8; frame_size];
        let mut filled = 0;
        while filled < frame_size {
            let read_size = self.file.read(&mut data[filled..])?;
            if read_size == 0 {
                break;
            }
            filled += read_size;
        }
        if filled == 0 {
            return Ok(None);
        }
        if filled != frame_size {
            return Err(Error::new(format!(
                "YUV file size is not a multiple of the frame size {frame_size} (trailing {filled} bytes)"
            )));
        }
        Ok(Some(YuvFrame {
            data,
            y_size: self.y_size,
            chroma_size: self.chroma_size,
        }))
    }
}
