# Mp4FileReader の video decoder 経路を AsyncVideoDecoder に移行して mp4 reader を async fn 化する

- Priority: Medium
- Created: 2026-07-02
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-mp4-reader-async-video-decoder
- Polished: 2026-07-02
- Reporter: @sile
- Decision Owner: @sile

## 目的

closed issue 0066 で `AsyncVideoDecoder` が追加された状態から、 `src/mp4/reader.rs` の **video decoder 経路** と、 その `set_video_decoder` を経由して decoder を注入する `src/obsws/source/file_mp4.rs` を `AsyncVideoDecoder` ベースに切り替える。

**audio decoder はスコープ外**。 `AsyncAudioDecoder` が未整備のため、 `audio_decoder: Option<AudioDecoder>` 経路 (`handle_audio_sample` / `flush_decoders` の audio 側 / `recreate_decoders` の audio 側 / `Mp4FileReader::set_audio_decoder`) は同期のまま維持する。

video 側の変更は次のとおり:

- `Mp4FileReader` の 5 関数 (`flush_decoders` / `reset_for_restart` / `apply_seek` / `recreate_decoders` / `send_eos_to_tracks`) を async fn 化
- 呼出元 18 箇所への `.await` 追従 (推奨案 §3 の詳細参照)
- `TrackSender` を decoder task に move し SYN/ACK 背圧を有効化 (推奨案 §2)
- warm-up 経路の意味論を `watch::channel<bool>` で保った上で `discard_video_decoder_output` 廃止 (推奨案 §1)
- `loop_playback` 5 経路 (継続 / Restart / Seek / reset_for_restart / Stop) 別の decoder task ライフサイクル管理 (推奨案 §3)
- `Mp4FileReader::set_video_decoder` 廃止 + `Mp4FileReaderOptions` への `video_decoder_options: Option<VideoDecoderOptions>` 追加

Branch prefix は `feature/refactor-` を採用する (兄弟 issue 0068 / 0072 / 0073 と整合、 外部プロトコルへの後方互換破壊なし)。

## 優先度根拠

Medium。

- closed issue 0066 の wrap 段階的移行方針 (δ) を closed issue 0057 §3 「中途半端な 2 系統共存を残さない」原則と整合させるには全使用側の移行が必要
- 本 issue 単独では外部挙動 (再生タイミング / 出力) は不変。 内部リファクタ相当で緊急性なし
- 後続の open issue 0073 (最終クリーンアップ) が本 issue 完了を待つ (open issue 0072 は互いに独立)

## 現状

`src/mp4/reader.rs` (2336 行) の video 経路が同期 pull pattern。 書き換え対象は以下:

### 同期関数の async fn 化 (5 関数、 呼出元 18 箇所)

| 関数 | 定義位置 | 呼出元 |
|------|----------|--------|
| `flush_decoders` | `:1274` | `:339, :350` (2 箇所) |
| `reset_for_restart` | `:1340` | `:378, :383` (2 箇所) |
| `apply_seek` | `:638` | `:465, :479, :484, :498, :503, :523, :528` (7 箇所) |
| `recreate_decoders` | `:1350` | `:475, :494, :519, :645, :1346` (5 箇所) |
| `send_eos_to_tracks` | `:1292` | `:341, :389` (2 箇所) |

### VideoDecoder 直叩き箇所 (video 側 7 箇所)

| API | 呼出位置 |
|-----|----------|
| `decoder.handle_input_sample(Some(...))` | `:1195, :1233, :1279, :1285` (4 箇所) |
| `crate::decoder::drain_video_decoder_output(decoder, ...)` | `:1236, :1286` (2 箇所) |
| `discard_video_decoder_output(decoder)` | `:1199` (定義は module-private helper `:1388`) |

`:1195` の `handle_input_sample` と `:1199` の `discard_video_decoder_output` は同一 if ブロック (`suppress_publish=true`、 warm-up 中) 内でペア動作。

