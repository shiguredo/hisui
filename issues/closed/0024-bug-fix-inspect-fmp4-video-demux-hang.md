# inspect が映像トラックを含む fMP4 を demux できず、エラー後にハングするのを直す

- Priority: High
- Created: 2026-06-04
- Completed: 2026-06-05
- Model: Opus 4.8
- Branch: feature/fix-inspect-fmp4-video-demux-hang
- Polished:

## 目的

`hisui inspect --decode <archive>.mp4` が、映像トラックを含むフラグメント MP4 (fMP4) を demux できず失敗し、さらにそのエラー後にプロセスが正常終了せずハングする問題を直す。

inspect は fMP4 入力をサポートする前提で実装されている (`src/mp4/sample_reader.rs` は通常 MP4 / fMP4 の両方を `Mp4Demuxer` で前方読みする) が、映像トラックを含む fMP4 では実際には動作しない。映像 fMP4 で inspect が全く使えず、かつハングにより CI 等での原因特定が困難になるため、実害が大きい。

なお、本 issue には独立した 2 つの不具合が含まれる。両者を切り分けて扱うこと。

1. demux 不具合: 映像トラックを含む fMP4 の demux が失敗する
2. ハング不具合: 処理失敗時に inspect プロセスが終了せずハングする

## 優先度根拠

High。

- 映像トラックを含む fMP4 に対して inspect が完全に機能しない (H264 / VP9 のいずれでも失敗)
- 失敗時にハングするため、終了コードでの失敗検知ができず、CI ではタイムアウトで SIGKILL されるまで待たされる
- ハング不具合は inspect 以外の処理失敗時にも同じ経路で発生しうる汎用的な欠陥である (後述)

## 現状

### 再現

```
hisui inspect --decode <archive>.mp4 --openh264 libopenh264.so
```

映像トラックを含む fMP4 に対して以下のエラーログを出力する。

```
[ERROR] hisui::media_pipeline - failed to run processor mp4_file_reader:
  Demux error <archive>.mp4: Failed to decode MP4 box:
  InvalidData: sample data range exceeds mdat boundary
```

「sample data range exceeds mdat boundary」というメッセージ本体は依存ライブラリ `shiguredo_mp4` (`=2026.3.0`, `Cargo.toml:28`) の fMP4 セグメント demux 内部から発生し、`Mp4Demuxer::next_sample()` が `DemuxError` を `Error` に変換して返している (`src/mp4/demuxer.rs:151-167`)。さらにエラー出力後にプロセスがハングする。

### 切り分け結果 (映像トラックの有無で結果が分かれる)

| 音声 | 映像 | 結果 |
|---|---|---|
| OPUS | なし | PASS |
| なし | H264 | FAIL (demux error + hang) |
| OPUS | VP9 | FAIL (demux error + hang) |

- 音声のみの fMP4 は成功する。映像トラックを含む fMP4 は H264 / VP9 のいずれでも失敗する。
- コーデックに依存せず映像トラックの有無で再現するため、コーデックデコードではなくフラグメント MP4 のセグメント demux (mdat 境界判定) の問題と考えられる。
- 非フラグメントの通常 MP4 や WebM は同条件で成功する。

### 不具合 1: demux 失敗

`Mp4Demuxer` は fMP4 を `Fmp4FileDemuxer` で前方読みし、demuxer が要求する入力範囲 (`RequiredInput`) を `read_required_range()` でファイルから供給する仕組みである (`src/mp4/demuxer.rs:151-178`)。

調査の観点 (本 issue 内で確定させること):

- `read_required_range()` (`src/mp4/file_kind.rs:64-97`) は `required.size` が `Some` のとき読み込み範囲を `start.saturating_add(size).min(file_size)` とファイルサイズで clamp している。demuxer が要求した範囲よりも実際に供給したバイト数が少ない場合に、demuxer 側の mdat 境界判定が破綻する可能性がある。供給範囲が要求を満たせているか確認すること。
- 入力 fMP4 が実際に不正 (mdat 境界が壊れている) なのか、それとも `shiguredo_mp4` / hisui のパース・入力供給が誤っているのかの切り分けが必要。同じ fMP4 を他のツールが正しく demux できるかも判断材料にする。
- 原因が `shiguredo_mp4` 内部にあると確定した場合は、`shiguredo_mp4` 側での修正と、修正版へのバージョン更新で対応する。hisui 側の入力供給バグであれば `read_required_range()` / `Mp4Demuxer` を修正する。

