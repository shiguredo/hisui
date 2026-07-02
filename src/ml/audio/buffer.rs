//! 任意サンプル数の f32 PCM を固定長チャンクに切り出す pull 型 buffer。

use std::collections::VecDeque;
use std::num::NonZeroUsize;

/// 入力 PCM を固定長チャンクに分割して取り出すバッファ。
///
/// pull 型 API (`take_chunk`) にすることで `while let Some(chunk) = buf.take_chunk() { ... }` で
/// 回せる。`chunk_samples = 0` の無限ループを防ぐため、コンストラクタは `NonZeroUsize` を要求する。
#[derive(Debug)]
pub struct AudioChunkBuffer {
    chunk_samples: usize,
    inner: VecDeque<f32>,
}

impl AudioChunkBuffer {
    /// 指定した固定長のチャンクを取り出す buffer を作成する。
    pub fn new(chunk_samples: NonZeroUsize) -> Self {
        Self {
            chunk_samples: chunk_samples.get(),
            inner: VecDeque::new(),
        }
    }

    /// PCM サンプルを末尾に追加する。
    pub fn push(&mut self, samples: &[f32]) {
        self.inner.extend(samples);
    }

    /// 蓄積済みが `chunk_samples` 以上あれば 1 チャンクを取り出す。無ければ `None`。
    pub fn take_chunk(&mut self) -> Option<Vec<f32>> {
        if self.inner.len() < self.chunk_samples {
            return None;
        }
        let chunk: Vec<f32> = self.inner.drain(..self.chunk_samples).collect();
        Some(chunk)
    }

    /// 未取り出しのサンプル数を返す。
    pub fn remaining(&self) -> usize {
        self.inner.len()
    }
}