audio 側の直叩き (`handle_audio_sample` `:1067` 内 / `flush_decoders` の audio 側 / `discard_decoder_output` `:1104` / `drain_audio_decoder_output` `:1138`) は削除しない。

### video_sender field 削除の副次修正 (2 箇所)

`Mp4FileReader.video_sender: Option<TrackSender>` (`:221`) を削除すると以下の判定条件も修正必要:

- `:327` の `if self.audio_sender.is_none() && self.video_sender.is_none()` → `if self.audio_sender.is_none() && !self.has_video_track()` に変更
- `:448` の `ReaderState::open(&self.path, self.audio_sender.is_some(), self.video_sender.is_some())?` → `has_video_track()` 判定に変更

ただし `has_video_track()` (`:308`) は `self.options.video_track_id.is_some()` を判定するが、 現状 `build_track_senders` (`:428` 付近) で `self.options.video_track_id.take()` して消費している。 この take を「clone に変える」または「別 field `video_publish_enabled: bool` を build 前に保存する」いずれかで対応 (推奨案 §4 参照)。

### TrackSender SYN/ACK 背圧

- `MAX_NOACKED_COUNT: u64 = 100` (`:24`)
- `struct TrackSender` (`:1446`)、 `send_video(&mut self, frame: VideoFrame) -> bool` (`:1481`)、 `send_eos(&mut self)` (`:1490`)
- 現状 decoder あり経路では `TrackPublisher` (`sender.sender`) を `drain_video_decoder_output` に直接渡しており、 `TrackSender::send_video` の `prepare_send().await` バイパス = 背圧なし。 decoder なし経路 (`sender.send_video` `:1241` 付近) は背圧あり
- 本 issue で `TrackSender::send_media(sample: MediaFrame) -> bool` を新設し、 decoder task 内で `sample` (MediaFrame) をそのまま流せるようにする。 `send_media` は内部で `MediaFrame::Video(arc)` を受けて `TrackPublisher::send_media(sample)` に委譲する形。 これにより背圧を有効化しつつ骨子コードの型整合も保つ

### loop_playback の decoder ライフサイクル発火点

`run_loop` (`:438` 定義、 while 本体 `:465-533`) と `wait_for_restart_command` 経路で 5 種類:

1. **loop 継続 (EOF `continue`)**: `:544`。 `recreate_decoders` 呼ばず decoder 継続 (次 loop 先頭はキーフレーム保証)
2. **`MediaLoopAction::Restart`**: `:475, :494, :519` で `recreate_decoders`
3. **`MediaLoopAction::Seek` / `OffsetSeek`**: `apply_seek` (`:465, :479, :484, :498, :503, :523, :528`) 経由、 その中で `recreate_decoders` (`:645`)
4. **`reset_for_restart` 経由** (`wait_for_restart_command` の `WaitResult::Play` / `Restart`): `:378, :383` で `reset_for_restart`、 その中で `recreate_decoders` (`:1346`)
5. **`MediaLoopAction::Stop`**: `RunLoopResult::Stopped` で run_loop を抜けて待機。 decoder 保持継続

### AsyncVideoDecoder の現状 API

`src/decoder.rs` (0066 完了時点):

- `pub struct AsyncVideoDecoder` (`:385`)
- `pub fn new(options: VideoDecoderOptions, stats: Stats) -> Self` (`:400`)
- `pub fn handle_input_sample_sync(&mut self, Option<MediaFrame>) -> Result<()>` (`:424`)
- `pub fn poll_output_sync(&mut self) -> Result<DecoderRunOutput>` (`:441`)
- `pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>>` (`:472`)

open issue 0068 (polished 2026-07-02) で `AsyncVideoDecoder::run` の追加が確定、 `handle_input_message` は追加しないことが確定。

本 issue の decoder task は `AsyncVideoDecoder::run` を再利用せず自前 loop を組む。 理由:

