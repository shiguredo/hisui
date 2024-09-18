# 既知の問題

## macOS, iOS の Safari で録画した H.264 の映像について合成が失敗することがある

OpenH264 のデコード不具合により macOS または iOS の Safari で録画した H.264 の映像を合成した時に失敗する事象を確認しています。
この不具合は、OpenH264 に報告済みです。
- https://github.com/cisco/openh264/pull/3787

この現象は以下の環境で確認しています。

- macOS
  - macOS 15.0
  - Safari 18.0
- iOS
  - iOS 17.5.1
  - Safari 17.5
