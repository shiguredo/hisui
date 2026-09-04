# inspect --decode のデコード結果対応付けが timestamp 非依存で欠落を誤読しうる

- Created: 2026-09-04
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-inspect-decode-sample-correspondence
- Polished: {YYYY-MM-DD}

## 目的

`hisui inspect --decode` の JSON で、`decoded_data_size` が無いサンプルを「その入力サンプルがデコードできなかった」と解釈できない状態を解消する。デコード出力とエンコード済みサンプルの対応を timestamp で行い、欠落の意味を一意に読めるようにする。

本 issue は、特定の MP4 が壊れていることや、VideoToolbox が特定フレームを捨てていることを前提にしない。確認できているのは、現行の対応付けではデコード出力の欠落と入力サンプルの成否を区別できないことである。

## 現状

### 対応付け

`src/subcommand_inspect.rs` の `OutputPrinter` は、デコード出力を timestamp では紐付けない。

- `handle_video_decoded_sample` が受け取るのは画素サイズだけ (`DecodedVideoInfo` に timestamp が無い)
- `try_apply_pending_video_decoded_infos` は、まだ `decoded_data_size` が無い先頭の `video_samples` 要素へ FIFO で載せる
- 音声も `try_apply_pending_audio_decoded_data_sizes` が同じ先頭未設定割り当て

そのため、デコード出力が 1 枚でも欠けると、欠けた位置より後のサンプルに別フレームの `decoded_data_size` / `width` / `height` が乗る。JSON 上でフィールドが無いサンプルは、実際には「まだ割り当てられていない末尾側」であり、入力サンプルそのもののデコード失敗を指すとは限らない。

この FIFO が最後まで走り切った場合、`decoded_data_size` が付くのはサンプル列の先頭側である。途中 1 枚の出力欠落だけなら、未設定になるのは末尾側であり、先頭サンプルだけが未設定で中間がすべて設定済み、という形にはならない。

### デコード出力数が入力と 1:1 にならない経路

macOS の H.264 / H.265 は `src/decoder.rs` の `VideoDecoderInner` が VideoToolbox を選ぶ。

- `src/decoder/video_toolbox.rs` の `VideoToolboxDecoder::decode` は `inner.decode` が `None` なら成功扱いで出力しない
- 出力時の timestamp は、その呼び出しの入力 `frame.to_stripped()` を使う。`src/decoder/libvpx.rs` の `LibvpxDecoder` や `src/decoder/openh264.rs` の `Openh264Decoder` のような `input_queue` は持たない
- `VideoDecoderInner::finish` は VideoToolbox では no-op である。Libvpx / OpenH264 / Dav1d / (feature 有効時) nvcodec は `finish` で残フレームを出す

これらは「入力数と出力数がずれうる」という事実であり、観測した欠落の原因がここだと特定してはいない。VideoToolbox の `finish` 実装や `None` の扱い自体の修正は本 issue の完了条件に含めない。対応付けを直したあと、再現資産が揃った時点で別途判断する。

`src/subcommand_inspect.rs` の `estimate_duration` は隣接サンプルの timestamp 差であり、MP4 の sample duration そのものではない。末尾サンプルは次が無いので `duration` は `None` のまま出る。JSON の `duration_us` とコンテナ上の duration を同一視しないこと。

### 観測 (原因は未確定)

macOS で H.265 (`hev1`) の MP4 を `inspect --decode` したとき、稀に次の JSON になることがあった。

- 先頭キーフレームと末尾サンプルだけ `decoded_data_size` / `width` / `height` が無い
- 中間サンプルはすべて付いている

失敗した MP4 と生 JSON は残していない。同じ構成で作った CRA 始まりの対照クリップは、hisui と ffmpeg の両方で全サンプルを復号できた。CRA 始まりは失敗の十分条件ではない。

この JSON は、VideoToolbox が 1 枚だけ出力しなかった場合の FIFO 割り当てでは説明できない。原因は VideoToolbox、hisui の対応付けや終了処理、MP4、ビットストリームのいずれかにまだ落ちていない。入力サンプルが壊れている証拠も、今回の観測からは得られていない。確認できたのは、hisui が一部のデコード出力を JSON に載せられなかったことだけである。

closed の 0092 は、解像度変更後の後半サンプルで `decoded_data_size` が欠ける silent drop だった。後半がまとめて未設定なら FIFO と両立する。本観測の「先頭と末尾だけ未設定」とはパターンが異なる。

## 設計方針

- `OutputPrinter` がデコード出力の timestamp を保持し、同じ timestamp のエンコード済みサンプルへ `decoded_data_size` / `width` / `height` を載せる
- デコード出力が無い timestamp のサンプルは未設定のまま残す。後続サンプルへ繰り下げない
- 音声も同じ対応付けにする (`OutputPrinter` 内の同一問題のため)
- デコーダー側の timestamp 保持 (`input_queue`) や VideoToolbox の `finish` は、対応付け修正後に必要なら別 issue にする。本 issue で混ぜない

## 完了条件

- JSON 上で `decoded_data_size` が無いことは、その timestamp のデコード出力が無かったことを意味する
- デコード出力が途中 1 件欠けても、欠けていない timestamp のサンプルに誤った `decoded_data_size` が乗らない
- 上記を `OutputPrinter` のテストで検証する (デコーダーをモックしない。対応付け関数へ timestamp 付きのエンコード列とデコード列を渡す)
- 既存の `inspect --decode` E2E (`tests/e2e.rs`) に回帰が無い

## 再観測時に保存するもの

失敗 JSON を再観測したら、原因切り分けの前に次を残す。対照クリップの再エンコードや open GOP 比較より、同じ失敗ファイルを hisui と ffmpeg の両方へ入力することを優先する。

- 失敗した MP4
- hisui の生 JSON
- hisui の正確なバージョンと git コミット
- VideoToolbox 経路の入力フレーム数と出力フレーム数
- ffmpeg がその MP4 から出したデコードフレーム数
- MP4 の実際の sample duration と、hisui が算出した `duration_us` の両方