- warm-up 中の discard 制御 (`discard_mode_tx`) が 0068 の `run` にはない
- `TrackSender::send_media` (背圧あり) を task 側で呼ぶ (0068 の `run` は `TrackPublisher::send_media` を直接呼び背圧なし)
- Stop 経路 (経路 5) で task を継続保持する制御 (0068 の `run` は Finished / PipelineClosed で終了)

### 既存テスト

- `src/mp4/reader.rs:1697-2336` の `#[cfg(test)] mod tests`
- **既存テスト内で `apply_seek` / `flush_decoders` / `recreate_decoders` / `reset_for_restart` を直接呼ぶテストは存在しない** (テスト内コメント `:1801-1803, :2068, :2106` で「`ProcessorHandle` / `ReaderState` が必要なので直接呼ばず内部状態だけ手動設定」と明記)。 したがって async fn 化に伴う既存テストの `#[tokio::test]` 化は不要 (0 件)
- 新規に「decoder task の生存 / 死亡 / EOS シーケンス / warm-up mode 遷移」の統合テストを追加する場合は実 pipeline 経由で書く

## 設計方針

### 決定事項 (実装で覆さない)

- `AsyncVideoDecoder` は 0066 導入分を利用 (再設計しない)
- decoder ライフサイクルは spawn pattern。 main task 内で `.await` 直呼出はしない
- audio decoder は同期のまま維持
- `AsyncVideoDecoder::run` / `handle_input_message` は本 issue で追加しない (自前 dispatch)
- Nvcodec feature 有効時と無効時で挙動差分なし

### 推奨案 §1: warm-up 経路 → **case A (task 内 discard mode 制御)**

- decoder task が `watch::Receiver<bool>` (discard_mode) を保持
- `handle_video_sample` の suppress_publish 判定 (`:1041` 付近) で `warmup_target` の状態遷移が起きた際に main が `discard_mode_tx.send(true/false)` を呼ぶ
- 新 task 起動時の初期値: `discard_mode = true` で spawn する (Seek 直後などは warm-up 突入する可能性があるため、 誤って publish しないよう安全側)。 通常再生開始時は `handle_video_sample` の初回呼出で warm-up 不要と判定した時点で `discard_mode_tx.send(false)` に切り替わる
- discard_mode の実際の発火タイミング (毎 sample か遷移点か) と audio 側 warm-up との整合は **実装段階で確定**する (残懸念、 §「残懸念」参照)

### 推奨案 §2: TrackSender → **case b (decoder task に move、 背圧有効化)**

- `Mp4FileReader.video_sender` field 削除
- 本 issue で `TrackSender::send_media(&mut self, sample: MediaFrame) -> bool` を新設 (`TrackPublisher::send_media` に委譲する薄いラッパ、 内部で SYN/ACK 背圧を効かせる)
- decoder task 生成時に `TrackSender` を move、 task 内で `sender.send_media(sample).await` を呼ぶ
- `TrackSender` は現状 `Send + Sync` (`sender: TrackPublisher` と `Ack: mpsc::Receiver<()>` の組合せ、 いずれも Send)
- audio_sender は main で維持 (audio 側は同期のまま)

### 推奨案 §3: loop_playback 5 経路別ライフサイクル

| 経路 | 前 task の始末 | 新 task |
|------|---------------|---------|
| 1. loop 継続 | そのまま継続 | 生成しない |
| 2. `MediaLoopAction::Restart` | EOS 送信 → `JoinHandle::await` | `recreate_decoders` 内で新 spawn |
| 3. `MediaLoopAction::Seek` / `OffsetSeek` | `JoinHandle::abort()` (残フレーム破棄) | `apply_seek` 内で新 spawn |
| 4. `reset_for_restart` 経由 | EOS 送信 → `JoinHandle::await` | `recreate_decoders` 内で新 spawn |
| 5. `MediaLoopAction::Stop` | そのまま継続保持 | 生成しない (次の Play/Restart で経路 4 に合流) |

`base_offset` 更新順序は現状実装を維持 (経路別):

