# 映像 sample_entry を SharedSampleEntry で全フレーム付与に統一する

- Priority: Low
- Created: 2026-06-08
- Completed:
- Model: Claude Opus 4.8
- Branch:
- Polished: 2026-06-09

## 目的

issue 0017 で音声側の sample_entry を「全出力フレームに載せる」方式へ変更し、共通型 `SharedSampleEntry` を導入した（差分最小化のため映像には手を付けなかった）。その結果、エンコード出力フレームの sample_entry の扱いが音声と映像で非対称になっている。

- 音声: 全出力フレームに sample_entry を載せる。フィールド型は `Option<SharedSampleEntry>`。
- 映像: 最初の出力フレーム（または SPS/PPS を含むフレーム）にだけ載せる。フィールド型は生の `Option<SampleEntry>`（writer の keyframe 補完で取りこぼしを防いでいる詳細は「現状」参照）。

この非対称は保守上分かりにくく、エンコードデータの sample_entry の扱いは音声・映像で揃っているべきである。本 issue では映像側にも 0017 と同じ変換を適用し、(1) 映像エンコーダが全出力フレームに sample_entry を載せ、(2) `VideoFrame.sample_entry` を `Option<SharedSampleEntry>` に揃えて、音声・映像で sample_entry の付与ポリシーと型を一貫させる。

本 issue の作業は「issue 0017 が音声に対して行った変更を映像に対して行う」ことが基本形であり、加えて映像エンコーダ固有の事情（後述）に対応する。なお、これはバグ修正ではなく一貫性のためのリファクタである（理由は優先度根拠を参照）。

## 優先度根拠

Low。映像は keyframe 補完によって muxer の契約（最初のサンプルに sample_entry 必須）を満たしており機能的なバグは無い。本 issue は型と付与ポリシーを音声と揃えるための仕上げで、時間があるときに対応する。

費用対効果は率直に書いておく。便益は「音声・映像で sample_entry の扱いが揃い、非対称が保守上の負債として残り続けるのを解消する」ことのみで、機能的な改善は無い。一方コストは小さくない（映像エンコーダ 5 種の改修、特に openh264 の mid-stream entry 保持という非自明な実装、keyframe 補完撤去によるリグレッションリスク、デコーダ 3 種・reader/writer・テストの広範な型追従）。便益が一貫性のみである点を踏まえ、CLAUDE.md「Premature Optimization is the Root of All Evil」と照らしても「やる」と判断する根拠は、この非対称が音声・映像をまたいで sample_entry を扱うたびに認知負荷を生み続ける負債だからである。

段階分割の余地: リスクは均質ではない。型統一（設計方針 1・4・5）は機械的・低リスク、全フレーム付与＋keyframe 補完撤去（設計方針 2・3）は openh264 の保持実装と「最初の出力フレームで必ず Some」前提に録画 correctness が依存するリスク集中部分。一貫性という目的には後者も必要だが、不安があれば「型統一のみ先行 → 全フレーム付与＋補完撤去は後続」と段階コミット / 段階 issue に分けてよい。

## 現状

- `VideoFrame.sample_entry`（`src/video.rs:50`）は 0017 完了後も生の `Option<SampleEntry>` のまま。共通型 `SharedSampleEntry`（`src/sample_entry.rs`）は 0017 で導入済みで、本 issue から利用できる。
- 映像エンコーダは sample_entry を「最初の出力フレーム（または SPS/PPS を含むフレーム）」にしか載せない。録画 writer はそれを取りこぼさないよう、`push_encoded_frame_with_metrics`（`src/encoder.rs:724-739`）で keyframe のときだけ `last_video_sample_entry` から補完している。「録画開始時のキーフレーム要求」＋この補完により、subscribe 直後の keyframe に必ず entry が届くため、映像では音声のような finalize 失敗レースは顕在化していない。
- 各エンコーダの sample_entry の持ち方は均質ではない（設計方針で個別に扱う）。

## 設計方針

### 1. フレーム型の変更（音声と同型）

`VideoFrame.sample_entry`（`src/video.rs:50`）を `Option<SampleEntry>` から `Option<SharedSampleEntry>` に変更する。生成側はラップ、読み出し側は `.as_ref().map(|e| e.get())` 等で `&SampleEntry` を取り出す。これは 0017 が `AudioFrame.sample_entry` に対して行ったのと同じ変換。

