enum Subcommand {
    Receive,
    SubscribeProgram,
    SendVideo {
        send_width: i32,
        send_height: i32,
        send_fps: u32,
    },
}

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = "obsws_bootstrap";
    noargs::HELP_FLAG.take_help(&mut args);

    // サブコマンド分岐
    let subcommand;
    if noargs::cmd("receive")
        .doc("WebRTC bootstrap で raw track を受信して MP4 出力する")
        .take(&mut args)
        .is_present()
    {
        subcommand = Subcommand::Receive;
    } else if noargs::cmd("subscribe-program")
        .doc("WebRTC bootstrap で SubscribeProgramTracks を送信し Program track を受信して MP4 出力する")
        .take(&mut args)
        .is_present()
    {
        subcommand = Subcommand::SubscribeProgram;
    } else if noargs::cmd("send-video")
        .doc("WebRTC で映像を送信し webrtc_source input として合成結果を MP4 出力する")
        .take(&mut args)
        .is_present()
    {
        let send_width: i32 = noargs::opt("send-width")
            .default("320")
            .doc("送信映像の幅")
            .take(&mut args)
            .then(|o| o.value().parse())?;
        let send_height: i32 = noargs::opt("send-height")
            .default("320")
            .doc("送信映像の高さ")
            .take(&mut args)
            .then(|o| o.value().parse())?;
        let send_fps: u32 = noargs::opt("send-fps")
            .default("30")
            .doc("送信フレームレート")
            .take(&mut args)
            .then(|o| o.value().parse())?;
        subcommand = Subcommand::SendVideo {
            send_width,
            send_height,
            send_fps,
        };
    } else if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    } else {
        return Ok(());
    }

    // 共通引数
    let verbose = noargs::flag("verbose")
        .short('v')
        .doc("詳細ログを出力する")
        .take(&mut args)
        .is_present();

    let host: String = noargs::opt("host")
        .default("127.0.0.1")
        .doc("接続先ホスト")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let port: u16 = noargs::opt("port")
        .doc("接続先ポート")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let duration: u64 = noargs::opt("duration")
        .default("5")
        .doc("トラック受信を待つ秒数")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let output_path: String = noargs::opt("output-path")
        .doc("MP4 出力先パス")
        .take(&mut args)
        .then(|o| o.value().parse())?;
    let input_mp4_path: String = noargs::opt("input-mp4-path")
        .doc("obsws 経由で入力として追加する MP4 ファイルパス")
        .take(&mut args)
        .then(|o| o.value().parse())?;

    args.finish()?;

    if verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_target(false)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_writer(std::io::stderr)
            .init();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let result = runtime.block_on(async {
        let local = tokio::task::LocalSet::new();
        match subcommand {
            Subcommand::Receive => {
                local
                    .run_until(obsws_bootstrap::client::run_client(
                        &host,
                        port,
                        duration,
                        &output_path,
                        &input_mp4_path,
                        false,
                    ))
                    .await
            }
            Subcommand::SubscribeProgram => {
                local
                    .run_until(obsws_bootstrap::client::run_client(
                        &host,
                        port,
                        duration,
                        &output_path,
                        &input_mp4_path,
                        true,
                    ))
                    .await
            }
            Subcommand::SendVideo {
                send_width,
                send_height,
                send_fps,
            } => {
                local
                    .run_until(obsws_bootstrap::client::run_send_video_client(
                        &host,
                        port,
                        duration,
                        &output_path,
                        &input_mp4_path,
                        send_width,
                        send_height,
                        send_fps,
                    ))
                    .await
            }
        }
    });

    match result {
        Ok(stats) => {
            let json = nojson::object(|f| {
                f.member("video_tracks_received", stats.video_tracks)?;
                f.member("audio_tracks_received", stats.audio_tracks)?;
                f.member("video_frames_received", stats.video_frames)?;
                f.member("audio_frames_received", stats.audio_frames)?;
                f.member("video_width", stats.video_width)?;
                f.member("video_height", stats.video_height)?;
                f.member("video_codec", stats.video_codec.as_str())?;
                f.member("audio_codec", stats.audio_codec.as_str())?;
                f.member("video_samples_written", stats.video_samples_written)?;
                f.member("audio_samples_written", stats.audio_samples_written)?;
                f.member("connection_state", stats.connection_state.as_str())?;
                f.member("webrtc_stats_error", stats.webrtc_stats_error.as_str())?;
                f.member("program_tracks_subscribed", stats.program_tracks_subscribed)
            });
            println!("{json}");
            Ok(())
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
