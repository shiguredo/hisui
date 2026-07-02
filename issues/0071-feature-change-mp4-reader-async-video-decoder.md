# Mp4FileReader を AsyncVideoDecoder に移行して mp4 reader の関数 4 つを async fn 化する

- Priority: Medium
- Created: 2026-07-02
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/change-mp4-reader-async-video-decoder
- Polished:

## 目的

closed issue 0066 で `AsyncVideoDecoder` が追加され、 同期 `VideoDecoder` は wrap 構造で挙動維持されている状態。 本 issue は使用側移行のうち `src/mp4/reader.rs` と、 その `set_video_decoder` を経由して decoder を注入する `src/obsws/source/file_mp4.rs` を `AsyncVideoDecoder` ベースに切り替える。

これに伴い以下が発生する:

- `Mp4FileReader` の 4 関数 (`flush_decoders` / `reset_for_restart` / `apply_seek` / `recreate_decoders`) の async fn 化
- 呼出元 16 箇所への `.await` 追従 (open issue 0068 起票時の見積 15 箇所から再カウント済み)
- `TrackSender` (SYN/ACK 背圧 `MAX_NOACKED_COUNT=100`) の decoder task への移譲、 または `TrackSender` は main task 側で維持しつつ decoder task から `TrackPublisher::send_media` する形態のどちらかを確定
- warm-up 経路 (`discard_video_decoder_output`) の意味論を保った再設計
- `loop_playback` 時の decoder ライフサイクル (loop 継続 / `MediaLoopAction::Restart` / Seek / `reset_for_restart` の 4 経路) 管理の確定
- `Mp4FileReader::set_video_decoder` (`:318`) 廃止 + `Mp4FileReaderOptions` への decoder 生成情報吸収

## 優先度根拠

Medium。

- closed issue 0066 で採用された「wrap 段階的移行方針 (δ)」を、 closed issue 0057 §3 「中途半端な 2 系統共存を残さない」原則と整合させるには最終的に全使用側の移行が必須。 本 issue はその最重量部分
- 本 issue 単独では外部挙動 (再生タイミング / 出力) は不変。 内部リファクタ相当で緊急性はないが、 open issue 0072 (inbound endpoint spawn pattern 化) / open issue 0073 (最終クリーンアップ) の後続作業が本 issue の完了を待つ
- 実装難所 (async 化波及、 背圧移譲、 warm-up 経路、 ループライフサイクル) が複数集中しており、 open issue 0068 に同居させると polish しきれない事情から分割された

## 現状

`src/mp4/reader.rs` (2336 行) は同期 pull pattern で `VideoDecoder` を扱っており、 本 issue で書き換える対象箇所は以下:

### 同期関数の async fn 化 (4 関数)

| 関数 | 定義位置 | 直接の呼出元 |
|------|----------|---------------|
| `flush_decoders` | `:1274` 付近 | `:339, :350` (2 箇所) |
| `reset_for_restart` | `:1340` 付近 | `:378, :383` (2 箇所) |
| `apply_seek` | `:638` 付近 | `:465, :479, :484, :498, :503, :523, :528` (7 箇所) |
| `recreate_decoders` | `:1350` 付近 | `:475, :494, :519, :645, :1346` (5 箇所、 `:1346` は `reset_for_restart` の内部呼出) |

合計 16 呼出元すべてに `.await` を付与する。 `run` / `run_loop` は既に async fn なので自然に伝播する。 `recreate_decoders` の内部呼出 (`:645` = `apply_seek` 内、 `:1346` = `reset_for_restart` 内) は関数側 async 化で自動的に awaitable になる。

### VideoDecoder 直叩き箇所

| API | 呼出位置 | 個数 |
|-----|----------|------|
| `decoder.handle_input_sample(Some(...))` | `:1195, :1233, :1279, :1285` | 4 箇所 |
| `crate::decoder::drain_video_decoder_output(decoder, ...)` | `:1236, :1286` | 2 箇所 |
| `discard_video_decoder_output(decoder)` | `:1199` | 1 箇所 (定義は `:1388` の module-private helper) |