「全フレーム付与」の対象はエンコーダ出力フレームに限る。生データ由来・デコーダ出力・リサイズ由来の `VideoFrame`（`src/video.rs:230/241/292/432/475/499/667/981` ほか。例: `// 生データにはサンプルエントリは存在しない`）は `None` のまま正しい。これらが構造的に `None` を持つため、`VideoFrame.sample_entry` は `Option` のままにする（非 Option 化は issue 0028 として検討したが実装不可で close 済み。詳細は関連の 0028 を参照）。

### 2. 映像エンコーダの全フレーム付与（エンコーダごとに方式が異なる）

「最初の出力フレームだけに載せる」構造を「全出力フレームに載せる」へ変える。ただし各エンコーダで sample_entry の持ち方が異なるため一律ではない。

- `svt_av1`（`src/encoder/svt_av1.rs`）/ `libvpx`（同 `libvpx.rs`）/ `nvcodec`（同 `nvcodec.rs`）: `sample_entry: Option<SampleEntry>` フィールドを持ち、出力時に `self.sample_entry.take()` で初回フレームにだけ載せている。音声 3 エンコーダと同じ形なので、フィールドを `SharedSampleEntry`（非 Option）にし、`new()` / 各コンストラクタで `SharedSampleEntry::new(...)` で確定、出力時は `Some(self.sample_entry.clone())`（Arc clone）に変える。なお nvcodec は `new_h264` / `new_h265` / `new_av1` の各コンストラクタで sample_entry を確定済みで、別フィールド `av1_sequence_header`（keyframe への OBU 付与用）は sample_entry とは独立なので混同しないこと。
- `video_toolbox`（`src/encoder/video_toolbox.rs`）: `sample_entry` フィールドを持たず、`self.is_first` のとき出力フレームの SPS/PPS/VPS から entry を生成し初回だけ載せる。フィールド（例: `sample_entry: Option<SharedSampleEntry>`）を新設し、初回生成時に `SharedSampleEntry::new(...)` で保持して以後は毎フレーム clone する方式に作り変える。これにより `is_first` フラグは「entry 未生成か」を `Option::is_none()` で代替できるため削除できる。
- `openh264`（`src/encoder/openh264.rs`）: `sample_entry` フィールドを持たず、SPS/PPS を含む出力フレームでだけ entry を生成し、含まないフレーム（P フレーム等）は `None` を載せている。さらにコメント（`src/encoder/openh264.rs:50-53`）のとおり keyframe 要求等で **SPS/PPS が mid-stream で更新され、entry が途中で変わりうる**。よって「保持した最新 entry を毎フレーム載せ、SPS/PPS を含むフレームが来たら保持 entry を差し替える」方式にする。フィールド（例: `last_sample_entry: Option<SharedSampleEntry>`）を新設する。注意: 補完撤去（設計方針 3）後、このフィールド保持を入れないと **SPS/PPS を伴わない全フレーム（大半の P フレーム）が `None` になる**ため、保持は必須。最初の SPS/PPS を受け取る前のフレームだけは `None` のままだが、録画開始は keyframe 要求により IDR（SPS/PPS 付き）から始まるため、最初の出力フレームで entry が確定する。

この結果、映像エンコーダの「載せるべき不変条件」は「**最初の出力フレーム以降、全出力フレームに `Some` が載る**」であり、「全フレームで同一 entry」ではない（openh264 は値が変わりうる）。entry が途中で変わっても、muxer は 2 サンプル目以降の同一 entry を `PartialEq` で集約するため出力は正しい（0017 で確認した muxer 契約）。

### 3. encoder.rs の keyframe 補完の撤去と last_video_sample_entry の削除

全フレーム付与後はエンコーダ出力に必ず entry が載るため、`push_encoded_frame_with_metrics`（`src/encoder.rs:724-739`）の「keyframe のときだけ `last_video_sample_entry` から補完する」分岐（`src/encoder.rs:729-737`）は不要になる。これを撤去し、補完責務をエンコーダ側へ一本化する。

撤去後、`last_video_sample_entry`（宣言 `src/encoder.rs:436`、書込 727、読込 733）は読み手を失って dead になるため、フィールドごと削除する（「`SharedSampleEntry` 経由に直す」のではない）。

「録画開始時のキーフレーム要求」機構（`request_upstream_video_keyframe` 等）は sample_entry 補完とは別目的（subscribe 直後のデコード可能性の確保）なので **残す**。撤去するのは sample_entry の補完分岐のみ。

### 4. 消費側（デコーダ）の型追従

