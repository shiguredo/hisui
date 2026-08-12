# VideoToolbox デコーダーで 1 つの MP4 内の解像度変更をデコードできない問題を修正する

- Created: 2026-08-04
- Completed:
- Branch: feature/fix-videotoolbox-decoder-resolution-change
- Polished: 2026-08-10
- Updated: 2026-08-10

## 目的

1 つの MP4 内で途中から解像度が変わる入力を、macOS の VideoToolbox デコーダー (H.264 / H.265) で正しくデコードできるようにする。特に H.265 の `hev1` (in-band パラメータセット変化が仕様準拠のパターン) に対応する。現状は後半のフレームが silent frame drop でデータ欠損する。

## 現状

### 対象と非対象

- hisui の MP4 リーダー (`src/mp4/sample_reader.rs`, `src/mp4/reader.rs`) は `sample.sample_entry` が `Some` で来るたび (`stsc` が別 `stsd` エントリーを指した瞬間) に `last_video_sample_entry` を更新してフレームに載せる。したがって**複数 `stsd` エントリーを持つ MP4** なら現行コードで `VideoToolboxDecoder::reinitialize_if_need` が発火し、正しく再初期化される。
- 問題が出るのは**単一 `stsd` + ビットストリーム内 SPS/PPS 変化**のパターン。

### 仕様上の位置付け (ISO/IEC 14496-15)

| Sample entry | in-band パラメータセット | hisui 現行のサポート | 本 issue の対象 |
|---|---|---|---|
| `avc1` (H.264) | 禁止 (置けば ill-formed) | サポート | 対象 (実運用対策) |
| `avc3` (H.264) | 許容 | 非サポート | 対象外 |
| `hvc1` (H.265) | 禁止 | サポート | 対象 (実運用対策) |
| `hev1` (H.265) | 許容 (正規パターン) | サポート | **対象 (仕様準拠のために必須)** |

- `avc1` / `hvc1` は SPS/PPS 変化時に**新しい sample entry を作るのが仕様**であり、in-band SPS/PPS は認められない。しかし ffmpeg の `-f concat -c copy` は複数入力を単一 stsd + inline SPS/PPS の形で出力する。仕様違反ながら実運用で自然発生するパターン。
- `hev1` は in-band パラメータセット変化が**仕様準拠の正規パターン**。hisui は `hev1` をサポート宣言しているのに、現行 VideoToolbox デコーダーはこの変化を追従できず、hev1 の仕様準拠上の欠陥になっている。
- `avc3` は hisui 全体で未サポート (別途起票する価値はあるが本 issue の対象外)。

### バグの構造

- `src/decoder/video_toolbox.rs` の `VideoToolboxDecoder::reinitialize_if_need` は `get_h264_sps_pps` / `get_h265_vps_sps_pps` で SPS/PPS/VPS を取得し保持値との差分で再初期化を判定する
- ただし `VideoFormat::H264` (AVCC / MP4) と `VideoFormat::H265` の経路では、`get_h264_sps_pps` / `get_h265_vps_sps_pps` の実装が `frame.sample_entry` (avcC / hvcc box) からしか読まない
- そのため単一 stsd で sample_entry が不変のまま bitstream 内 SPS/PPS だけが変わる入力では検知できず、後半のフレームが古い設定のデコーダーに渡って VideoToolbox が `status=-12909` を返す
- `VideoFormat::H264AnnexB` 経路は `H264AnnexBNalUnits::new(&frame.data)` でフレームデータから直接 SPS/PPS を拾うため本問題は発生しない
- VP9 / AV1 は `reinitialize_raw_codec_if_need` で解像度変化そのものを検知しているため本問題は発生しない
- nvcodec (`src/decoder/nvcodec.rs`) はフレームデータを Annex.B 形式で NVDEC に渡すため inline パラメータセットを拾える可能性が高い (本 issue のスコープ外。追従の検証と対応は issue 0093 / 0094)

### 再現手順と実測結果 (2026-08-04, H.264 avc1 で確認)

再現手順:

1. 640x480 と 320x320 の H.264 baseline MP4 を ffmpeg で 25 fps × 1 秒ずつ生成する
2. `ffmpeg -f concat -safe 0 -i list.txt -c copy concat.mp4` で単一 stsd に結合する (ffmpeg は `Auto-inserting h264_mp4toannexb bitstream filter` を挟んで後半の SPS/PPS を inline に書き出す)
3. macOS 上で `hisui inspect --decode --verbose concat.mp4` を実行する

実測結果:

- MP4 の `stsd` エントリーは 1 つのみ、トラック宣言は `avc1 640x480`
- キーフレームは 2 つ (t=0 / t=1000000us)、両方に `SEI(6) + SPS(7) + PPS(8) + IDR(5)` の NALU 列を含む
- **前半 25 フレーム (640x480)**: `decoded_data_size: 460800, width: 640, height: 480` で正しくデコード
- **後半 25 フレーム (320x320)**: `decoded_data_size` / `width` / `height` の**フィールドごと欠落** (デコード失敗による silent drop)
- ログに `[shiguredo_video_toolbox] output_callback() failed: status=-12909` がちょうど 25 個 (欠落フレーム数と一致)
- **プロセスは exit code 0 で成功終了** (hard error にならず silent frame drop)

