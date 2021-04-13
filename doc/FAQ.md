# FAQ

## 複数の CPU コアを用いて映像をエンコードしたい

hisui で利用している [libvpx](https://github.com/webmproject/libvpx/) には, 
いくつかのマルチスレッドを利用する機能がありますが,
hisui のデフォルトでは off にしています.
hisui が生成するそれほど解像度の高くない映像には, あまり効果が見られないためです.

hisui は次のオプションを用意しています. `--libvpx` で始まるものは VP8/VP9 共通で, `--libvp9` で始まるものは VP9 の場合にしか効果がありません.

- `--libvpx-threads`
    - 映像のエンコードに利用するスレッド数を指定します. hisui でのデフォルトは 0 でマルチスレッド機能は無効です.
- `--libvp9-tile-columns`
    - 複数スレッドでのエンコーディングのために映像の列単位での分割を指定します. 2^(指定した数) の分割が行なわれます. hisui でのデフォルトは 0 です.
- `--libvp9-row-mt`
    - 1 を指定すると行ベースの非決定的マルチスレディングが有効になります. hisui でのデフォルトは 0 で無効です.

[Recommended Settings for VOD Media  |  Google Developers](https://developers.google.com/media/vp9/settings/vod) の "Tiling and Threading Recommendations" が
`threads` と `tile-columns` を決定する参考になります.

解像度の高くない映像でも `gamemoderun` コマンドを利用すると, マルチスレッドでのエンコードが高速化する場合がありました.
Ubuntu の場合は次でインストールされます. 

```
sudo apt install gamemode
```

`gamemoderun` やコマンドラインオプションを組合せて録画合成する例を示します.

```
gamemoderun ./hisui -f ~/report-XXX.json --libvpx-threads 4 --libvp9-tile-columns 2 --libvp9-row-mt 1
```

### 参考

- [The WebM Project | VP8 Encode Parameter Guide](https://www.webmproject.org/docs/encoder-parameters/)
- [Recommended Settings for VOD Media  |  Google Developers](https://developers.google.com/media/vp9/settings/vod)
- [mrintrepide/VP9 Encode Guide.md](https://gist.github.com/mrintrepide/3033c35ee9557e66cff7806f48dbd339)
- [VP9 Encoding Guide - wiki](http://wiki.webmproject.org/ffmpeg/vp9-encoding-guide)
- [FeralInteractive/gamemode: Optimise Linux system performance on demand](https://github.com/FeralInteractive/gamemode)
