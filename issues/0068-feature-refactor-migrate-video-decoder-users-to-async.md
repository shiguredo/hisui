# VideoDecoder の全使用側を AsyncVideoDecoder に移行して同期 API を削除する

- Priority: Medium
- Created: 2026-06-29
- Completed:
- Model: Claude Opus 4.7
- Branch: feature/refactor-migrate-video-decoder-users-to-async
- Polished:
- Reporter: @sile

## 目的

open issue 0066 で導入される `AsyncVideoDecoder` への **全使用側の段階的移行と、最終クリーンアップ** (同期 `VideoDecoder` 削除 + `AsyncVideoDecoder` を `VideoDecoder` にリネーム) を扱うフォローアップ issue。

0066 は「`AsyncVideoDecoder` 新規追加 + 既存 `VideoDecoder` を内部 channel ベースに改修 (外部 API は維持)」までを担い、各使用側の `AsyncVideoDecoder` への切り替えと旧 API 削除は本 issue (および必要に応じて本 issue から分割される別 issue 群) で進める。これにより 0066 段階での既存使用側 0 行書き換えを実現する代わりに、本 issue で段階的に 2 系統共存状態を解消する。

## 優先度根拠

Medium。

- 親 issue 0066 で 2 系統共存を許容する方針 (closed/0057 採用案 C の (δ) 派生) を採ったため、本 issue で最終的に 1 系統に収束させる必要がある。 closed/0057 §3 「中途半端な 2 系統共存を残さない」原則と整合させる責務は本 issue にある
- 0066 完了後に着手 (依存)。 0067 (encoder) の同様の移行 issue とは並列実施可能
- 本 issue を着手しないまま放置すると、長期的に「`VideoDecoder` (同期) と `AsyncVideoDecoder` (非同期) のどちらを使うべきか」という API 選択の負債が蓄積する

## 現状

issue 0066 完了直後の状態を前提とする (0066 未完時点では本 issue は着手しない)。0066 完了時点では:

- `AsyncVideoDecoder` が `src/decoder.rs` に新規追加されており、`recv().await` で `Result<VideoFrame>` を受け取れる
- 既存 `VideoDecoder` (同期) は内部に `AsyncVideoDecoder` を保持する wrap 構造に切り替わっており、出力は内部 channel 経由で受け取るが、外部 API (`poll_output()` / `drain_video_decoder_output` / `discard_video_decoder_output`) は挙動不変で全使用側が引き続き同期 pull で動いている
- 各 inner (`Libvpx` / `Openh264` / `Dav1d` / `VideoToolbox` / `Nvcodec`) は同期 fn コンストラクタで `tokio::sync::mpsc::UnboundedSender<crate::Result<VideoFrame>>` を内包する形に変更済み

本 issue で書き換える対象 (9 ファイル + 最終クリーンアップ):

