# miri を使ったテストを追加できないかを検討する

- Priority: Low
- Created: 2026-05-29
- Completed:
- Model: Opus 4.7
- Branch: feature/add-miri-tests
- Polished: 2026-05-29

## 目的

[rust-lang/miri](https://github.com/rust-lang/miri) は Rust の MIR (Mid-level Intermediate Representation) インタプリタで、未定義動作 (UB)・use-after-free・データ競合・aliasing 規則違反 (Stacked Borrows / Tree Borrows) など、通常のテストでは検出が難しいメモリ安全性違反を検出できる。本 issue では、hisui に対して miri を用いたテストを導入する余地があるかを **検討する** ことが目的で、実装着手より前に「どの範囲なら現実的に走らせられるか」「導入コストに見合うか」を見極める。実装方針が固まったら別 issue に切り出す。

## 優先度根拠

- hisui は現状 PBT (proptest) と fuzzing (cargo-fuzz) でメモリ安全性・パニック耐性を一定カバーしており、明確な不具合が出ているわけではない。
- 一方で `unsafe` ブロックが少数ながら存在し（後述）、また `shiguredo_*` 系の外部 crate が unsafe を含むため、stacked borrows レベルの UB 検出は理論的には価値がある。
- 一方、hisui は大量の C ライブラリへの FFI に依存しているため、miri がそのままの構成で全テストを走らせるのは現実的でない。「適用可能な範囲が極めて狭い」 or 「適用前提でテストや CI を組み替えるコストが高い」のどちらかで終わる可能性が高く、検討の結果として「導入しない」になっても妥当。
- 以上から、緊急度は低く、`Low` 優先で「やる/やらない/どこまでやる」を判断するための調査 issue とする。

## 現状

### Rust toolchain

- `rust-toolchain.toml` は `channel = "stable"`。miri は nightly でのみ提供されるため、現状そのままでは miri は実行できない。
- miri を導入する場合、選択肢は (a) toolchain を nightly に切り替える、(b) `rust-toolchain.toml` を残したまま miri 用途だけ nightly に override する、の 2 通り。本プロジェクトは stable 縛りで開発フロー (rustfmt, clippy) を組んでいるので、(b) を前提とする検討が現実的。

### unsafe の利用箇所（hisui 本体）

`src/` 配下の `unsafe` 出現箇所は以下のみ:

- `src/subcommand_server.rs`: `raw_player::quit()` の呼び出し (`unsafe { raw_player::quit() }`)。SAFETY コメントあり。
- `src/obsws/response/general.rs`: 詳細未確認だが少数。
- `src/webrtc/video.rs` / `src/webrtc/audio.rs`: WebRTC FFI ラッパー周辺。

hisui の `unsafe` は、ほとんどが「外部 (主に C) ライブラリの thin wrapper」に集中している。これらは miri が C 側を実行できないため、そもそも miri の対象外になる可能性が高い。

### 外部 crate の依存

`Cargo.toml` を見ると、hisui は以下の C ライブラリ系 crate に強く依存している:

- shiguredo_libvpx, shiguredo_dav1d, shiguredo_libyuv, shiguredo_openh264, shiguredo_opus, shiguredo_svt_av1, shiguredo_fdk_aac, shiguredo_nvcodec
- shiguredo_audio_device, shiguredo_video_device, shiguredo_audio_toolbox, shiguredo_video_toolbox
- aws-lc-rs, rustls (TLS は rustls 純 Rust だが crypto は aws-lc-rs = C 実装)
- sora_sdk (内部に WebRTC = C/C++ を含む)

これらに依存するテストは miri では動かない (FFI を解釈できない)。`tests/` 配下の `decoder_tests.rs`, `mixer_*`, `writer_mp4_tests.rs` などはこれらに直接依存しているため、miri 対象から除外する必要がある。

### miri 適用が現実的な候補

純 Rust ロジックで、外部 C ライブラリに依存しない部分が候補。簡易調査ベースで以下が該当する見込み:

- `src/timestamp/` 系: タイムスタンプの算術ロジック。
- `src/types.rs`: `ContainerFormat`, `CodecName` などのパース・JSON 変換。
- `src/layout/` 系 (もし存在すれば): レイアウト計算。`layout-examples/` の入力をパースして処理するロジック。
- `src/json.rs`, `src/codec_string.rs`: 純 Rust ロジック。
- `src/mp4/reader.rs` のうち `Mp4FileDemuxer` を用いない純粋関数 (`calculate_timestamps` 等)
- shiguredo_mp4 / shiguredo_m3u8 / shiguredo_mpd 等の純 Rust crate を用いるロジックの一部。

これらの中で「実際に unsafe や生ポインタ操作を含むコードを呼び出す」テストはほぼ無く、純粋に「正しい入出力か」のテストになる。**そうなると miri は単に "遅い PBT 実行環境" にしかならず、価値が薄い**。

価値が出るのは:

- `shiguredo_mp4` などの **依存 crate 側に unsafe がある** 場合、その crate 内部の aliasing 違反を hisui のテスト経由で検出できる可能性。
- ただしこれは依存 crate 側の責務であり、`shiguredo_mp4` 等のリポジトリで個別に miri を回す方が筋。

### 既存テストインフラ

- `tests/`: `decoder_tests.rs`, `e2e.rs`, `layout_tests.rs`, `mixer_audio_tests.rs`, `mixer_video_tests.rs`, `reader_webm_tests.rs`, `writer_mp4_tests.rs`。多くが C ライブラリに依存。
- PBT / fuzzing は CLAUDE.md で言及されているが、`pbt/` ディレクトリは現状リポジトリに無い (将来追加予定の枠組み)。
- 既存 CI は stable 前提。

## 設計方針 (検討内容)

本 issue は「検討」が成果物。実装着手はしない。検討で答えを出すべき問いと、それに対する現時点の見立てを列挙する。

### Q1. そもそも導入する価値があるか

- **結論の方向性**: 限定的。hisui 本体の `unsafe` は thin FFI wrapper に集中しており、miri はその先 (C 側) を解釈できない。
- 検証する手段としては、`cargo +nightly miri test -- <subset>` で純 Rust 部分の小さなテストだけ手動で走らせてみる pilot を行い、何件かの UB が引っかかるかどうかを実測する。引っかからない場合は導入見送り、引っかかる場合は対象範囲を決めて導入する。

### Q2. もし導入するなら、対象はどこに絞るか

- 候補:
  - timestamp / types / codec_string / json などの純 Rust モジュール。
  - shiguredo 系 crate のうち hisui が直接使う API (mp4 parser, m3u8 parser 等) の薄いラッパーテスト。
- 除外:
  - decoder / encoder / mixer / scaler / yuv / audio_device / video_device / WebRTC / TLS / 録画 / 合成系すべて。

### Q3. CI でどう走らせるか

- 選択肢:
  - (a) 既存の `cargo test` とは別ジョブで `cargo +nightly miri test --test <name>` を選択的に実行。
  - (b) PR ごとには走らせず、`develop` push 時 or 週次の cron で実行（実行時間が長くなりがちなため）。
  - (c) ローカル開発者用の make target / script だけ用意して CI には載せない。
- 第一候補は (b)。CI 時間を抑えつつ、UB の早期検出は得たい。

### Q4. ソース側で必要な対応

- `#[cfg(miri)]` で miri 実行時にスキップするテストを設ける。FFI を呼ぶテストは全部この属性を付ける。
- `tests/` の中で miri 対象テストだけ別ファイル (`tests/miri_*.rs`) に切り出し、`#[cfg(miri)]` 付与の手間を最小化する案もある。
- proptest や tokio runtime を含むコードは miri 上で動かないか極端に遅いため、対象テストは std + 純粋 Rust に絞る。

### Q5. nightly 依存をどう管理するか

- `rust-toolchain.toml` は stable のままにし、miri 実行時のみ `rustup run nightly cargo miri test` でオーバーライドする。
- nightly が壊れて miri が動かない期間が出るため、CI ジョブは fail-soft (`continue-on-error` 相当) で運用する選択肢もある。本番ビルドを止めない設計にする。

### Q6. データ競合検出 (`-Zmiri-disable-isolation` / Stacked Borrows / Tree Borrows) の利用

- データ競合検出 (`-Zmiri-tree-borrows -Zmiri-many-seeds=...`) は、対象が pure Rust の場合に限り意味がある。tokio runtime を含めるとほぼ確実にスタックする。
- まずは `MIRIFLAGS=-Zmiri-tree-borrows` のみで pilot し、追加オプションは結果を見て決める。

## 完了条件

本 issue は「検討の完了」が条件。以下を満たした時点で close する。

- 上記 Q1〜Q6 の各論点について、pilot 実行ベースで方針を確定させ、本 issue に追記する。
- 結論として:
  - 「導入する」場合: 対象範囲・CI 設計・必要なソース変更を別 issue 群に切り出し、本 issue は close する。
  - 「導入しない」場合: 理由 (pilot で UB が見つからなかった / コストに見合わない 等) を本 issue に明記して close する。
- pilot 結果は再現可能な形でログを残す (実行コマンド、対象テスト、検出結果)。

## 解決方法

### 検討手順

1. nightly toolchain を rustup でインストールし、`rustup +nightly component add miri` でセットアップする。
2. 純 Rust の小さな対象 (例: `src/types.rs` の `ContainerFormat::from_path`、`src/codec_string.rs`、`src/timestamp/` 系) に対する単体テストを 1〜2 本に絞って `cargo +nightly miri test --test <name>` を実行する。
3. FFI を含むテストは `#[cfg(not(miri))]` で除外し、miri 実行時のコンパイルだけは通るようにする。
4. 実行結果を踏まえ、以下を判断する:
   - 検出された問題があれば、それを別 issue として登録する。
   - 検出された問題が無くても、対象範囲を拡げて pilot を続けるか、見送るかを決める。
5. 結論を本 issue に追記して close する。

### 想定される結論パターン

- **パターン A (導入見送り)**: 純 Rust 対象が薄く、検出される UB が無く、CI コスト・toolchain 二重化のコストに見合わない場合。本 issue で根拠を残して close。
- **パターン B (限定導入)**: timestamp / types / 一部 mp4 関連の純 Rust テストだけ miri 対象にし、週次 CI ジョブで走らせる。導入 PR を別 issue で起こす。
- **パターン C (依存 crate 側で対応すべきと判定)**: hisui で miri は走らせず、`shiguredo_mp4` などの依存 crate 側で miri テストを整備すべきと判定し、それを依存 crate 側の issue として起票する。

### 留意点

- 検討段階なのでコード変更は最小限。`Cargo.toml` を触らず、`rustup` 経由でローカルにだけ miri を入れて pilot するのが望ましい。
- 検討中に「ローカル pilot のためだけに `cfg(miri)` を src に撒く」のは過剰。pilot は `tests/` 配下に新規テストを 1〜2 本追加するだけで成立させる。
- miri は実行速度が通常テストの 100〜1000 倍遅い。pilot 段階でも時間制約を意識し、ループ回数・入力サイズを小さくする。