映像デコーダは `frame.sample_entry` をパターンマッチ・関数渡しで読むため、`Option<SharedSampleEntry>` 化に伴い `.get()` 経由に直す。`video_toolbox` / `nvcodec` は feature・プラットフォーム依存なので `--features` 指定でのビルド確認が必須（0017 で CI の `test-fdk-aac` / `test-nvidia-video-codec` が漏れを検出した経緯あり）。

- `src/decoder/openh264.rs:134-139`（`Some(SampleEntry::Avc1(...)) = frame.sample_entry.as_ref()` のマッチ）
- `src/decoder/video_toolbox.rs:276`（H.264）/ `:323-326`（H.265）の `&frame.sample_entry` のマッチ（macOS feature）
- `src/decoder/nvcodec.rs:97-100`（`frame.sample_entry` を `extract_parameter_sets_annexb` に渡す。nvcodec feature）

### 5. reader / writer の映像経路の型追従（音声と同型）

`VideoFrame.sample_entry` 型変更に伴い、生の `Option<SampleEntry>` を扱っている映像経路を `SharedSampleEntry` 経由に直す。これは 0017 が音声経路に対して行った変換（`.map(SharedSampleEntry::new)` でラップ、muxer 渡しでは `.get().clone()` で生 `SampleEntry` を取り出す）と同型。

- reader（映像フレーム生成。音声の隣接サイトは 0017 で対応済み）:
  - `src/mp4/reader.rs:1189` / `:1224`（`sample_entry: context.sample_entry` を `.map(SharedSampleEntry::new)` でラップ）
  - `src/mp4/sample_reader.rs:138`（同上）
  - `src/sora/recording_mp4_reader.rs:151`（映像フレーム生成。`sample_entry`（`:101` の生 `Option<SampleEntry>`）を `.map(SharedSampleEntry::new)` でラップ。音声側 `:297` は対応済みでその対称）
  - `src/rtmp/frame.rs`（RTMP → 内部フレーム変換。音声は 0017 で対応済み）: 映像 sample_entry の読み出し（`:99-103` で `entry` を `extract_nalu_length_size` / `create_video_sequence_header` に渡す箇所）を `entry.get()` に、VideoFrame 構築（`:299` の `Some(sample_entry.clone())`、内部フィールド `video_sample_entry: Option<SampleEntry>`（`:168`）由来）を `Some(SharedSampleEntry::new(...))` でラップする。**`rtmp` は feature gate されずデフォルトビルドに含まれるため必須**（これを漏らすと `cargo check` が落ちる）。
- writer（映像経路。音声は 0017 で対応済みで、その対称形）。各 writer の `last_video_sample_entry`（現状 `Option<SampleEntry>`）を `Option<SharedSampleEntry>` にし、clone_from・codec string 抽出・muxer 渡し・`fill_missing_sample_entries` を `.get()` 経由に直す:
  - `mp4/hybrid_writer.rs`: `last_video_sample_entry`（`:82`）、映像 `or_else`（`:219`, `:422`）、映像入口（受信時の取り込み）。既存の映像テスト（`hybrid_writer_captures_video_sample_entry_at_ingress` / `keeps_video_*` / `counts_missing_video_*`、`:1451` の `assert_eq!` 等）も `SharedSampleEntry` 経由に更新する。
  - `dash/writer.rs`: `last_video_sample_entry`（`:257`）、clone_from（`:484`）、`or_else`（`:525`）、`fill_missing_sample_entries`（`:1006-1021`、映像・音声共通関数なので映像分岐の引数型に注意）、codec string 抽出。
  - `hls/writer.rs`: `last_video_sample_entry` は 2 構造体（`:324` / `:341`）。clone_from・`or_else`（`:610`）・`fill_missing_sample_entries`（`:1170-1181`）・codec string 抽出。
  - `mp4/writer.rs`（標準 Mp4Writer）: passthrough（補完なし）。映像 sample_entry を muxer に渡す箇所を `.get().clone()` に直す。

なお writer の `or_else` 補完そのもの（および `Option` 分岐）の **除去は本 issue では行わない**。0017 が音声でも or_else を残したのと揃え、補完ロジックは構造を保ったまま型だけ追従させる（`VideoFrame.sample_entry` は `Option` のまま残るため、補完は引き続き必要）。

## 実装スコープ（変更対象ファイル）

