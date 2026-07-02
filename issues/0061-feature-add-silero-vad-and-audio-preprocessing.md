# Silero VAD と音声前処理ライブラリを実装する

- Priority: Medium
- Created: 2026-06-24
- Completed:
- Model: Opus 4.7
- Branch: feature/add-silero-vad-and-audio-preprocessing
- Polished: 2026-07-02

## 目的

Whisper 推論 (0062) の前段として、Silero VAD (ONNX) による音声区間検出と、リサンプル / チャンク buffer 等の音声前処理ライブラリを `src/ml/audio/` 配下に実装する。親 issue 0012 系列の音声前処理層であり、0062 (Whisper エンジン) の直接の前提となる。

## 優先度根拠

Medium。0062 の前提となる中核層で、単独では利用者から見える機能を提供しない。

## 現状

- hisui には ML 用の音声前処理 (Silero VAD、16 kHz モノラル f32 リサンプル) がない
- `src/ml.rs` は `pub mod device;` の 1 行のみ、`src/ml/audio/` サブディレクトリは未作成。`src/lib.rs:19-20` で `#[cfg(feature = "candle")] pub mod ml;` として親モジュールが既に candle feature ゲート下にあり、`src/ml/device.rs` も candle 依存
- 既存 `src/audio/converter.rs` の `AudioConverter` は `I16Be ↔ I16Be` の線形補間リサンプルとモノラル → ステレオ変換を担うが、(a) 出力が i16、(b) ステレオ → モノラルダウンミックスを明示的に拒否 (`src/audio/converter.rs:91`)、のため VAD 前段用途には使えない
- 既存の音声デコーダ (Opus / AAC / AudioToolbox / fdk-aac) はすべて `AudioFormat::I16Be` の `AudioFrame` を返す
- 既存 `Channels` (`src/audio.rs:15`) は `pub struct Channels(u8)` で `MONO` / `STEREO` の 2 定数のみ、`SampleRate` (`src/audio.rs:51`) は `pub struct SampleRate(u32)`
- 0059 で candle feature (candle-onnx を含む) と `scripts/download_ml_models.py` (`silero-vad` ターゲット、保存先 `<dest>/silero-vad/onnx/model.onnx`、SHA256 = `a4a068cd6cf1ea8355b84327595838ca748ec29a25bc91fc82e6c299ccdc5808`) が完成。protobuf-compiler は 0059 で CI と self-hosted runner に導入済み
- 0059 で `test-candle` CI ジョブが最小骨格 (`cargo clippy` + `cargo test`、`timeout-minutes: 20`) で追加済み。既存 `test-apple-toolbox` (`cargo test --features candle,candle-metal -p hisui`) と `test-nvidia-video-codec` (`cargo test --features candle,candle-cuda -p hisui -- --test-threads=1`) も candle 系ビルド確認を担う
- 0059 からの申し送り事項として「0061 で `src/ml.rs` に `pub mod audio;` 追加、`src/error.rs` に `From<candle_core::Error> for crate::Error` 先着導入」が明示されている
- PoC ブランチ `origin/feature/try-candle` (PR #246) の `src/ml/audio/silero_vad.rs` は Silero VAD v5 の実推論に成功しており、ONNX 入出力仕様と state / context の持ち回し実装は下記「Silero VAD v5 仕様」節に集約する

## 設計方針

### 0059 からの申し送り事項

- `src/ml.rs` の `pub mod device;` に続けて `pub mod audio;` を 1 行追加する。**追加 cfg ゲートは不要** (親 `pub mod ml;` が `src/lib.rs:19-20` で既に `#[cfg(feature = "candle")]` されているため)
- `src/error.rs` の `#[cfg(feature = "nvcodec")] impl From<shiguredo_nvcodec::Error> for Error` (現状 L268-274) の直下・`#[cfg(test)] mod tests` (現状 L276) の直前に、既存 nvcodec impl と同じ 5 行パターンで以下を追加する

```rust
#[cfg(feature = "candle")]
impl From<candle_core::Error> for Error {
    #[track_caller]
    fn from(e: candle_core::Error) -> Self {
        Self::new(e.to_string())
    }
}
```

candle-onnx の関数群は `candle_core::Result` を返すため上記 1 本で足りる。

### モジュール構成

`src/ml/audio/` 配下に以下 5 ファイルを新規追加する:

- `resample.rs`: 任意サンプルレート・任意チャンネル数の PCM を 16 kHz モノラル f32 に変換する関数群 (polyphase FIR)
- `buffer.rs`: `AudioChunkBuffer` (`&[f32]` を固定長チャンクに切り出す pull 型 API)
- `silero_vad.rs`: Silero VAD v5 ONNX モデルロードと 512 サンプル単発推論、state / context 管理
- `vad.rs`: `VadGate` struct (`SileroVad` を保持し、閾値ゲート・発話区間集約を行う)
- `config.rs`: `VadConfig` 等の設定構造体

`vad.rs` の `VadGate` は enum / trait ではなく **struct 直接**とする (Silero VAD 一本、フォールバックなし)。

本 issue は音声前処理層のみ。`MediaFrame::Text` (0060 の成果物) は本 issue で扱わず、0062 の `TranscriptionProcessor` が publish する。

### 型の統一

- チャンネル数は既存 `crate::audio::Channels` を使う (`NonZeroU8` 等を新規に導入しない)
- サンプルレートは既存 `crate::audio::SampleRate` を使う
- パスは Rust 標準の `AsRef<Path>` 慣習を使う
- スライスは素直な `&[f32]` を受け取る (`AsRef<[f32]>` ジェネリックは使わない)

### 公開 API シグネチャ

```rust
use crate::audio::{AudioFrame, Channels, SampleRate};
use std::time::Duration;

// resample.rs
pub fn resample_to_16k_mono(pcm: &[f32], src_hz: SampleRate, channels: Channels) -> crate::Result<Vec<f32>>;
pub fn audio_frame_to_16k_mono(frame: &AudioFrame) -> crate::Result<Vec<f32>>;
```

- `audio_frame_to_16k_mono` は `AudioFormat::I16Be` のみ受け付ける。他フォーマット (`Opus` / `Aac`) は `Err`
- I16Be の PCM は `data.chunks_exact(2)` + `i16::from_be_bytes` で i16 化、`/ 32768.0f32` で f32 正規化、`Channels` に応じてモノ / ステレオを分岐、ステレオはチャンネル平均でダウンミックス
- 空スライスは `Ok(vec![])`
- `resample_to_16k_mono` の出力長は `ceil(input_len * 16000 / src_hz)` (端数切り上げ)
- サポートする src_hz は 8000 / 16000 / 22050 / 24000 / 32000 / 44100 / 48000 Hz。それ以外は `Err`

```rust
// buffer.rs
pub struct AudioChunkBuffer { /* 内部 VecDeque<f32> */ }
impl AudioChunkBuffer {
    pub fn new(chunk_samples: usize) -> Self;
    pub fn push(&mut self, samples: &[f32]);
    /// 蓄積済みが chunk_samples 以上あれば 1 チャンクを取り出す。無ければ None。
    pub fn take_chunk(&mut self) -> Option<Vec<f32>>;
    pub fn remaining(&self) -> usize;
}
```

pull 型 (`take_chunk`) にすることで `while let Some(chunk) = buf.take_chunk() { ... }` で回せ、Iterator の借用境界問題を回避する。

```rust
// silero_vad.rs
/// Silero VAD v5 ONNX モデルの単発推論器。onnx model / state / context / sr / device の 5 フィールドを持つ。
pub struct SileroVad { /* 詳細は「Silero VAD v5 仕様」節参照 */ }
impl SileroVad {
    /// ONNX モデルを開いて state / context / sr テンソルを device 上で初期化する。
    /// パス不在・ONNX パースエラー・テンソル生成失敗は Err。
    /// (PoC は `&Device` を受け取っていたが、lifetime 波及を避けるため本 issue では所有権で受け取る)
    pub fn load<P: AsRef<std::path::Path>>(model_path: P, device: candle_core::Device) -> crate::Result<Self>;
    /// 512 サンプル (32 ms @ 16 kHz) ちょうどのチャンクを受けて発話確率 (0.0 - 1.0) を返す。
    /// chunk.len() != 512 の場合は Err。
    pub fn chunk_probability(&mut self, chunk: &[f32]) -> crate::Result<f32>;
    /// state と context を初期値 (ゼロテンソル) にリセットする (別ストリーム切り替え時に呼ぶ)。
    pub fn reset(&mut self);
}
```

`Device` は `Clone` を実装するため所有権で受け取る (呼び出し側は `select_device()` の返り値を保持し、モデルごとに `device.clone()` を渡す)。

```rust
// vad.rs
pub struct VadGate {
    silero: SileroVad,
    buffer: AudioChunkBuffer, // chunk_samples = 512
    config: VadConfig,
    /* 通し番号カウンタ、確定前 segment 状態 */
}
impl VadGate {
    pub fn new(silero: SileroVad, config: VadConfig) -> Self;
    /// 16 kHz f32 モノラル PCM を受けて確定済みの発話区間 (start_sample 昇順) のみを返す。
    /// 発話継続中や min_silence_ms 未達の区間は Self 内に保持し、次の feed または flush で確定する。
    /// 512 サンプル境界に満たない残余は Self 内 buffer に貯めて次の feed に持ち越す。
    /// 1 回の feed で複数 SpeechSegment を返し得る (feed 内に発話 → 無音 → 発話が複数回起きた場合)。
    pub fn feed(&mut self, samples: &[f32]) -> crate::Result<Vec<SpeechSegment>>;
    /// 現在確定していない segment を強制確定して返す (ストリーム終端で呼ぶ)。
    /// 発話中の場合、min_speech_ms を満たしていれば SpeechSegment として確定、満たしていなければ破棄する。
    pub fn flush(&mut self) -> crate::Result<Vec<SpeechSegment>>;
    /// 通し番号を 0 に戻し、SileroVad::reset を呼ぶ (別 track / 別ストリーム切り替え時)。
    pub fn reset(&mut self);
}

pub struct SpeechSegment {
    /// VadGate::new / reset 以降の 16 kHz サンプル通し番号 (inclusive start、Rust Range 慣習)。
    pub start_sample: u64,
    /// 16 kHz サンプル通し番号 (exclusive end)。
    pub end_sample: u64,
    pub max_probability: f32,
}

impl SpeechSegment {
    /// 16 kHz 換算での発話開始時刻を Duration で返す。
    /// 1 サンプル = 62_500 ns (16000 は 1_000_000_000 の約数) なので丸め誤差ゼロ。
    /// u64::MAX / 62_500 ≈ 9370 年ぶんまでオーバーフロー無しに扱える。
    pub fn start_time(&self) -> Duration {
        Duration::from_nanos(self.start_sample * 62_500)
    }
    pub fn end_time(&self) -> Duration {
        Duration::from_nanos(self.end_sample * 62_500)
    }
}
```

`SpeechSegment` は index のみを持ち PCM 本体を保持しない。**呼び出し側 (0062 の `TranscriptionProcessor`) が `feed` に流し込んだ 16 kHz PCM を全区間保持し、`start_sample` / `end_sample` で slice する責務を負う** (元音源が 48 kHz 等でも Whisper は 16 kHz 入力なので、リサンプル後の 16 kHz PCM を保持するのが自然)。

1 つの `VadGate` は 1 つの track に紐付ける (0062 側で TrackId ごとに `VadGate::new` する)。理由: SileroVad の内部 state / context と VadGate の通し番号カウンタを別 track と混ぜると意味を失うため。

```rust
// config.rs
pub struct VadConfig {
    pub threshold: f32,       // Silero VAD 公式 python wrapper のデフォルト 0.5
    pub min_speech_ms: u32,   // 同 250
    pub min_silence_ms: u32,  // 同 100
}
impl Default for VadConfig { /* 上記デフォルト */ }
```

### Silero VAD v5 仕様

- モデル: `onnx-community/silero-vad` の `onnx/model.onnx`
- ONNX 入力:
  - `input`: `[1, 576]` f32、**dim=1 の先頭に context (前フレーム末尾 64 サンプル)、続いて新規 chunk (512 サンプル) の順で cat**
  - `state`: `[2, 1, 128]` f32、v4 の h/c 分離ではなく v5 では単一 3D テンソル
  - `sr`: `[]` i64 = 16000
- ONNX 出力: `output` (`[1, 1]` f32、発話確率)、`stateN` (`[2, 1, 128]` f32、次呼び出し用 state)
- 入力チャンクは 512 サンプル固定 (v4 の 256 サンプルはサポートしない)
- `SileroVad` の struct が保持するテンソル:
  - `state: Tensor` (`[2, 1, 128]` f32)、初期値ゼロ、`stateN` で置換
  - `context: Tensor` (`[1, 64]` f32)、初期値ゼロ、**推論後に新規 chunk の末尾 64 サンプル (chunk[chunk.len()-64..], ONNX 出力ではない) で置換する**
  - `sr: Tensor` (`[]` i64 = 16000)、load 時に生成、以降不変
  - `device: Device` (load 時に受け取った所有権を保持)
- `chunk_probability` の実装順序: (1) `input = cat(context, chunk, dim=1)` (2) 推論 → 確率と `stateN` を取得 (3) `state = stateN`、`context = chunk[chunk.len()-64..].to_tensor()` (4) 確率を返す
- `SileroVad::load` の Err ケース単体テスト: パス不在 / 非 ONNX バイト列 (magic bytes を持たない 32 byte のランダム風データ)

### リサンプル

- 入力: 任意サンプルレート (`SampleRate::from_u32` で 8000 / 16000 / 22050 / 24000 / 32000 / 44100 / 48000 の 7 通り)、任意チャンネル数 (`Channels::MONO` / `Channels::STEREO`)、`f32` サンプル
- 出力: 16 kHz モノラル `f32`
- アルゴリズム:
  - チャンネル数 > 1 の場合はチャンネル平均でモノラルにダウンミックス
  - 非整数比 (44100 → 16000 等) を扱うため **polyphase FIR** で実装する:
    - **gcd で L/M を簡約**する。`g = gcd(16000, src_hz)`、`L = 16000 / g`、`M = src_hz / g` とする (例: 48000 → 16000 は `g=16000`, `L=1`, `M=3` の単純デシメーション、44100 → 16000 は `g=100`, `L=160`, `M=441`、22050 → 16000 は `g=50`, `L=320`, `M=441`)
    - Prototype filter のタップ数は `L * 64` (base_taps=64) で Kaiser 窓 β=8.6、カットオフ 8000 Hz (Nyquist)
    - サブフィルタ配列は `Vec<Vec<f32>>` として `L` 個保持する。中間バッファは持たない
- Kaiser 窓の設計に必要な第 0 種変形 Bessel 関数 `I0(x)` は Rust std / `libm` (依存追加なし方針) の範囲では利用できないため、`src/ml/audio/resample.rs` 内に級数展開の自前実装を置く
  - 実装は逐次更新: `t_{n+1} = t_n * (x/2)^2 / (n+1)^2` (`t_0 = 1`)。累積和が前項比で `|t_n| / |sum| < 1e-12` になるまで加算 (安全上限 100 項)
- 状態を持たない純関数として実装する

### 既存 `src/audio/converter.rs` との棲み分け

- 既存 `AudioConverter`: I16Be ↔ I16Be、線形補間、モノラル → ステレオ (compose / mixer 向け)
- 本 issue の `resample.rs`: 任意フォーマット → 16 kHz モノラル f32、Kaiser 窓 polyphase FIR、ステレオ → モノラルダウンミックス (VAD / Whisper 向け)

用途と精度要件が異なるため共通化しない。

### モデルパス規約と integration テスト skip 動線

- ライブラリ API はパスを引数で受け取る (環境変数や固定パスにしない)
- Integration テストは環境変数 `HISUI_ML_MODELS_DIR` を **テスト内 `std::env::var` から直読** する
  - `HISUI_ML_MODELS_DIR` が **未設定 or 対象ファイル不在の場合はテストを skip** (`println!` で理由を出す)
  - ただし CI での silent skip を検知するため、環境変数 `HISUI_CI=1` が設定されているときは skip せず panic する。CI 側で `HISUI_CI: "1"` を job level env に設定して仕様退行を防ぐ
- CI では job level env で `HISUI_ML_MODELS_DIR: ${{ github.workspace }}/ml-models` (絶対パス) + `HISUI_CI: "1"` として export する
- ライブラリ API は環境変数を読まない
- `scripts/download_ml_models.py` は 0059 で silero-vad ターゲット定義済み。本 issue では script 側を変更しない

### テスト

配置は shiguredo-rust 規約に従う。階層 module は `_` 区切りで表現する (既存 `tests/test_ml_device.rs` の先例)。

- 単体テスト:
  - `tests/test_ml_audio_resample.rs`: リサンプル、Bessel I0 精度検証 (`i0(0.0) = 1.0` 厳密、`i0(1.0) ≈ 1.2660658732`、`i0(8.6) ≈ 894.62` を相対誤差 `1e-6` 未満)
  - `tests/test_ml_audio_buffer.rs`
  - `tests/test_ml_audio_vad.rs` (VadGate の集約ロジック、モデル不要な部分)
- PBT: `pbt/tests/prop_ml_audio/` サブディレクトリ分割 (既存 `pbt/tests/prop_tune/` に倣う)
  - `main.rs` (`mod resample_props; mod buffer_props; mod vad_props;` の宣言)
  - `resample_props.rs` / `buffer_props.rs` / `vad_props.rs`
- Integration テスト: `tests/test_ml_audio_silero_vad.rs` (`#![cfg(feature = "candle")]`) で実 Silero VAD ロード + 実推論

PBT の不変条件 (最低限):

- **resample** (`resample_to_16k_mono`):
  - 出力長 = `ceil(input_samples * 16000 / src_hz)` (端数切り上げ)
  - 同一入力で 2 回リサンプルすると同一出力 (決定性)
  - 定数信号 (DC) は定数信号に写る (DC ゲイン 1.0 の許容誤差内)
  - ステレオ入力はチャンネル平均でモノラルにダウンミックスされる
- **AudioChunkBuffer**:
  - `push` した総サンプル数 = `take_chunk` で取り出した総サンプル数 + `remaining()`
  - `take_chunk` で取り出したチャンクは常に `chunk_samples` に一致
- **VadGate**:
  - 全区間で `chunk_probability` が閾値未満なら空 `Vec` を返す
  - 閾値超え区間長が `min_speech_ms` 未満なら SpeechSegment としてカウントされない
  - 返り値の `Vec<SpeechSegment>` は `start_sample` 昇順に並ぶ
  - VadGate の集約部分を純関数として切り出し、SileroVad モックなしで検証できる設計にする

Integration テストの内容:

- **合成音源による基本動作** (発話陽性判定は 0062 で担う):
  - `SileroVad::load` が `Err` を返さない
  - 3 秒の zero-fill を `VadGate::feed` に流し、返る `Vec<SpeechSegment>` が空
  - 512 サンプル zero-fill を 3 回 `SileroVad::chunk_probability` に流し、返る確率が全て `< 0.5` かつ 3 回の値が等しい (state 初期状態でゼロ入力は決定的)
- **エラーパス**:
  - パス不在で `SileroVad::load` が `Err`
  - 非 ONNX バイト列 (magic bytes を持たない 32 byte) で `SileroVad::load` が `Err`
- **skip 動線**:
  - 開発者ローカル: `HISUI_ML_MODELS_DIR` 未設定 or ファイル不在なら `println!` skip
  - CI: `HISUI_CI=1` が設定されているので上記条件でも panic (退行検知)
  - `test-candle` (Ubuntu、実モデル取得あり + `HISUI_CI=1`): 実行される
  - `test-apple-toolbox` / `test-nvidia-video-codec` (本 issue では実モデル取得を積まない): `HISUI_CI` を設定しないので skip される

生の発話音声を使った発話陽性判定 integration テストは 0062 で追加する。

### CI (`.github/workflows/ci.yml`) 更新

`test-candle` ジョブに以下 step を積み増す:

- **uv セットアップ**: 既存 `.github/workflows/pytest.yml` L53 と同じ `astral-sh/setup-uv@fac544c07dec837d0ccb6301d7b5580bf5edae39 # v8.2.0` を使う
- **モデルキャッシュ**: `actions/cache@v4` (既存 CI の SHA pin 慣習に揃える)
  - `path: ml-models/silero-vad`
  - `key: silero-vad-a4a068cd6cf1ea8355b84327595838ca748ec29a25bc91fc82e6c299ccdc5808` (`download_ml_models.py` の SHA256 全 64 桁をベタ書き)
  - `restore-keys` は使わない (prefix match で古い cache を復元しても hash 検証で reject されるため無駄)
- **モデル取得**: `uv run scripts/download_ml_models.py --dest ml-models/ silero-vad`
- **環境変数**: `test-candle` ジョブの env セクション (job level) に以下 2 つを追加
  - `HISUI_ML_MODELS_DIR: ${{ github.workspace }}/ml-models`
  - `HISUI_CI: "1"`

`test-apple-toolbox` と `test-nvidia-video-codec` は本 issue では実モデル取得を積まない (それぞれ 0062 の Whisper 実推論・CUDA 実推論のタイミングで判断)。両ジョブは `HISUI_ML_MODELS_DIR` / `HISUI_CI` を設定しないため integration テストは skip される。

`timeout-minutes` は現状 20 分を維持する (silero-vad は 2 MB 未満で軽量)。

### pbt/Cargo.toml への candle feature 伝播

PBT 対象 (`resample` / `buffer` / `VadGate` 集約ロジック) は `src/ml/audio/` 配下 (candle feature ゲート下) にあるため、pbt から touch するには `pbt/Cargo.toml` の `hisui` 依存に `features = ["candle"]` を付ける必要がある。

- `pbt/Cargo.toml` の `hisui = { path = ".." }` を `hisui = { path = "..", features = ["candle"] }` に変更する
- 変更に伴い `cargo check --workspace --no-default-features` が通ることは、**実装着手時の最初のコミットで検証する**。protobuf-compiler が host にインストール済み (0059 で完了) の環境で通る想定
- 万一通らない場合の代替として `default-features = false, features = ["candle"]` に切り替える。切り替えを採用した場合は選択理由 (default feature の何が pbt で不要か) を PR に記録する
- 実 Silero VAD 推論を PBT で回す想定は本 issue にはない (integration テストの担当)

### CHANGES.md エントリ

本 issue は内部実装 (音声前処理ライブラリ層) のため CHANGES.md エントリは追加しない (親 0012 L110-111 の確定方針)。

### 依存

- `candle-onnx` (0059 で追加済み、バージョン `=0.11.0`) を Silero VAD の ONNX ロード・推論に使う
- 外部 crate 依存は追加しない

## 完了条件

- `src/ml.rs` に `pub mod audio;` が追加されている (追加 cfg ゲートは付けない)
- `src/ml/audio/{resample, buffer, silero_vad, vad, config}.rs` の 5 ファイルが新規追加されている
- `src/error.rs` に `#[cfg(feature = "candle")] impl From<candle_core::Error> for Error` が既存 nvcodec impl (L268-274) の直下・`#[cfg(test)] mod tests` (L276) の直前に追加されている
- 各モジュールの公開 API シグネチャが本 issue の「公開 API シグネチャ」節と整合している (`Channels` / `SampleRate` / `Duration::from_nanos(sample * 62_500)` / `AsRef<Path>` の型選択を含む)
- Silero VAD v5 の context 更新順序 (推論後に `chunk[chunk.len()-64..]` を新 context にする) が実装されている
- polyphase FIR のリサンプルは `L / M` を gcd で簡約した上で実装されている
- 単体テスト (`tests/test_ml_audio_{resample, buffer, vad}.rs`) が追加され green、Bessel I0 精度が本 issue のテスト節で規定した数値目標を満たす
- PBT (`pbt/tests/prop_ml_audio/{main, resample_props, buffer_props, vad_props}.rs`) が追加され green
- Integration テスト (`tests/test_ml_audio_silero_vad.rs`) が追加され、zero-fill / エラーパス / skip 動線が本 issue のテスト節通りに動作する
- `pbt/Cargo.toml` の `hisui` 依存に `features = ["candle"]` (または代替の `default-features = false, features = ["candle"]`) が追加され、下記 cargo コマンド一式が引き続き green
- `.github/workflows/ci.yml` の `test-candle` ジョブに uv セットアップ / モデルキャッシュ / `download_ml_models.py` step / `HISUI_ML_MODELS_DIR` + `HISUI_CI` (job level env) が追加されている
- 次のコマンドがすべて green:
  - `cargo fmt --all --check`
  - `cargo check --workspace --no-default-features`
  - `cargo clippy --workspace --no-default-features -- --deny warnings`
  - `cargo check --workspace`
  - `cargo clippy --workspace --all-targets -- --deny warnings`
  - `cargo test --workspace`
  - `cargo clippy --features candle --all-targets -p hisui -- --deny warnings`
  - `cargo test --features candle -p hisui`

## 解決方法

設計方針に従って `src/ml/audio/` 配下の 5 モジュールを新規追加し、`src/ml.rs` への `pub mod audio;` 追加と `src/error.rs` への `From<candle_core::Error>` 実装追加を行う。`pbt/Cargo.toml` に candle feature を伝播させる。`.github/workflows/ci.yml` の `test-candle` ジョブに uv・モデルキャッシュ・モデル取得 step・`HISUI_ML_MODELS_DIR` + `HISUI_CI` 環境変数を追加する。PoC ブランチ (`origin/feature/try-candle`) の `src/ml/audio/silero_vad.rs` の実推論成功実績を根拠に、SileroVad の API を新規設計する (Device は `&Device` から所有権受け渡しに変更、context 更新規則は本 issue の「Silero VAD v5 仕様」節で確定)。

### 後続 issue 側に必要な追記

- 0062 (Whisper): `AudioChunkBuffer::new(30 * 16000)` を Whisper 30 秒チャンク生成に流用する。`SpeechSegment` の `start_sample` / `end_sample` (16 kHz 換算) を使って呼び出し側が保持する 16 kHz PCM を slice する責務を負う。実発話音声を使った integration テスト (testdata 実音源) は 0062 で追加する
- 0064 (YOLO): 本 issue で追加した `From<candle_core::Error> for crate::Error` を再利用する
