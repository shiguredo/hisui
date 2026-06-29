use std::collections::VecDeque;

use tokio::sync::mpsc;

use crate::video::{VideoFormat, VideoFrame};

/// Sender 化された dav1d (AV1) デコーダー
///
/// デコード済みフレームは `tx` 経由で上位に送信する。
#[derive(Debug)]
pub struct Dav1dDecoder {
    inner: shiguredo_dav1d::Decoder,
    // デコード済みフレームと元入力フレームを対応付けるための保持列
    input_queue: VecDeque<VideoFrame>,
    // デコード結果を上位 run() の Receiver に流す Sender
    tx: mpsc::Sender<crate::Result<VideoFrame>>,
}

impl Dav1dDecoder {
    pub fn new(tx: mpsc::Sender<crate::Result<VideoFrame>>) -> crate::Result<Self> {
        Ok(Self {
            inner: shiguredo_dav1d::Decoder::new(shiguredo_dav1d::DecoderConfig::default())?,
            input_queue: VecDeque::new(),
            tx,
        })
    }

    pub async fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if frame.format != VideoFormat::Av1 {
            return Err(crate::Error::new(format!(
                "expected AV1 format, got {:?}",
                frame.format
            )));
        }

        self.inner.decode(&frame.data)?;
        self.input_queue.push_back(frame.to_stripped());
        self.send_decoded_frames().await?;
        Ok(())
    }

    pub async fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.send_decoded_frames().await?;
        Ok(())
    }

    /// dav1d の出力フレームを取り出して Sender に送る
    async fn send_decoded_frames(&mut self) -> crate::Result<()> {
        while let Some(decoded) = self.inner.next_frame()? {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("decoded frame produced without input frame"))?;
            let out = VideoFrame::new_i420(
                input_frame,
                decoded.width(),
                decoded.height(),
                decoded.y_plane(),
                decoded.u_plane(),
                decoded.v_plane(),
                decoded.y_stride(),
                decoded.u_stride(),
                decoded.v_stride(),
            );
            self.tx
                .send(Ok(out))
                .await
                .map_err(|_| crate::Error::new("decoded frame channel closed"))?;
        }
        Ok(())
    }
}