### 不具合 2: 処理失敗時のハング

demux 失敗そのものとは独立に、processor が `Err` を返したときに inspect がハングする。原因は以下の経路にある。

- `spawn_processor()` は processor の future が `Err` を返すと `error_flag` を立ててログ出力するだけで、その後 `ProcessorHandle` (および publish していた `TrackPublisher`) が drop される (`src/media_pipeline.rs:596-604`)。
- `TrackPublisher::drop` は subscriber に `Message::Eos` を送らず、subscriber を pipeline へ返却するだけである (`src/media_pipeline.rs:1179-1196`)。
- inspect の `OutputPrinter::run` は購読する各トラックが `Message::Eos` を受信して `active_streams` が空になるまで `tokio::select!` ループを回し続ける (`src/subcommand_inspect.rs:442-461`)。reader 失敗時はどのトラックにも EOS が届かないため、このループが永久にブロックする。
- `Mp4SampleReader::run` も正常終了時にしか `send_eos()` を呼ばない。途中エラーで `?` リターンすると EOS を送らずに抜ける (`src/mp4/sample_reader.rs:79-155`)。

このため、reader が demux エラーで落ちても output_printer は EOS を待ち続け、プロセス全体が終了しない。これは inspect 固有ではなく、「上流 processor が異常終了したとき、下流の購読側が EOS を受け取れずブロックしうる」という pipeline の汎用的な欠陥である。

### 再現用 fMP4 ファイル

調査時にローカルで利用できる再現用アーカイブを別途用意している (リポジトリにはコミットしない)。

- 映像 (H264) を含み失敗するもの
- 映像 (VP9) を含み失敗するもの
- 音声 (OPUS) のみで成功するもの

## 設計方針

不具合 1 と不具合 2 を独立に修正する。

### 不具合 1 (demux 失敗)

- まず入力 fMP4 が正しいか、`shiguredo_mp4` / hisui の入力供給が誤っているかを切り分ける。
- hisui 側の入力供給バグ (`read_required_range()` の clamp 等で要求範囲を満たせていない) であれば hisui 内で修正する。要求範囲がファイル末尾を超える場合の扱い (エラーにするか、不足を検出して明示エラーにするか) を確定する。
- `shiguredo_mp4` 内部のバグであれば、`shiguredo_mp4` 側を修正し、hisui の依存バージョンを修正版へ更新する (バージョンはマイナーまで指定する規約に従う)。
- いずれの場合も、映像トラックを含む fMP4 を inspect できることを検証可能にする。

### 不具合 2 (ハング)

- processor が異常終了した場合に、その processor が publish していたトラックの購読側へ確実に終了が伝わるようにする。candidate は次のいずれか (本 issue 内で確定させること):
  - `TrackPublisher::drop` 時に、まだ EOS / 正常完了していなければ subscriber へ `Message::Eos` (またはエラーを表す終了メッセージ) を送る。
  - processor 異常終了 (`error_flag` セット) を pipeline が検知して、購読側のループを終了させる。
- inspect 固有ではなく pipeline 全体の問題として直す。回避策として inspect 側だけにタイムアウトを足すようなその場しのぎは採らない。
- 異常終了時はプロセスが非ゼロ終了コードで終わることが望ましい。`run_internal` は現状 pipeline 完了後に常に `Ok(())` を返す (`src/subcommand_inspect.rs:74-108`) ため、processor の `error_flag` を参照して終了コードに反映する方針も併せて検討する。

## 完了条件

- 映像トラック (H264 / VP9) を含む fMP4 に対して `hisui inspect --decode` が demux エラーを出さずに完走し、音声・映像のコーデックとサンプル情報を正しく出力する。
- 音声のみ fMP4、通常 MP4、WebM の inspect が引き続き成功する (リグレッションなし)。
- いずれかの processor が異常終了した場合に inspect プロセスがハングせず、有限時間で終了する。異常終了時は非ゼロ終了コードを返す。
- 上記を検証するテストを追加する。切り詰めフラグメントの許容・過剰許容防止 (初期化中破損はエラー)・異常終了時の終了パス (EOS 伝播・終了コード) をカバーする。モック・スタブは使わない。
  - NOTE: 本プロジェクトには PBT 基盤 (`pbt/`) が無く、対象も `pub(crate)` / private のため、いずれも `src/` 内の in-file 単体テストで対応した。
