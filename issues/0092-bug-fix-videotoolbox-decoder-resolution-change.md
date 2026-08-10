# VideoToolbox デコーダーで 1 つの MP4 内の解像度変更をデコードできない問題を修正する

- Created: 2026-08-04
- Completed:
- Branch: feature/fix-videotoolbox-decoder-resolution-change
- Polished:

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
- nvcodec (`src/decoder/nvcodec.rs`) はフレームデータを Annex.B 形式で NVDEC に渡すため inline パラメータセットを拾える可能性が高い (本 issue のスコープ外・要検証)

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

H.265 の `hev1` は未実測だが、`get_h265_vps_sps_pps` が `frame.sample_entry.hvcc_box` からしか読まない構造は同じで、in-band パラメータセット変化のある hev1 入力に対して同種の失敗が発生する。

## 設計方針

フレームデータ (AVCC 形式) 内の SPS/PPS/VPS NALU を検出して再初期化判定に使う。

- `VideoFormat::H264` (AVCC): AVCC の length-prefix を辿って SPS (type 7) / PPS (type 8) NALU を抽出し、保持値と比較する。フレーム内に該当 NALU があればそれを優先、無ければ従来どおり `frame.sample_entry` にフォールバックする (`avc1` の仕様準拠入力向け)
- `VideoFormat::H265` (AVCC): 同様に VPS (type 32) / SPS (type 33) / PPS (type 34) を抽出する。フォールバック方針も同じ (`hvc1` の仕様準拠入力向け)
- `VideoFormat::H264AnnexB` は既にフレームデータから拾っているため変更なし
- 既存の H.264 NALU パーサ (`crate::video::h264::parse_avcc_sps_pps_lists` 等) と H.265 の NALU 定数 (`H265_NALU_TYPE_VPS` / `H265_NALU_TYPE_SPS` / `H265_NALU_TYPE_PPS`) を活用し、専用のパーサを新設しない方向で検討する

## 完了条件

- 上記の再現手順で作成する concat MP4 を `hisui inspect --decode` に渡したとき、後半 25 フレームにも `width` / `height` / `decoded_data_size` が付き、`output_callback() failed: status=-12909` のログが 0 件になること
- H.265 についても hev1 で同等の再現データを作成し、正しくデコードできることを確認する
- 上記の再現データをテストデータとして加え、`hisui inspect --decode` の JSON 出力で全サンプルに `width` / `height` が付くことを assert する回帰テストを追加すること (配置場所は既存の `testdata/` レイアウトに合わせる)
- 既存のテストが全て通ること

## 関連

- avc3 サポート追加 (in-band パラメータセットを正規に扱うが hisui 全体でハンドラが無い。本 issue のスコープ外)