`drain_video_decoder_output` は `AsyncVideoDecoder` 側へ移行するため直接呼出は消える (decoder task が `output_tx` へ直接流すか、 non-blocking で `poll_output` を回す形に置換)。 `discard_video_decoder_output` は warm-up 経路の意味論 (`suppress_publish=true` 中の出力を捨てる) を保った上で mp4/reader.rs 内で完結して削除される。

### TrackSender SYN/ACK 背圧

- `MAX_NOACKED_COUNT: u64 = 100` (`:24`)
- `struct TrackSender` 定義 (`:1446` 付近)
- `if self.noacked_sent > MAX_NOACKED_COUNT` の ACK 待ちロジック (`:1463`)
- `Mp4FileReader.video_sender: Option<TrackSender>` field 宣言 (`:221`)
- `send_eos_to_tracks` (`:1292` 付近) が `video_sender.send_eos()` を呼ぶ

decoder あり経路 (`drain_video_decoder_output(decoder, &mut sender.sender)` `:1236, :1286`) では `sender.sender` (= `TrackPublisher`) を直接渡していて `TrackSender::send_video` の `prepare_send().await` はバイパスされ、 現状 SYN/ACK 背圧が効いていない。 decoder なし経路の `sender.send_video` 経路 (`:1241` 付近) は背圧あり。 本 issue で decoder あり経路も背圧を有効化するか、 現状の非対称を温存するかを設計方針で確定する。

### loop_playback ライフサイクル

`Mp4FileReader::run_loop` (`:465-533` 付近) 内で `recreate_decoders` を呼ぶ経路は 4 種類:

1. `run_loop` 内側の EOF 到達で loop 継続時: 現状は decoder を再生成しない (継続)
2. `MediaLoopAction::Restart` (`:475, :494, :519`): `recreate_decoders` を明示的に呼出
3. `MediaLoopAction::Seek` / `OffsetSeek` (`:479, :484, :498, :503, :523, :528`): `apply_seek` 経由で `recreate_decoders` (`:645`)
4. `wait_for_restart_command` 経路 (`:378, :383`): `reset_for_restart` 経由で `recreate_decoders` (`:1346`)

現状の同期実装は `recreate_decoders` (`:1350`) 内で `self.video_decoder = Some(decoder)` と単純上書きし、 前 decoder は暗黙 drop で終了する。 spawn pattern 化後の前 decoder task の始末 (継続使い回し / EOS 送信 + `JoinHandle::await` + 新規 spawn / abort) を経路ごとに確定する。

### set_video_decoder 廃止と obsws/source/file_mp4.rs 連動

- `Mp4FileReader::set_video_decoder(&mut self, decoder: crate::decoder::VideoDecoder)` (`:318`) は同期 fn
- 現状の呼出元は `src/obsws/source/file_mp4.rs:61` の 1 箇所のみ (`:54` で `VideoDecoder::new` してから `reader.set_video_decoder(decoder)`)
- 本 issue で `set_video_decoder` を削除する場合、 `Mp4FileReaderOptions` に `decoder_options: Option<VideoDecoderOptions>` / `openh264_lib: Option<Openh264Library>` / `enable_video_decoder: bool` のいずれか等を追加して decoder 生成情報を吸収する必要がある

### AsyncVideoDecoder の現状 API

`src/decoder.rs` (0066 完了時点) が提供する API:

- `pub struct AsyncVideoDecoder` (`:385`)
- `pub fn AsyncVideoDecoder::new(options, stats) -> Self` (`:400`)
- `pub fn handle_input_sample_sync(&mut self, Option<MediaFrame>) -> Result<()>` (`:424`)
- `pub fn poll_output_sync(&mut self) -> Result<DecoderRunOutput>` (`:441`)
- `pub async fn next_decoded_frame_async(&mut self) -> Option<crate::Result<VideoFrame>>` (`:472`)

`handle_input_message` (Message enum の dispatch) と `run` (`ProcessorHandle` ベースの実行ループ) は同期 wrap の `VideoDecoder` にのみ存在し、 `AsyncVideoDecoder` 側には未実装。 本 issue の spawn pattern 実装で必要になれば、 spawn クロージャ内で自前で `Message::Media / Eos / Syn(_)` を `handle_input_sample_sync(Some/None/() ) ` に dispatch する形で構築する (もしくは open issue 0068 で `AsyncVideoDecoder::handle_input_message` / `AsyncVideoDecoder::run` の追加を先行させる可能性あり。 open issue 0068 の polish で確定させる)。