- 原因が `shiguredo_mp4` 側であった場合は、依存バージョンの更新を `CHANGES.md` に記載する。
  - NOTE: 真因は入力ファイルの切り詰めであり (後述「解決方法」)、`shiguredo_mp4` の修正・依存更新は不要だった。

## 解決方法 (2026-06-05)

### 真因 (当初の仮説とは異なる)

当初は hisui の入力供給 (`read_required_range()` の clamp) や `shiguredo_mp4` のパースバグを疑ったが、調査の結果、真因は **入力 fMP4 の末尾付近のフラグメントが途中までしか書かれていない (録画プロセスのクラッシュ等による切り詰め)** ことだった。`trun` が宣言するサンプルデータ量が実際の `mdat` に収まらず、`shiguredo_mp4` が `sample data range exceeds mdat boundary` を返すのは正しい挙動であり、hisui / `shiguredo_mp4` のパースバグではなかった。

再現ファイルでの確認:

- H264: 末尾の映像フラグメントの `trun` が宣言する 118955 バイトに対し `mdat` は 6840 バイトのみ
- VP9: 末尾の映像フラグメントが同様に切り詰め (その後ろに正常な音声フラグメントが続くため、ファイル末尾の box とは限らない)

「映像トラックの有無で結果が分かれた」のは、これらのアーカイブで切り詰められていたのが映像フラグメントだったことによる観測であり、コーデックやトラック種別そのものが原因ではない。

### 不具合 1 (demux 失敗) の対応 — fMP4 のベストエフォート読み取り

`src/mp4/demuxer.rs` の `Mp4Demuxer::next_sample()` で、フラグメント (moof + mdat) の処理に失敗した際、失敗位置 (`last_supply_offset`) が以下のいずれかなら、原因を問わずエラーにせず、そこまでに読めたサンプルを返してストリーム終端 (`Ok(None)`) として扱うようにした。

- 構造は揃っているメディアフラグメント (`is_media_fragment`)
- ボックスが EOF で途切れている (`is_truncated_box_at_eof`: ヘッダが読めない / 宣言サイズがファイル末尾を超える)

moov / ftyp など初期化中の破損は引き続きエラーにする (過剰許容を避ける)。検知時は warn ログを出力する。ボックスヘッダの読み取りは自前実装をやめ `shiguredo_mp4::BoxHeader::decode` に委譲した。

なお「壊れたフラグメントの細かい原因別ハンドリング」は壊れやすいため行わず、検知したらそこで読み取りを止める方針とした。将来、破損を一切許容しない厳密モードが必要になればオプションとして追加を検討する。

### 不具合 2 (ハング) の対応

- `TrackPublisher` に `eos_sent` を持たせ (EOS 送出は `send()` 内で一元的に記録)、Drop 時に EOS 未送信かつ再 publish 待ち (`TrackState::marked_for_republish`、`unpublish_track` 由来) でもない場合は subscriber を閉じる (`drain_returned_subscribers`)。購読側の `recv()` が EOS を返し、下流のハングを防ぐ。`unpublish_track` による再 publish 待ちは従来通り subscriber を保持する (OBSWS の republish に影響しない)。
- `MediaPipeline::run()` が「いずれかの processor が異常終了したか」を `bool` で返すようにし、inspect はこれを終了コードに反映する (異常時は非ゼロ終了)。パイプライン全体の異常終了フラグは `processor_failed` という名前にし、`ProcessorHandle` の `error_flag` (個別メトリクス) との衝突を解消した。

### テスト

プロジェクトに PBT 基盤が無く、対象も内部可視性のため、いずれも in-file 単体テストで対応した (`src/mp4/demuxer.rs`, `src/media_pipeline.rs`)。モック / スタブは不使用。

- 切り詰め許容: 末尾フラグメント / mdat ヘッダ / moof ヘッダ / moof 本体の各切り詰めケース
- 過剰許容防止: moov 破損はエラーになること、`is_media_fragment` が moof と初期化中ボックスを区別すること
- ハング修正: publisher が EOS 未送信で異常終了した際に購読側が EOS を受け取り、`run()` が異常終了を報告すること

### 変更履歴

`CHANGES.md` に `[FIX]` 2 件 (フラグメント切り詰めの許容 / processor 異常終了時のハング回避と非ゼロ終了) として記載した。

## 関連

- issues/0020 (inspect の fMP4 テスト整合性)、issues/0023 (inspect の format での fMP4 区別) と同じ inspect / fMP4 領域。本 issue の demux 不具合が解決しないと、これらの fMP4 inspect の前提が成立しない。
