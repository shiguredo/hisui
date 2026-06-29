use std::collections::VecDeque;

use tokio::sync::mpsc;

use crate::video::{VideoFormat, VideoFrame};

/// Sender 化された libvpx (VP8 / VP9) デコーダー
///
/// デコード済みフレームはコンストラクタで受け取った `tx` 経由で上位に送信する。
/// 旧 pull 型 API (`next_decoded_frame()`) は廃止し、`decode()` / `finish()` は
/// 内部で `tx.send().await` を呼ぶ async fn に統一している。
#[derive(Debug)]
pub struct LibvpxDecoder {
    inner: shiguredo_libvpx::Decoder,
    // デコード済みフレームと元入力フレームを対応付けるための保持列
    input_queue: VecDeque<VideoFrame>,
    // デコード結果を上位 run() の Receiver に流す Sender
    tx: mpsc::Sender<crate::Result<VideoFrame>>,
}

impl LibvpxDecoder {
    pub fn new_vp8(tx: mpsc::Sender<crate::Result<VideoFrame>>) -> crate::Result<Self> {
        tracing::debug!("create libvpx(VP8) decoder");
        Ok(Self {
            inner: shiguredo_libvpx::Decoder::new(shiguredo_libvpx::DecoderConfig::new(
                shiguredo_libvpx::DecoderCodec::Vp8,
            ))?,
            input_queue: VecDeque::new(),
            tx,
        })
    }

    pub fn new_vp9(tx: mpsc::Sender<crate::Result<VideoFrame>>) -> crate::Result<Self> {
        tracing::debug!("create libvpx(VP9) decoder");
        Ok(Self {
            inner: shiguredo_libvpx::Decoder::new(shiguredo_libvpx::DecoderConfig::new(
                shiguredo_libvpx::DecoderCodec::Vp9,
            ))?,
            input_queue: VecDeque::new(),
            tx,
        })
    }

    pub async fn decode(&mut self, frame: &VideoFrame) -> crate::Result<()> {
        if !matches!(frame.format, VideoFormat::Vp8 | VideoFormat::Vp9) {
            return Err(crate::Error::new(format!(
                "expected VP8 or VP9 format, got {:?}",
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

    /// libvpx の出力フレームを取り出して Sender に送る
    async fn send_decoded_frames(&mut self) -> crate::Result<()> {
        while let Some(image) = self.inner.next_frame()? {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("decoded frame produced without input frame"))?;
            let out = if image.is_high_depth() {
                // 高ビット深度データは I420 に正規化する
                VideoFrame::new_i420_from_high_depth(
                    input_frame,
                    image.width(),
                    image.height(),
                    image.y_plane(),
                    image.u_plane(),
                    image.v_plane(),
                    image.y_stride(),
                    image.u_stride(),
                    image.v_stride(),
                )?
            } else {
                // 通常の 8 ビット I420 データ
                VideoFrame::new_i420(
                    input_frame,
                    image.width(),
                    image.height(),
                    image.y_plane(),
                    image.u_plane(),
                    image.v_plane(),
                    image.y_stride(),
                    image.u_stride(),
                    image.v_stride(),
                )
            };
            self.tx
                .send(Ok(out))
                .await
                .map_err(|_| crate::Error::new("decoded frame channel closed"))?;
        }
        Ok(())
    }
}