### 既存テスト影響

- `src/mp4/reader.rs` 内の `#[cfg(test)] mod tests` (`:1785-2106` 付近) に `reset_for_restart_preserves_timestamp_continuity` などの回帰テストがあり、 async fn 化に伴い `#[test]` → `#[tokio::test]` への昇格と `.await` 追記が必要
- `ProcessorHandle` を渡す関数のテスト内利用 (現状はコメントで「`ProcessorHandle` なしで呼べない」旨が記載されている箇所あり) は、 モック / スタブ禁止規約下で実 pipeline 起動を要する。 テスト内での `ProcessorHandle` 準備コストが実装コストに反映される点に留意

## 設計方針

### 未確定論点 (polish で確定させる)

以下は本 issue 起票時点で意図的に選択肢を残している。 `/polish-issue 71` 段階で 1 案に絞り込む:

1. **warm-up 経路 (`discard_video_decoder_output`) の再設計方針**
    - A: decoder task に flush モード制御チャネル追加 (`suppress_publish` を伝えて出力を task 内で捨てる)
    - B: warm-up 中は decoder task を落とし、 warm-up 明けに再起動 (task 再生成コスト増)
    - C: main task 側で出力先を切り替える (task から常に流し、 main が publish するか捨てるかを選ぶ)
    - D: warm-up 中は decoder に入力せず demuxer 側でフレームを捨てる (デコーダー内部状態が回復しないため keyframe まで待つ必要がある)
2. **`TrackSender` (SYN/ACK 背圧) の移譲先**
    - a: main task 側で維持 (`sender.send_video(...)` を main が呼ぶ、 decoder task は `TrackPublisher` を持たず main への channel だけ持つ)
    - b: decoder task 側に move (task 内で `sender.send_video(...).await` を呼ぶ、 main task は `TrackSender` を持たない)
    - decoder あり経路で現状効いていない背圧を本 issue で有効化するか温存するかも同時に確定
3. **`loop_playback` 4 経路別 decoder task ライフサイクル**
    - 経路 1 (loop 継続): 継続使い回し (現状挙動)
    - 経路 2-4 (Restart / Seek / reset_for_restart): 前 task へ EOS 送信 → `JoinHandle::await` → 新 spawn の順序を確定
    - タイムスタンプ連続性 (`reset_for_restart_preserves_timestamp_continuity` テスト) を担保する order を明示
4. **`set_video_decoder` 廃止後の options 注入方式**
    - `Mp4FileReaderOptions` に何を追加するか (`decoder_options` / `openh264_lib` / `enable_video_decoder` bool flag の組合せ)
    - `src/obsws/source/file_mp4.rs:54-61` の呼出構造をどう置換するか
5. **decoder task 入力 channel の bounded/unbounded と型**
    - `tokio::sync::mpsc::channel::<Message>(N)` の `N` (bounded 採用時) を何にするか、 `unbounded_channel` を使うか
    - decoder 内部 channel (0066 で unbounded 確定) との整合。 main → decoder task 間の背圧をどう定義するか
    - `Message` (`crate::Message`、 `Media / Eos / Syn`) をそのまま流すか、 decoder 専用の enum (`Media / Eos` のみ) を新設するか

### 決定事項 (polish で覆さない前提)

- `AsyncVideoDecoder` は 0066 で導入済みのものを利用 (再設計しない)
- 各 inner (`Libvpx / Openh264 / Dav1d / VideoToolbox / Nvcodec`) は 0066 で `OutputSink` (`UnboundedSender<crate::Result<VideoFrame>>` + `total_output_metric: StatsCounter` のペアリング構造体) 内包に統一済み
- decoder 内部 channel は unbounded (0066 確定)
- decoder ライフサイクルは spawn pattern (`tokio::spawn(async move { ... })`) で管理する。 `AsyncVideoDecoder` を main task 内で `.await` 直呼出はしない (mp4 reader 全体の pull ループの block を避けるため)

### shiguredo-rust 規約整合

- モック / スタブ不使用 (テストは実 decoder + tokio channel + 実 `ProcessorHandle`)
- `#[non_exhaustive]` 不使用
- 新規 trait 追加なし
- 既存の error 型 (`crate::Error`) を維持

