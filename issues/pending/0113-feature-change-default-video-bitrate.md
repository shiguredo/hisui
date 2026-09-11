# 既定の映像ビットレート計算を見直す

- Priority: Medium
- Created: 2026-09-11
- Completed:
- Branch: feature/change-default-video-bitrate
- Polished:

## 目的

現在の既定映像ビットレートは固定値で、合成後の解像度や FPS を考慮していない。解像度 (および FPS) に比例した既定値にすることを検討する。

## pending とした理由

- 計算式の設計と、HLS / MPEG-DASH の variant 既定値との整合の検討が必要。
- 既定値を変えると既存利用者の出力品質・帯域に影響するため、互換性の扱いを決める必要がある。
- 方針が固まるまで `issues/pending/` で保留する。

## 現状

- `src/obsws/coordinator/output.rs` の `start_encoder_processors` は H.264 に `2_000_000` bps、音声に `128_000` bps を直値で渡す。
- `src/obsws/coordinator/output_hls.rs` の `DEFAULT_HLS_VIDEO_BITRATE_BPS` と `src/obsws/coordinator/output_dash.rs` の `DEFAULT_DASH_VIDEO_BITRATE_BPS` も `2_000_000`。
- HLS / DASH の variant の `video_bitrate` は必須で、未指定はエラー。既定値を持つのは `HlsVariant::default` / `DashVariant::default`。
- 解像度・FPS からビットレートを算出するロジックは存在しない。

## 設計方針 (要検討・未確定)

- 解像度 (と FPS) からビットレートを求める関数を追加し、各出力の既定値として使う。
- FPS を考慮に入れるか、解像度のみにするかを決める。
- 既存の固定値をどう移行するか (互換のための上書き指定) を決める。

## 完了条件

- 解像度に応じた既定ビットレートになること。
- 既存の明示指定に回帰がないこと。

(解決方法は pending のため、設計確定後に記載する。)
