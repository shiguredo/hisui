use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::Duration,
};

use orfail::OrFail;

use crate::{
    layout::Layout,
    video::{VideoFrame, VideoFrameReceiver},
};

/// 合成結果を含んだ YUV ファイルを書き出すための構造体
#[derive(Debug)]
pub struct YuvWriter {
    file: BufWriter<File>,
    input_video_rx: VideoFrameReceiver,
}

impl YuvWriter {
    /// [`YuvWriter`] インスタンスを生成する
    pub fn new<P: AsRef<Path>>(
        path: P,
        layout: &Layout,
        input_video_rx: VideoFrameReceiver,
    ) -> orfail::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .or_fail()?;

        Ok(Self {
            file: BufWriter::new(file),
            input_video_rx,
        })
    }

    /// 新しい入力（合成後の映像）を待機して、それの出力ファイルへの書き込みを行う
    ///
    /// 結果は現在の書き込み位置を示すタイムスタンプで、全ての書き込みが完了した場合には `Ok(None)` が返される。
    pub fn poll(&mut self) -> orfail::Result<Option<Duration>> {
        if let Some(frame) = self.input_video_rx.peek() {
            let timestamp = frame.timestamp;
            self.append_video_frame().or_fail()?;
            Ok(Some(timestamp))
        } else {
            // 全ての入力の処理が完了した
            self.finalize().or_fail()?;
            Ok(None)
        }
    }

    fn append_video_frame(&mut self) -> orfail::Result<()> {
        // 次の入力を取り出す（これは常に成功する）
        let frame = self.input_video_rx.recv().or_fail()?;

        // YUV データを出力ファイルに書き込む
        self.file.write_all(&frame.data).or_fail()?;

        Ok(())
    }

    fn finalize(&mut self) -> orfail::Result<()> {
        self.file.flush().or_fail()?;
        Ok(())
    }
}
