use std::{num::NonZeroUsize, path::PathBuf};

pub fn run(mut args: noargs::RawArgs) -> noargs::Result<()> {
    let _working_dir: PathBuf = noargs::opt("working-dir")
        .short('w')
        .ty("PATH")
        .default(".")
        .doc(concat!(
            "合成処理を行う際の作業ディレクトリを指定します\n",
            "\n",
            "レイアウトファイル内に記載された相対パスの基点は、このディレクトリとなります。\n",
            "また、レイアウトで、このディレクトリの外のファイルが指定された場合にはエラーとなります。"
        ))
        .take(&mut args)
        .then(|a| a.value().parse())?;
    let _layout_file_path: Option<PathBuf> = noargs::opt("layout-file")
        .short('l')
        .ty("PATH")
        .env("HISUI_LAYOUT_FILE_PATH")
        .doc(
            r#"合成に使用するレイアウトファイルを指定します

省略された場合には、以下の内容のレイアウトで合成が行われます:
{
  "audio_sources": [ "archive*.json" ],
  "video_layout": {
    "max_columns": 3,
    "video_sources": [ "archive*.json" ]
  }
}"#,
        )
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let _output_file_path: Option<PathBuf> = noargs::opt("output-file")
        .short('o')
        .ty("PATH")
        .doc(concat!(
            "合成結果を保存するファイルを指定します\n",
            "\n",
            "この引数が未指定の場合には、 `--working-dir` 引数で\n",
            "指定したディレクトリに `output.mp4` という名前で保存されます"
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let _stats_file_path: Option<PathBuf> = noargs::opt("stats-file")
        .short('s')
        .ty("PATH")
        .doc("合成中に収集した統計情報 (JSON) を保存するファイルを指定します")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let _openh264: Option<PathBuf> = noargs::opt("openh264")
        .ty("PATH")
        .env("HISUI_OPENH264_PATH")
        .doc("OpenH264 の共有ライブラリのパスを指定します")
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    let _progress_bar: bool = noargs::flag("progress-bar")
        .short('p')
        .doc("指定された場合は、合成の進捗を表示します")
        .take(&mut args)
        .is_present();
    let _max_cpu_cores: Option<NonZeroUsize> = noargs::opt("max-cpu-cores")
        .short('c')
        .ty("INTEGER")
        .env("HISUI_MAX_CPU_CORES")
        .doc(concat!(
            "合成処理を行うプロセスが使用するコア数の上限を指定します\n",
            "（未指定時には上限なし）\n",
            "\n",
            "NOTE: macOS ではこの引数は無視されます",
        ))
        .take(&mut args)
        .present_and_then(|a| a.value().parse())?;
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    todo!()
}