H.265 の `hev1` は 2026-08-10 に実機確認済み。`get_h265_vps_sps_pps` が `frame.sample_entry.hvcc_box` からしか読まない構造は同じで、in-band パラメータセット変化のある hev1 入力に対して H.264 と同種の失敗 (status=-12909 × 25) が発生する。なお H.265 の再現データは `ffmpeg -c:v libx265 -x265-params repeat-headers=1` で各キーフレームに VPS/SPS/PPS を入れてから concat する必要がある (デフォルトの concat では後半キーフレームに in-band パラメータセットが入らない)。

## 設計方針

フレームデータ (AVCC 形式) 内の SPS/PPS/VPS NALU を検出して再初期化判定に使う。

- `VideoFormat::H264` (AVCC): AVCC の length-prefix を辿って SPS (type 7) / PPS (type 8) NALU を抽出し、保持値と比較する。フレーム内に該当 NALU があればそれを優先、無ければ従来どおり `frame.sample_entry` にフォールバックする (`avc1` の仕様準拠入力向け)
- `VideoFormat::H265` (AVCC): 同様に VPS (type 32) / SPS (type 33) / PPS (type 34) を抽出する。フォールバック方針も同じ (`hvc1` の仕様準拠入力向け)
- `VideoFormat::H264AnnexB` は既にフレームデータから拾っているため変更なし
- AVCC の length-prefix 走査による SPS/PPS/VPS 抽出は既存コードに無いため専用パーサの新設が必要 (`H264AnnexBNalUnits` / `H265AnnexBNalUnits` は Annex.B 専用、`parse_avcc_sps_pps_lists` は WebM リーダー削除 (issue 0090) で削除済み)。NALU 長プレフィックスは既存デコーダーと同様に 4 バイト固定 (`NALU_HEADER_LENGTH`) で扱う
- 先行実装の参考: `src/decoder/openh264.rs` の `build_annexb_input` は H.264 の AVCC length-prefix 走査で SPS/PPS を検出し、フレーム内に無ければ `sample_entry` の avcC から補完する既存実装 (H.265 の VPS/SPS/PPS 検出は無い)。NALU 長プレフィックスの走査自体は `src/decoder/nvcodec.rs` の `decode()` 内にも存在する (こちらは NAL タイプ検出を含まない)
- H.265 の NALU 定数 (`H265_NALU_TYPE_VPS` / `H265_NALU_TYPE_SPS` / `H265_NALU_TYPE_PPS`) と `parse_sps` / `parse_hevc_sps` (EBSP から RBSP を抽出する既存実装) は現存するので活用する

## 完了条件

- 上記の再現手順で作成する concat MP4 を `hisui inspect --decode` に渡したとき、後半 25 フレームにも `width` / `height` / `decoded_data_size` が付き、`output_callback() failed: status=-12909` のログが 0 件になること
- H.265 についても hev1 で同等の再現データを作成し、正しくデコードできることを確認する。なお H.265 の再現データは H.264 と同じ concat 手順では後半キーフレームに in-band VPS/SPS/PPS が入らないため、x265 エンコード時に `repeat-headers=1` を指定して各キーフレームに VPS/SPS/PPS を含めること
- 上記の再現データをテストデータとして加え、`hisui inspect --decode` の JSON 出力で全サンプルに `width` / `height` が付くことを assert する回帰テストを追加すること (配置場所は既存の `testdata/` レイアウトに合わせる)。VideoToolbox は macOS 専用のため、このテストは macOS でのみ実行する (Linux CI には H.264 / H.265 のデコーダーエンジンが無く失敗する。既存の `tests/decoder_tests.rs` の `#[cfg(target_os = "macos")]` と同じ扱いとする)
- 既存のテストが全て通ること

## 関連

- avc3 サポート追加 (in-band パラメータセットを正規に扱うが hisui 全体でハンドラが無い。本 issue のスコープ外)
- nvcodec デコーダーの解像度変化追従 (issue 0093。本 issue と同じく sample_entry / パラメータセット変化の追従問題を nvcodec 側で扱う)
- nvcodec デコーダーの `contains_parameter_sets` 全 NALU 走査化 (issue 0094)

## 残存する懸念 (2026-08-10 の判断)

SPS / PPS / VPS は NALU タイプごとに独立してフォールバックするため、キーフレームがパラメータセットの**一部のみ**を in-band に含む入力 (例: H.265 で VPS のみ in-band なし、SPS / PPS のみ in-band) では、新旧混在セットで再初期化が走り得る。混在セットで VideoToolbox の初期化が失敗した場合、Err は `reinitialize_if_need` の `if let Ok` の外 (`*self = Self::new_h265(...)?`) から伝播し、パイプライン全体が停止する。また、キーフレーム間で in-band 有無が不均一な入力では、旧 (sample_entry) と新 (in-band) の間で再初期化が往復するフラップが発生し得る。

ただし、主要エンコーダ (x264 / x265 / VideoToolbox / NVENC) はパラメータセットをセットで出力するため、実データでは混在ケースは確認されていない。2026-08-10 のレビュー時点で、**エラー扱いのまま維持する**と判断した (実際に混在ケースに遭遇したら、その時点で混在セットの検出と再初期化スキップ等の対応を検討する)。