| # | 対象ファイル | 利用パターン |
|---|---------------|--------------|
| 1 | `src/subcommand_inspect.rs` | `VideoDecoder::new` を `MediaPipeline::spawn_processor` 経由で生成、`decoder.run(handle, ...)` を呼ぶ単発 decode |
| 2 | `src/sora/recording_subcommand_compose.rs` | `spawn_processor_task` 経由で生成、 `decoder.run(handle, ...)` |
| 3 | `src/sora/recording_subcommand_vmaf.rs` | 同上 (2 call site) |
| 4 | `src/mp4/reader.rs` | `Mp4FileReader::set_video_decoder` で外部注入された decoder を `handle_input_sample` + `drain_video_decoder_output` 直叩き。`recreate_decoders` (`:1350`) / `flush_decoders` (`:1274`) / `reset_for_restart` (`:1340`) / `apply_seek` (`:638`) の async fn 化を含む大改修。呼出元 15 箇所 (`:339, :350, :378, :383, :465, :475, :479, :484, :494, :498, :503, :519, :523, :528, :645`) の `.await` 付与。`video_sender: TrackSender` (`:1446`、SYN/ACK 背圧 `MAX_NOACKED_COUNT = 100` `:24, :1462-1470`) を decoder task 側で維持 |
| 5 | `src/rtmp/inbound_endpoint.rs` | 構造体 `decoder: Option<VideoDecoder>` (`:249, :266`) を保持、受信ループで `decoder.handle_input_sample(frame)` + `drain_video_decoder_output(decoder, tx)` 直呼出 (`:418, :422`)。Sender 化後は構造体内 `decoder_input_tx: Option<mpsc::Sender<Message>>` + `decoder_join_handle: Option<JoinHandle<crate::Result<()>>>` に置換、受信ループを spawn pattern に再設計 |
| 6 | `src/rtsp/subscriber.rs` | 同上 (構造体 `:64, :238`、`handle_input_sample` `:657`、`drain_video_decoder_output` `:662`) |
| 7 | `src/srt/inbound_endpoint.rs` | 同上 (構造体 `:169, :406`、`handle_input_sample` `:441`、`drain_video_decoder_output` `:445`) |
| 8 | `src/obsws/source/file_mp4.rs` | `VideoDecoder::new` (`:54`) + `reader.set_video_decoder(decoder)` (`:61`) の注入パターン。mp4 reader 改修 (#4) と連動して廃止 |
| 9 | 最終クリーンアップ | 同期 `VideoDecoder` 構造体と関連 API (`poll_output` / `drain_video_decoder_output` / `discard_video_decoder_output` / `Mp4FileReader::set_video_decoder`) をコードベースから完全削除、`AsyncVideoDecoder` を `VideoDecoder` にリネーム |

## 設計方針

着手段階で必要に応じて細分化する (例: mp4 reader と inbound 系を別 issue に切り出す、最終クリーンアップを別 issue にする等)。本 issue 起票時は粒度判断を保留して 1 件にまとめる。

実装着手時の推奨順序は「現状」§の優先順位 (影響範囲の小さい順)。mp4 reader と obsws/file_mp4 は連動するので同時改修する必要あり。inbound 系 (#5-7) は受信ループ構造が似ているため、 1 つを実装すれば他 2 つは類似パターンで進められる想定。

### 移行パターン

```rust
// 旧 (同期 VideoDecoder)
let decoder = VideoDecoder::new(options, stats);
loop {
    let frame = receive_input_frame();
    decoder.handle_input_sample(Some(frame))?;
    while let Some(decoded) = decoder.next_decoded_frame() {
        output_tx.send(decoded);
    }
}

// 新 (AsyncVideoDecoder)
let (decoder_input_tx, decoder_input_rx) = mpsc::channel::<Message>(N);
let mut async_decoder = AsyncVideoDecoder::new(options, stats);
let decoder_join_handle = tokio::spawn(async move {
    while let Some(message) = decoder_input_rx.recv().await {
        async_decoder.handle_input_message(message)?;
        while let Some(result) = async_decoder.next_decoded_frame_async().await {
            output_tx.send(result?);
        }
    }
    Ok::<_, crate::Error>(())
});
// 受信側: decoder_input_tx.send(...).await でフレーム投入、終了時に drop + join_handle.await
```

詳細な型 / シグネチャは 0066 で確定する `AsyncVideoDecoder` の API に従う (本 issue 起票時点では未確定の部分あり、 0066 完了時点で本 issue 本文を必要に応じて補正する)。

### mp4 reader の async fn 化波及

`Mp4FileReader` は同期 pull pattern を多用しているため、Sender 化に伴い大規模な async fn 化が必要:

- `recreate_decoders` (`src/mp4/reader.rs:1350`) → `async fn`
- `flush_decoders` (`:1274`) → `async fn`
- `reset_for_restart` (`:1340`) → `async fn`
- `apply_seek` (`:638`) → `async fn`
- 上記の呼出元 15 箇所すべてに `.await` 付与

加えて、ループ間 (`loop_playback` 再生時) の decoder ライフサイクル管理を「前 decoder task に EOS 投げ → join_handle.await → 新 decoder spawn」のシーケンスに変更。`TrackSender` の SYN/ACK 背圧 (`MAX_NOACKED_COUNT = 100`) は decoder task 側で維持する必要があり、`TrackSender` ごと decoder task に move するか、別の方式で背圧を保つかは実装段階で確定。

### inbound endpoint の構造体改修

RTMP / RTSP / SRT inbound endpoint 3 ファイルは構造が似ているため、 1 つを実装してから他 2 つを類似パターンで進める想定:

- 構造体内 `decoder: Option<VideoDecoder>` → `decoder_input_tx: Option<mpsc::Sender<Message>>` + `decoder_join_handle: Option<JoinHandle>`
- 受信ループ内 `decoder.handle_input_sample(frame)` → `decoder_input_tx.send(Message::Media(frame)).await?`
- 受信ループ内 `drain_video_decoder_output(decoder, tx)` → 不要 (decoder task が直接 `output_tx` に流す)
- 終了時に `decoder_input_tx` drop → `join_handle.await?`
- 受信ループ全体が async 化されているはずなので、追加の async 化波及は少ない (RTMP/RTSP/SRT は元から tokio task で動いている)

### shiguredo-rust 規約整合

- トレイト追加なし
- `#[non_exhaustive]` 不使用
- モック / スタブ不使用 (テストは実 decoder + tokio channel)
- 規約上の許可取得は不要

## 完了条件

- 上記「現状」§の 9 項目すべてが完了している
- 同期 `VideoDecoder` 構造体および関連同期 API がコードベースから完全に削除されている:
  - `VideoDecoder` (旧、`AsyncVideoDecoder` の wrap 型)
  - `VideoDecoder::poll_output()`
  - `drain_video_decoder_output` ヘルパ
  - `discard_video_decoder_output` ヘルパ
  - `Mp4FileReader::set_video_decoder` (`src/mp4/reader.rs:318`)
- `AsyncVideoDecoder` が `VideoDecoder` にリネームされている (最終形)
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo check --workspace --no-default-features`
- `cargo clippy --workspace --all-targets -- --deny warnings`
- `cargo clippy --workspace --no-default-features -- --deny warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`

すべて通る。

実装規模が想定 (推定 1500-2500 行) を 1.5 倍以上超える場合は、 mp4 reader 系 / inbound 系 / 最終クリーンアップ等への分割を検討する (Decision Owner = `@sile` が判断)。

## 解決方法

実装着手時の推奨手順:

1. `src/subcommand_inspect.rs` を `AsyncVideoDecoder` に移行する (最小影響範囲、 pattern 確立)
2. `src/sora/recording_subcommand_compose.rs` を移行する
3. `src/sora/recording_subcommand_vmaf.rs` を移行する (2 call site)
4. `src/mp4/reader.rs` を `AsyncVideoDecoder` に移行する (関数 4 つの async fn 化 + 呼出元 15 箇所追従 + `TrackSender` 移譲 + ライフサイクル管理改修)
5. `src/obsws/source/file_mp4.rs` を移行する (mp4 reader 改修と連動、`set_video_decoder` 廃止)
6. `src/rtmp/inbound_endpoint.rs` を移行する (構造体改修 + 受信ループ spawn pattern 化)
7. `src/rtsp/subscriber.rs` を移行する (6 と同じ pattern)
8. `src/srt/inbound_endpoint.rs` を移行する (6 と同じ pattern)
9. 最終クリーンアップ: 同期 `VideoDecoder` 関連 API 削除 + `AsyncVideoDecoder` を `VideoDecoder` にリネーム
10. `cargo fmt` / `cargo check` (default + `--no-default-features`) / `cargo clippy` / `cargo test` 全通過確認

各 step ごとに `cargo check` を通せる構造で進める (中間状態でビルドが通らないと debug が困難になるため)。

## CHANGES.md について

内部リファクタにつき記載不要。`VideoDecoder` 系は library として外部公開していないため、API 変更の後方互換影響は obsws coordinator / mixer / writer / subcommand 階層等の crate 内利用箇所のみ。

## 関連

- open/0066 (`feature/refactor-add-async-video-decoder`): 親 issue。本 issue の前提となる `AsyncVideoDecoder` 新規追加と既存 `VideoDecoder` 内部 channel 化を行う。本 issue は 0066 完了後に着手する
- open/0067 (`feature/refactor-video-encoder-sender-interface`): encoder 側。同じ (δ) 方針で encoder にも `AsyncVideoEncoder` (or `AsyncAudioEncoder`) を追加する想定。本 issue と並行 or 後続で対応 (encoder 側も同様の使用側移行 issue が必要になる見込み)
- closed/0057 §3: 設計検討の親 issue。採用案 C で「中途半端な 2 系統共存を残さない」と禁じた状態を、 0066 で意図的に許容し、本 issue で最終的に 1 系統に収束させて 0057 §3 と整合させる