- `reset_for_restart` (`:1342-1346`): `base_offset` 更新 → 前 task EOS+join → 新 task spawn (`recreate_decoders`)
- `apply_seek` (`:645-662`): 前 task abort → 新 task spawn (`recreate_decoders`) → `base_offset` 更新

`reset_for_restart_preserves_timestamp_continuity` テストは `base_offset` 更新順序を検証しており、 現状順序を保つことで通る。

Seek 時に abort を採用する理由: EOS+drain 待ちすると seek 前フレームが post-seek 位置で publish される可能性があるため、 即時終了で破棄。 新 task 起動時に `TrackSender` を再作成する race (初回 SYN 待ち) が warm-up 明けの最初の publish で発生し得るが、 これは既存の decoder なし経路が既に持っている挙動と同じ扱いで許容 (残懸念参照)。

### 推奨案 §4: Mp4FileReaderOptions への注入方式

`Mp4FileReaderOptions` (`:203`) に以下を追加:

```rust
pub struct Mp4FileReaderOptions {
    // 既存 field
    pub video_decoder_options: Option<VideoDecoderOptions>,
}
```

- `None` の場合: video decoder task は spawn しない。 raw video publish 経路 (現状の `sender.send_video` 直呼出) は本 issue で削除する (呼出元は `obsws/source/file_mp4.rs` のみで、 常に video decoder を設定しているため raw publish は使われていない)
- `Some(options)`: `Mp4FileReader::run` の冒頭で `spawn_video_decoder_task(options.clone(), ...)` を呼ぶ (`take` ではなく `clone`。 `recreate_decoders` で複数回参照するため)
- `openh264_lib` は `VideoDecoderOptions.openh264_lib` (`decoder.rs:324`) に含まれるが、 `Mp4FileSource::create_reader` (`obsws/source/file_mp4.rs:21-36`) は `ProcessorHandle` を持たないため options 構築時に `openh264_lib` を埋め込めない。 対処: `openh264_lib` を除いた `VideoDecoderOptions` を `create_reader` で構築し、 `Mp4FileReader::run` 内で `handle.config().openh264_lib.clone()` を merge して補完する
- `Mp4FileReaderOptions` の struct literal を全 field 明示している呼出元 (`obsws/source/file_mp4.rs:25-30`) は `video_decoder_options: Some(VideoDecoderOptions::default())` の明示または `..Default::default()` 追加が必要 (`#[non_exhaustive]` は付けない)
- `has_video_track()` の判定を build_track_senders 後も維持するため、 `build_track_senders` は `video_track_id.take()` ではなく `clone()` に変更 (副次修正)

### 推奨案 §5: decoder task 入力 channel → unbounded + 専用 enum

- `tokio::sync::mpsc::unbounded_channel::<DecoderInput>()`
- 型: `enum DecoderInput { Media(MediaFrame), Eos }` (`MediaFrame::Video(Arc<VideoFrame>)` をそのまま流す、 二重変換を避ける)
- `crate::Message` (Media / Eos / Syn) は使わない (Syn は mp4 reader レベルで無視)
- 背圧は下流 SYN/ACK (推奨案 §2) が担う

### spawn pattern の骨子

```rust
struct VideoDecoderTask {
    input_tx: tokio::sync::mpsc::UnboundedSender<DecoderInput>,
    discard_mode_tx: tokio::sync::watch::Sender<bool>,
    join_handle: tokio::task::JoinHandle<crate::Result<()>>,
}

impl VideoDecoderTask {
    async fn shutdown(self) -> crate::Result<()> {
        let _ = self.input_tx.send(DecoderInput::Eos);
        // JoinHandle::await の Err は panic の場合 JoinError::is_panic() で判定可能。
        // 呼出側は shutdown 経路で warn 握り潰し、 panic は明示的に log。
        match self.join_handle.await {
            Ok(result) => result,
            Err(e) => Err(crate::Error::new(format!("video decoder task join failed: {e}"))),
        }
    }
    fn abort(self) {
        self.join_handle.abort();
    }
}

pub struct Mp4FileReader {
    // 削除: video_sender: Option<TrackSender> (task に move)
    // 削除: video_decoder: Option<VideoDecoder>
    video_decoder_task: Option<VideoDecoderTask>,
    // audio_sender / audio_decoder は現状維持
}
```

