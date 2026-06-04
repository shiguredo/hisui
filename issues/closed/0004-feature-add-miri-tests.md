# miri を使ったテストを追加できないかを検討する

- Priority: Low
- Created: 2026-05-29
- Completed: 2026-06-04
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

検討の結論は **miri の一般的な導入は見送る（想定パターン A）**。pilot を実機で実施し、Q1〜Q6 を実測で確定した。

### 実施環境

- 日付: 2026-06-04
- toolchain: `rust-toolchain.toml` は stable のまま据え置き、`rustup run nightly` で override
- nightly: rustc 1.98.0-nightly (57d06900f 2026-05-27)
- miri: 0.1.0 (57d06900fd 2026-05-27)
- セットアップ: `rustup component add --toolchain nightly miri rust-src`（ローカルのみ。`Cargo.toml` と `rust-toolchain.toml` は未変更）

### 実施した pilot と結果

前提として `cargo miri test` は `hisui` lib 全体（aws-lc-sys / sora_sdk / shiguredo_webrtc 等の C-FFI 依存を含む）をコンパイルする。**この全体コンパイルは miri ターゲットで成功した**（依存ビルドを除き約 37 秒）。FFI は呼び出し時のみ問題になるため、コンパイル自体は通る。

純 Rust モジュールの既存 unit test を厳密なモジュールパスで絞って実行した（いずれも新規テストの追加なし）。結果は次のとおり。

| 対象フィルタ | 件数 | Stacked Borrows (既定) | Tree Borrows |
| --- | --- | --- | --- |
| `json::tests`（json 5 + sora stats json 2） | 7 | ok | ok |
| `timestamp::mapper::tests` | 11 | ok | ok |
| `timestamp::sample_aligner::tests` | 4 | ok | ok |
| `codec_string::tests` | 15 | ok | ok |

再現コマンド:

```console
rustup component add --toolchain nightly miri rust-src
SDKROOT=$(xcrun --show-sdk-path) rustup run nightly cargo miri test --lib -- json::tests
# Tree Borrows を使う場合は MIRIFLAGS を付与する
MIRIFLAGS=-Zmiri-tree-borrows SDKROOT=$(xcrun --show-sdk-path) \
  rustup run nightly cargo miri test --lib -- timestamp::mapper::tests
```

**UB は Stacked Borrows / Tree Borrows のいずれでも検出されなかった。**

一方、フィルタを `json`（素朴な部分一致）にすると tokio ランタイムを張る `endpoint_http_metrics::tests::...json_format` まで巻き込み、miri が即 abort した。

```text
error: unsupported operation: can't call foreign function `kqueue` on OS `macos`
  ... mio::sys::unix::selector::Selector::new
  ... tokio::runtime::Builder::build
```

miri は最初の unsupported operation / UB でプロセス全体を abort する。tokio・FFI を踏むテストが 1 件でも混じると pure テストの結果も得られない。lib の unit test は約 564 件あり、上記で miri 実行できたのは 37 件（約 6.5%）にとどまる。

### Q1〜Q6 の結論

- **Q1（価値があるか）**: 限定的。hisui 本体の `unsafe` は 11 箇所すべてが FFI の薄いラッパー（`raw_player::quit` / `libc::getrusage` / `libc::statfs` / WebRTC バッファ操作）で、その先（C 側）を miri は解釈できない。miri が実際に動かせる純 Rust モジュール（json / timestamp / codec_string / sora stats json）には `unsafe` も生ポインタも無く、miri は「遅い再実行環境」にとどまる。
- **Q2（対象範囲）**: 仮に絞るなら上記 4 モジュール。ただし価値は低い。唯一の例外は `src/webrtc/video.rs` の `copy_plane`（`ptr::copy_nonoverlapping` を使う自前の stride コピー）。これは FFI に依存しない生ポインタ操作なので、宛先を `Vec` で確保した隔離テストを書けば miri で検証する価値がある。
- **Q3（CI）**: 仮に導入するなら別ジョブ・週次が妥当。ただし下記 Q4 の制約で素朴な実行は不可。
- **Q4（ソース側対応）**: 致命的。`cargo miri test --lib` の素朴な一括実行はできない。部分一致フィルタですら tokio テストを巻き込んで abort する。pure テストだけを安全に回すには `#[cfg(not(miri))]` での FFI/tokio テスト除外、または miri 対象テストの専用モジュール隔離が必須で、約 564 件のテストへ網羅的に属性を付与・維持するコストが発生する。「最小コスト」では収まらない。
- **Q5（nightly 管理）**: `rust-toolchain.toml` を stable のまま `rustup run nightly cargo miri` で override できることを実証した。toolchain 二重化は可能。
- **Q6（Tree Borrows 等）**: pure 対象では `-Zmiri-tree-borrows` でも UB ゼロ。tokio を含めると即 abort するため `-Zmiri-many-seeds` 等のスケジューリング探索は対象外。

### 最終結論

- **一般的な miri テストスイートの導入は見送る（パターン A）**。理由は (1) miri 実行可能な純 Rust 部分に自前 `unsafe` が無く価値が「遅い PBT」止まり、(2) FFI/tokio テストの除外という非自明な構造変更が必須でコストに見合わない、(3) Stacked / Tree Borrows いずれでも UB ゼロ。
- **依存 crate 側（shiguredo_mp4 等）の `unsafe` は依存 crate 側で miri を回すのが筋（パターン C）**。codec_string 経由でも UB は出ていない。
- **唯一の残課題は `copy_plane`**。ただし当プロジェクトは stable + PBT(proptest) + fuzzing(cargo-fuzz) を方針としており、生ポインタ境界の検証は将来整備する fuzz target で賄う方が toolchain 二重化を避けられて筋が良い。miri 専用導入は行わない。

### 留意点（実施記録）

- pilot のための変更は「ローカルへの miri コンポーネント追加」のみ。`Cargo.toml` / `rust-toolchain.toml` / `src` / `tests` は未変更。
- miri は通常テストより十分遅い（pure テスト 7〜15 件で 3〜6 秒）。FFI 依存込みの全体ビルドは別途必要。