1. `src/video.rs`: `VideoFrame.sample_entry` の型変更。内部の生成サイト（`None` のものは型が合うので変更不要、`self.sample_entry.clone()` を載せる `:667` 等は型追従）。
2. `src/encoder.rs`: keyframe 補完分岐の撤去、`last_video_sample_entry` フィールド削除。
3. 映像エンコーダ 5 種（`svt_av1` / `libvpx` / `nvcodec` / `video_toolbox` / `openh264`）の全フレーム付与（設計方針 2 の方式別）。
4. 映像デコーダ 3 種（`openh264` / `video_toolbox` / `nvcodec`）の `.get()` 追従（設計方針 4）。既存のデコーダ単体テスト（`src/decoder/openh264.rs` の `#[cfg(test)]` が `VideoFrame { sample_entry: Some(...) }` を構築する箇所、`:181` / `:218` 等）も `SharedSampleEntry::new(...)` に更新。
5. reader / writer の映像経路（設計方針 5）。
6. テスト（後述）。

## テスト

CLAUDE.md のテスト役割分担に従う。本リポジトリには現状 PBT 基盤（`pbt/` クレート・`proptest`）が無いため、検証は `tests/*_tests.rs` の統合テストと各モジュールの `#[cfg(test)]` 単体テストで行う（0017 と同方針。PBT 基盤新設は本 issue のスコープ外）。

- 不変条件テスト（新規）: 映像エンコーダが「最初の出力フレーム以降、全出力フレームに `Some` の sample_entry を載せる」ことを検証する。`src/encoder/*.rs` には現状 `#[cfg(test)] mod tests` が無いため新設になる。常時利用可能なエンコーダ（`libvpx` の VP8/VP9 等）で最低限検証し、feature / プラットフォーム依存（`nvcodec` / `video_toolbox`）は該当 feature 有効時のみ走るよう `#[cfg(...)]` でガードする。
- openh264: SPS/PPS が更新されるフレームを含めても全フレームに `Some` が載る（かつ更新時に値が差し替わる）ことを検証する。実エンコーダを回す統合テスト（`tests/*_tests.rs`）で確認する。
- 既存テストの更新: `mp4/hybrid_writer.rs` の映像テスト、`src/decoder/openh264.rs` の `#[cfg(test)]` の `VideoFrame { sample_entry: Some(...) }` 構築箇所を `SharedSampleEntry::new` 経由に型追従させる。`tests/mixer_video_tests.rs` は `sample_entry: None` の入力ヘルパのみで型互換のため変更不要だが、VP8 を end-to-end で回すため libvpx の全フレーム付与の統合検証に使える。
- feature / プラットフォーム別ビルド確認: `nvcodec` feature、`video_toolbox`（macOS）でのビルドを必ず確認する（デフォルト `cargo check` では検出されない）。

## 完了条件

- `VideoFrame.sample_entry` が `Option<SharedSampleEntry>` になり、音声と型が揃うこと。
- 映像エンコーダが「最初の出力フレーム以降、全出力フレームに `Some` の sample_entry を載せる」こと（5 エンコーダすべて。openh264 は SPS/PPS 更新時に値が差し替わってよい）。これを単体 / 統合テストで検証する。
- `src/encoder.rs` の keyframe 限定補完が撤去され、`last_video_sample_entry` フィールドが削除されること。「録画開始時のキーフレーム要求」機構は残ること。
- 映像デコーダ・reader・writer の映像経路が `SharedSampleEntry` 経由でコンパイル・動作すること。`nvcodec` / `video_toolbox` の feature・プラットフォーム別ビルドも通ること。
- 録画機能（特に短時間録画・録画開始直後の映像トラック）にリグレッションが無いこと。
- CHANGES.md: 本 issue はリファクタで、利用者（録画機能の利用者）から見た挙動は不変。`VideoFrame` はクレートルートに `pub use`（`src/lib.rs`）されており `sample_entry` は `pub` フィールドなので、厳密には Rust ライブラリ利用者向けの公開 API 変更だが、0017 が `AudioFrame` の同種変更を CHANGES.md 無記載で行った前例がある。これに倣い `### misc` への記載要否を判断する（未リリース部分の内部変更として記載不要とする判断も可。実装時に確定する）。

## 関連

- issue 0017（音声側の全フレーム付与と共通型 `SharedSampleEntry` 導入。本 issue の直接の前提・実装の手本。closed）
- issue 0028（sample_entry フィールドの非 Option 化。生フレームが構造的に `None` を持つため実装せず close 済み。本 issue は非 Option 化を前提とせず、音声・映像の一貫性のみを目的とする）
