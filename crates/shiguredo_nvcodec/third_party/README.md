# third_party ディレクトリについて

third_party ディレクトリは、外部から取得したコードを置いておくためのディレクトリです。

## コードフォーマッタについて

third_party ディレクトリ以下のコードは可能な限り外部から取得した状態を維持し、コードフォーマッタの利用はしません。
これは、ライブラリのアップデート時に不要な差分が出るのを避けることを目的とします。

## third_party/nvcodec について

`third_party/nvcodec` は [NVIDIA Video Codec SDK](https://developer.nvidia.com/video-codec-sdk) から取得したものを使用しています。