decoder task の loop 本体骨子 (0068 の骨子と同型、 warm-up mode / TrackSender / Stop 経路のみ mp4 特有):

```rust
async fn video_decoder_loop(
    options: VideoDecoderOptions,
    stats: crate::stats::Stats,
    mut input_rx: tokio::sync::mpsc::UnboundedReceiver<DecoderInput>,
    discard_mode_rx: tokio::sync::watch::Receiver<bool>,
    mut sender: TrackSender,
) -> crate::Result<()> {
    let mut decoder = AsyncVideoDecoder::new(options, stats);
    loop {
        let input = match input_rx.recv().await {
            Some(input) => input,
            None => {
                // main が VideoDecoderTask を drop (通常経路の shutdown の途中 or 緊急停止)。
                // shutdown 経路なら EOS が事前に送られていて Finished で早期 return 済み。
                // 到達時は緊急経路のため send_eos は呼ばず終了する
                return Ok(());
            }
        };
        let is_eos = matches!(input, DecoderInput::Eos);
        match input {
            DecoderInput::Media(sample) => decoder.handle_input_sample_sync(Some(sample))?,
            DecoderInput::Eos => decoder.handle_input_sample_sync(None)?,
        }
        // Openh264 は 1 サンプル入力で 0-2 frame 出力する (closed/0066 参照) ため、
        // Pending / Finished に達するまで内側 loop で drain する必要がある
        loop {
            match decoder.poll_output_sync()? {
                DecoderRunOutput::Processed(sample) => {
                    // Ref<'_, bool> は `*` deref 直後に drop され await を跨がない
                    if !*discard_mode_rx.borrow() {
                        if !sender.send_media(sample).await {
                            return Ok(());
                        }
                    }
                }
                DecoderRunOutput::Pending => break,
                DecoderRunOutput::Finished => {
                    sender.send_eos();
                    return Ok(());
                }
            }
        }
        // handle_input_sample_sync(None) 後は poll_output_sync が必ず Finished を返す
        // 不変条件のため到達不能 (0068 の骨子 :169 参照)
        if is_eos {
            return Err(crate::Error::new("video decoder task still pending after EOS"));
        }
    }
}
```

`spawn_video_decoder_task` は decoder task を生成して `VideoDecoderTask` を返す。 spawn の際に `stats.set_default_label("component", "video_decoder")` を実行してから `AsyncVideoDecoder::new` を呼ぶ (現状 `recreate_decoders` の `:1368` と同じ処理)。

### エラーパス

- **decoder task の panic**: `JoinHandle::await` の Err が `JoinError::is_panic()` なら `error!` log、 上位 (`Mp4FileReader::run`) は `crate::Error` を返して pipeline 停止
- **`sender.send_media` 失敗 (pipeline closed)**: task 内で `return Ok(())`。 main の次回 `input_tx.send` が Err → main が停止を検知
- **`poll_output_sync` の Err (Nvcodec 非同期 callback エラー)**: `?` で伝搬、 task が `Err(e)` で終了。 上位経路と同じ扱い
- **Err 経路で `send_eos` を呼ばない**: 下流は `TrackPublisher` drop 時の subscriber close (`Message::Eos` と `SubscriberTx drop` の両方) で終了検知する既存契約に依拠
- **loop 継続中に task 死亡**: main が `input_tx.send` 失敗で検知
- **`recreate_decoders` 中の hung**: EOS 送信後の drain で SYN/ACK 背圧により無制限に await する可能性あり (subscribers 側で drain が正常進行している前提)。 timeout は付けない
- **`Mp4FileReader::run` の緊急停止 (Err/panic) で task leak**: 実装段階で `Drop` トレイト実装で `task.abort()` を追加するかは prototype で確定 (残懸念)