## 完了条件

- `Mp4FileReader::set_video_decoder` (`:318`) が削除されている
- `Mp4FileReader` の 4 関数 (`flush_decoders` / `reset_for_restart` / `apply_seek` / `recreate_decoders`) が async fn 化されている
- 上記 4 関数への 16 呼出元すべてに `.await` が付与されている
- `handle_input_sample` 4 箇所 / `drain_video_decoder_output` 2 箇所 / `discard_video_decoder_output` 1 箇所の VideoDecoder 直叩きが、 `AsyncVideoDecoder` ベースの spawn pattern 経路に置換されている
- warm-up (`suppress_publish=true`) 時の decoder 出力 discard 意味論が維持されている (回帰テストで確認)
- SYN/ACK 背圧の扱いが設計方針 §2 で確定した通りに実装されている
- `loop_playback` の 4 経路すべてで decoder のライフサイクルが期待どおり動作 (`reset_for_restart_preserves_timestamp_continuity` を含む既存テストが通る)
- `src/obsws/source/file_mp4.rs` の `set_video_decoder` 呼出が消え、 `Mp4FileReaderOptions` 経由の注入に置換されている
- `Mp4FileReader::tests` の async 化追従 (`#[tokio::test]` 化と `.await` 付与) が完了している
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

## 解決方法

実装着手時の推奨手順 (詳細は polish で確定):

1. 設計方針 §「未確定論点」の 5 論点を実装着手前に polish で確定させる
2. `Mp4FileReaderOptions` を拡張 (`set_video_decoder` 廃止後の注入先を確保)
3. `recreate_decoders` を async fn 化して spawn pattern を導入 (decoder task の生成 / join / EOS 送信のヘルパを合わせて実装)
4. `apply_seek` / `flush_decoders` / `reset_for_restart` を async fn 化し、 呼出元 16 箇所へ `.await` を付与
5. `handle_input_sample` / `drain_video_decoder_output` / `discard_video_decoder_output` 各呼出を decoder task 経路に置換 (warm-up 経路の意味論も同時に反映)
6. `TrackSender` SYN/ACK 背圧を §「未確定論点」§2 の確定案に沿って移譲 or 温存
7. `src/obsws/source/file_mp4.rs` の呼出を `Mp4FileReaderOptions` 経由に置換
8. `Mp4FileReader::tests` を async 化して回帰テストを走らせる
9. `cargo fmt` / `cargo check` (default + `--no-default-features`) / `cargo clippy` / `cargo test` を完了条件全項目で通す

各 step で `cargo check` を通せる中間状態を保つ (`AsyncVideoDecoder::handle_input_message` / `run` が未実装なら、 spawn クロージャ内で `Message` を自前 dispatch する形で回避する)。

## CHANGES.md について

内部リファクタにつき記載不要。 `Mp4FileReader` は library として外部公開していない (hisui は bin crate)。 API 変更の影響は crate 内利用箇所 (obsws / mixer / writer / subcommand 階層) のみ。

## 関連

- closed/0066 (`feature/refactor-add-async-video-decoder`): 親 issue。 `AsyncVideoDecoder` を導入し `VideoDecoder` を wrap 構造に切り替えた
- open/0068 (`feature/refactor-migrate-video-decoder-users-to-async`): 兄弟 issue。 `src/subcommand_inspect.rs` / `src/sora/recording_subcommand_compose.rs` / `src/sora/recording_subcommand_vmaf.rs` の単純 call site 置換 3 ファイルを扱う。 0068 の polish 過程で本 issue が分離された
- open/0072 予定: RTMP / RTSP / SRT inbound endpoint (`src/rtmp/inbound_endpoint.rs` / `src/rtsp/subscriber.rs` / `src/srt/inbound_endpoint.rs`) の spawn pattern 化。 本 issue と互いに独立
- open/0073 予定: 最終クリーンアップ (同期 `VideoDecoder` 削除 + `AsyncVideoDecoder` を `VideoDecoder` にリネーム)。 0068 / 0071 / 0072 の全完了を待つ
- closed/0057 §3 (`feature/refactor-callback-friendly-codec-interface`): 設計判断の親 issue。 採用案 C 「中途半端な 2 系統共存を残さない」原則との整合は 0073 で最終達成される
