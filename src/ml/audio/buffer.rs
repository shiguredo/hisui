//! 任意サンプル数の f32 PCM を固定長チャンクに切り出す pull 型 buffer。

use std::collections::VecDeque;

/// 入力 PCM を固定長チャンクに分割して取り出すバッファ。
///
/// pull 型 API (`take_chunk`) にすることで `while let Some(chunk) = buf.take_chunk() { ... }` で
/// 回せる。Iterator を返す設計だと `&mut self` の借用境界問題が発生するのを回避する目的。
#[derive(Debug)]
pub struct AudioChunkBuffer {
    chunk_samples: usize,
    inner: VecDeque<f32>,
}

impl AudioChunkBuffer {
    /// 指定した固定長のチャンクを取り出す buffer を作成する。
    pub fn new(chunk_samples: usize) -> Self {
        Self {
            chunk_samples,
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