### send_eos_to_tracks の変更

現状 (`:1292` 定義、 呼出 `:341, :389` の 2 箇所) は同期 fn で audio + video 両方に `send_eos()`。 変更:

```rust
async fn send_eos_to_tracks(&mut self) {
    if let Some(sender) = self.audio_sender.as_mut() {
        sender.send_eos();
    }
    // video 側は decoder task の shutdown 内で send_eos が呼ばれる
    if let Some(task) = self.video_decoder_task.take() {
        if let Err(e) = task.shutdown().await {
            tracing::warn!("video decoder task shutdown failed: {e}");
        }
    }
}
```

呼出元 2 箇所 (`:341`, `:389`) に `.await` を付与。

### recreate_decoders の signature

- `async fn recreate_decoders(&mut self, handle: &ProcessorHandle)` (Result は返さない、 現状維持)
- audio 側: 現状通り `AudioDecoder::new` の Err を `tracing::warn` で握り潰し
- video 側: `self.options.video_decoder_options.as_ref().cloned()` (take ではない) で options を取得、 `openh264_lib` を `handle.config()` から merge、 前 task を経路別に始末 (推奨案 §3)、 `spawn_video_decoder_task` で新 task 生成
- 前 task の join Err は `tracing::warn` で握り潰し進行

## 完了条件

- `Mp4FileReader::set_video_decoder` (`:318`) が削除されている
- 5 関数 (`flush_decoders` / `reset_for_restart` / `apply_seek` / `recreate_decoders` / `send_eos_to_tracks`) が async fn 化されている
- 上記 5 関数への 18 呼出元すべてに `.await` が付与されている
- video 側 7 直叩き箇所 (`handle_input_sample` 4 / `drain_video_decoder_output` 2 / `discard_video_decoder_output` 1) が decoder task 経路に置換されている
- `Mp4FileReader.video_sender` field が削除され、 `:327, :448` の判定条件が `has_video_track()` ベースに修正されている
- `build_track_senders` が `video_track_id.take()` から `clone()` に変更されている
- warm-up (`suppress_publish=true`) 時の出力 discard 意味論が維持
- SYN/ACK 背圧 (`MAX_NOACKED_COUNT=100`) が decoder あり経路でも有効
- `TrackSender::send_media(sample: MediaFrame) -> bool` が新設されている
- `loop_playback` 5 経路 (推奨案 §3) で decoder task ライフサイクルが期待通り動作 (`reset_for_restart_preserves_timestamp_continuity` を含む既存テストが通る)
- `send_eos_to_tracks` の video 側責任が decoder task に移り、 二重 EOS 送信がない
- `Mp4FileReaderOptions.video_decoder_options: Option<VideoDecoderOptions>` が追加され、 `obsws/source/file_mp4.rs:25-30` の struct literal が更新されている
- `obsws/source/file_mp4.rs:54, :61` の `VideoDecoder::new` + `set_video_decoder(decoder)` が削除
- `set_audio_decoder` 呼出 (`obsws/source/file_mp4.rs:49`) は残っている
- 既存 `Mp4FileReader::tests` は `#[test]` のまま (async 化影響を受ける既存テストなし)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

**準備段階**

1. `Mp4FileReaderOptions` に `video_decoder_options: Option<VideoDecoderOptions>` field 追加。 全 field 明示している `obsws/source/file_mp4.rs:25-30` に `..Default::default()` 追加 (または `video_decoder_options: None` 明示)
2. `TrackSender::send_media(&mut self, sample: MediaFrame) -> bool` を新設
3. `enum DecoderInput { Media(MediaFrame), Eos }` と `struct VideoDecoderTask` / `spawn_video_decoder_task` / `video_decoder_loop` を追加 (未使用のため `#[allow(dead_code)]` で警告抑制)
4. `obsws/source/file_mp4.rs` の Options 構築で `video_decoder_options: Some(VideoDecoderOptions::default())` を設定 (この時点では `set_video_decoder` も呼ばれ続けるので両経路併存)

**移行段階 (同一 commit)**

5. 以下を同時実施:
    - `Mp4FileReader::set_video_decoder` 削除
    - `video_decoder` field を `video_decoder_task: Option<VideoDecoderTask>` に置換
    - `video_sender` field 削除、 `:327, :448` の判定条件修正、 `build_track_senders` の `video_track_id` を `take` から `clone` に変更
    - 5 関数の async fn 化と 18 呼出元への `.await` 付与
    - video 側 7 直叩きを task 経路に置換、 `discard_video_decoder_output` 削除
    - `send_eos_to_tracks` を推奨案どおり修正
    - `Mp4FileReader::run` 内で `handle.config().openh264_lib` を merge して初回 task spawn
    - `obsws/source/file_mp4.rs:54, :61` の `VideoDecoder::new` + `set_video_decoder(decoder)` を削除

**仕上げ段階**

6. `cargo fmt / check / clippy / test` を default + `--no-default-features` の両方で通す

## CHANGES.md について

内部リファクタにつき記載不要。 影響は crate 内 (`obsws/source/file_mp4.rs` 1 ファイル) のみ。

## 残懸念 (実装段階で prototype して確定させる項目)

以下は polish で 1 案に絞りきれず、 実装で試行錯誤しないと最適解が見えない性質のため、 実装段階で確定させる:

1. **`flush_decoders` の意味論**: 現状の「EOS は送らない」契約と `DecoderInput::Eos` 送信の非対称。 (a) `DecoderInput::Flush` を新設 (task 側は flush 後に再開)、 (b) `flush_decoders` を廃止して `send_eos_to_tracks` に統合、 (c) 現状呼出元 2 箇所 (`:339, :350`) はどちらも直後に `send_eos_to_tracks` が呼ばれるので (b) が現実的
2. **`discard_mode` 切替タイミング**: (a) 毎 sample 判定 + 変化時のみ send、 (b) 毎 sample 無条件 send (watch は最新値以外破棄)、 (c) 遷移点のみ send。 audio 側の warm-up 判定との整合も含めて決定
3. **`apply_seek` の TrackSender 再作成 race**: 経路 3 で abort 後の新 task 起動で `TrackSender::new` の初回 SYN 待ちが発生。 warm-up 明けの最初の publish で待ちが顕在化するが、 lazy engine 初期化と併せた実測で許容可能か判定
4. **`Mp4FileReader::drop` 時の task leak 対策**: `Drop` trait 実装で `task.abort()` を呼ぶか、 `Mp4FileReader::run` の末尾で必ず `shutdown().await` を呼ぶかを実装で確定
5. **`AsyncVideoDecoder` の `Send` 実装確認**: 0066 で暗黙前提だが、 `tokio::spawn` で move する時点で Send 要件を明示的に確認 (Nvcodec / VideoToolbox 系 inner の `Send` 実装)

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): 親 issue。 `AsyncVideoDecoder` を導入
- open/0068 (`feature/refactor-migrate-video-decoder-users-to-async`, polished 2026-07-02): 兄弟 issue。 subcommand_inspect + sora の 4 call site 単純置換。 `AsyncVideoDecoder::run` を追加するが本 issue は再利用しない
- open/0072 (`feature/refactor-inbound-endpoint-async-video-decoder`): 兄弟 issue。 inbound endpoint spawn pattern 化。 本 issue の `VideoDecoderTask` struct は 0072 でも参照実装として利用可能 (共通化は 0073 で検討)
- open/0073 (`feature/refactor-remove-sync-video-decoder-and-rename`): 最終クリーンアップ。 本 issue 完了を待つ
- closed/0057 §3: 設計判断の親 issue。 §3 分割表への 0071 / 0072 / 0073 行追加は 0073 完了時にまとめて対応
- audio 側の async 化 (`AsyncAudioDecoder` 追加 + `Mp4FileReader` の audio 経路移行) は本 issue スコープ外。 将来別 issue で扱う
