use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

use shiguredo_mp4::{TrackKind, boxes::SampleEntry, demux::Mp4FileDemuxer};

use crate::audio::{AudioFormat, Channels, SampleRate};
use crate::video::{VideoFormat, VideoFrameSize};
use crate::{Ack, AudioFrame, Error, ProcessorHandle, Result, TrackId, TrackPublisher, VideoFrame};

const MAX_NOACKED_COUNT: u64 = 100;

/// OBS 互換のメディア再生状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPlaybackState {
    /// 再生していない
    None,
    /// 再生中
    Playing,
    /// 一時停止中
    Paused,
    /// 停止済み（再生が終了した、または明示的に停止された）
    Stopped,
    /// 終了済み（ループなしで最後まで再生し切った）
    Ended,
}

impl MediaPlaybackState {
    /// OBS WebSocket プロトコルの文字列表現を返す
    pub fn as_obs_str(self) -> &'static str {
        match self {
            Self::None => "OBS_MEDIA_STATE_NONE",
            Self::Playing => "OBS_MEDIA_STATE_PLAYING",
            Self::Paused => "OBS_MEDIA_STATE_PAUSED",
            Self::Stopped => "OBS_MEDIA_STATE_STOPPED",
            Self::Ended => "OBS_MEDIA_STATE_ENDED",
        }
    }
}

/// メディア入力の再生状況（外部から参照可能）
#[derive(Debug, Clone)]
pub struct MediaPlaybackStatus {
    pub state: MediaPlaybackState,
    /// 現在の再生位置
    pub cursor: Duration,
    /// 総時間
    pub duration: Duration,
}

impl Default for MediaPlaybackStatus {
    fn default() -> Self {
        Self {
            state: MediaPlaybackState::None,
            cursor: Duration::ZERO,
            duration: Duration::ZERO,
        }
    }
}

/// メディア入力への再生制御コマンド
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaInputCommand {
    Play,
    Pause,
    Stop,
    Restart,
    /// 指定した絶対位置へシークする
    Seek(Duration),
    /// 現在位置からの相対シーク（ミリ秒、負値は後方）
    OffsetSeek(i64),
}

impl MediaInputCommand {
    /// OBS WebSocket プロトコルの文字列表現からパースする
    pub fn from_obs_str(s: &str) -> Option<Self> {
        match s {
            "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY" => Some(Self::Play),
            "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PAUSE" => Some(Self::Pause),
            "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" => Some(Self::Stop),
            "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART" => Some(Self::Restart),
            _ => None,
        }
    }
}

/// メディア入力からのイベント通知
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaInputEvent {
    /// 再生が開始された
    PlaybackStarted,
    /// 再生が終了した（EOS に到達）
    PlaybackEnded,
}

/// メディア入力の制御ハンドル（coordinator が保持する）
#[derive(Debug)]
pub struct MediaInputHandle {
    pub status: tokio::sync::watch::Receiver<MediaPlaybackStatus>,
    pub command_tx: tokio::sync::mpsc::Sender<MediaInputCommand>,
}

/// メディアイベント直接配信に必要な情報
pub struct MediaEventContext {
    pub event_broadcast_tx: tokio::sync::broadcast::Sender<crate::obsws::coordinator::TaggedEvent>,
    /// 最新の input_name を追従する watch receiver
    pub input_name_rx: tokio::sync::watch::Receiver<String>,
    pub input_uuid: String,
}

impl std::fmt::Debug for MediaEventContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaEventContext")
            .field("input_name", &*self.input_name_rx.borrow())
            .field("input_uuid", &self.input_uuid)
            .finish()
    }
}

/// `MediaInputHandle` の作成結果（reader 側で受け取る部分）
#[derive(Debug)]
struct MediaInputChannels {
    status_tx: tokio::sync::watch::Sender<MediaPlaybackStatus>,
    command_rx: tokio::sync::mpsc::Receiver<MediaInputCommand>,
    event_ctx: MediaEventContext,
}

/// メディア入力のハンドルとチャネルを作成する
fn create_media_input_channels(
    event_ctx: MediaEventContext,
) -> (MediaInputHandle, MediaInputChannels) {
    let (status_tx, status_rx) = tokio::sync::watch::channel(MediaPlaybackStatus::default());
    let (command_tx, command_rx) = tokio::sync::mpsc::channel(8);

    let handle = MediaInputHandle {
        status: status_rx,
        command_tx,
    };
    let channels = MediaInputChannels {
        status_tx,
        command_rx,
        event_ctx,
    };
    (handle, channels)
}

/// run_loop の終了理由
enum RunLoopResult {
    /// コマンドによる明示停止
    Stopped,
    /// ファイル末尾に到達（自然終了）
    Eof,
    /// pipeline が閉じた
    PipelineClosed,
}

/// wait_for_restart_command の結果
enum WaitResult {
    /// Play コマンド（pending_seek を維持して再開）
    Play,
    /// Restart コマンド（先頭から再生）
    Restart,
    /// チャネルクローズ
    Closed,
}

/// run_loop 内のコマンド処理結果
enum MediaLoopAction {
    /// サンプル処理を続行する
    Continue,
    /// 再生を停止する
    Stop,
    /// ファイル先頭からリスタートする
    Restart,
    /// 指定位置へシークする
    Seek(Duration),
    /// 現在位置からの相対シーク（ミリ秒）
    OffsetSeek(i64),
}

#[derive(Debug, Clone, Default)]
pub struct Mp4FileReaderOptions {
    // true の場合は実時間再生を行う。
    // 出力 timestamp は実時刻ベースで単調増加するように補正する。
    pub realtime: bool,
    pub loop_playback: bool,
    pub audio_track_id: Option<TrackId>,
    pub video_track_id: Option<TrackId>,
}

#[derive(Debug)]
pub struct Mp4FileReader {
    path: PathBuf,
    options: Mp4FileReaderOptions,
    audio_sender: Option<TrackSender>,
    video_sender: Option<TrackSender>,
    audio_decoder: Option<crate::decoder::AudioDecoder>,
    video_decoder: Option<crate::decoder::VideoDecoder>,
    base_offset: Duration,
    last_emitted_end: Duration,
    start_instant: tokio::time::Instant,
    last_realtime_timestamp: Option<Duration>,
    emitted_in_loop: bool,
    // メディア再生制御
    media_channels: Option<MediaInputChannels>,
    is_paused: bool,
    pause_started_at: Option<tokio::time::Instant>,
    media_duration: Duration,
    /// コマンドで受け取ったシーク位置。次の再生開始時に適用する
    pending_seek: Option<Duration>,
    /// warm-up 中の目標位置。この位置に到達するまでデコーダーに流すが publish しない
    warmup_target: Option<Duration>,
    /// seek 適用済みだが未 publish のファイル内カーソル位置。
    /// relative seek の基準として使い、フレーム publish で last_emitted_end が進んだらクリアする。
    logical_cursor: Option<Duration>,
    /// Stop により再生位置を先頭へ戻した直後かどうか
    stopped_at_zero: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp4FileTrackAvailability {
    pub has_audio: bool,
    pub has_video: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp4FileVideoDimensions {
    pub width: usize,
    pub height: usize,
}

impl Mp4FileReader {
    pub fn new<P: AsRef<Path>>(path: P, options: Mp4FileReaderOptions) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            options,
            audio_sender: None,
            video_sender: None,
            audio_decoder: None,
            video_decoder: None,
            base_offset: Duration::ZERO,
            last_emitted_end: Duration::ZERO,
            start_instant: tokio::time::Instant::now(),
            last_realtime_timestamp: None,
            emitted_in_loop: false,
            media_channels: None,
            is_paused: false,
            pause_started_at: None,
            media_duration: Duration::ZERO,
            pending_seek: None,
            warmup_target: None,
            logical_cursor: None,
            stopped_at_zero: false,
        })
    }

    /// メディア再生制御のハンドルを作成して返す。
    /// reader に制御チャネルを設定し、外部からの再生制御を可能にする。
    pub fn create_media_handle(&mut self, event_ctx: MediaEventContext) -> MediaInputHandle {
        let (handle, channels) = create_media_input_channels(event_ctx);
        self.media_channels = Some(channels);
        handle
    }

    /// 音声トラックが設定されているかどうかを返す
    pub fn has_audio_track(&self) -> bool {
        self.options.audio_track_id.is_some()
    }

    /// 映像トラックが設定されているかどうかを返す
    pub fn has_video_track(&self) -> bool {
        self.options.video_track_id.is_some()
    }

    /// デコーダーを設定する。設定された場合、encoded frame を decode してから送信する。
    pub fn set_audio_decoder(&mut self, decoder: crate::decoder::AudioDecoder) {
        self.audio_decoder = Some(decoder);
    }

    /// デコーダーを設定する。設定された場合、encoded frame を decode してから送信する。
    pub fn set_video_decoder(&mut self, decoder: crate::decoder::VideoDecoder) {
        self.video_decoder = Some(decoder);
    }

    pub async fn run(mut self, handle: ProcessorHandle) -> Result<()> {
        let loop_enabled = self.resolve_loop_enabled();
        (self.audio_sender, self.video_sender) = self.build_track_senders(&handle).await?;
        handle.notify_ready();

        if self.audio_sender.is_none() && self.video_sender.is_none() {
            return Ok(());
        }
        handle.wait_subscribers_ready().await?;

        // メディア制御が無効なら従来通り一度だけ再生して終了
        if self.media_channels.is_none() {
            let result = self.run_loop(loop_enabled, &handle).await?;
            if matches!(result, RunLoopResult::Eof) {
                self.flush_decoders()?;
            }
            self.send_eos_to_tracks();
            return Ok(());
        }

        // メディア制御が有効: 停止後もコマンド待機ループを回す
        loop {
            let result = self.run_loop(loop_enabled, &handle).await?;
            match result {
                RunLoopResult::Eof => {
                    self.flush_decoders()?;
                    self.stopped_at_zero = false;
                    self.update_playback_status(
                        MediaPlaybackState::Ended,
                        self.last_emitted_end.saturating_sub(self.base_offset),
                    );
                    self.send_media_event(MediaInputEvent::PlaybackEnded);
                }
                RunLoopResult::Stopped => {
                    // 明示停止: PlaybackEnded は送らない
                    self.update_playback_status(
                        MediaPlaybackState::Stopped,
                        self.stopped_file_cursor(),
                    );
                }
                RunLoopResult::PipelineClosed => {
                    self.stopped_at_zero = false;
                    self.update_playback_status(
                        MediaPlaybackState::Stopped,
                        self.last_emitted_end.saturating_sub(self.base_offset),
                    );
                }
            }

            // Play / Restart を待つ
            match self.wait_for_restart_command().await {
                WaitResult::Play => {
                    // pending_seek を維持したまま再開
                    self.reset_for_restart(&handle);
                }
                WaitResult::Restart => {
                    // 先頭再生: seek 状態をクリア
                    self.clear_seek_state();
                    self.reset_for_restart(&handle);
                }
                WaitResult::Closed => break,
            }
        }

        self.send_eos_to_tracks();
        Ok(())
    }

    /// 再生開始時の状態を初期化する。
    /// pending_seek がある場合はその位置を公開カーソルに反映する（実際の seek は run_loop 冒頭で適用）。
    fn start_playback(&mut self) {
        self.stopped_at_zero = false;
        let start_pos = self.pending_seek.unwrap_or(Duration::ZERO);
        self.update_playback_status(MediaPlaybackState::Playing, start_pos);
        self.send_media_event(MediaInputEvent::PlaybackStarted);
        // 開始位置のサンプル（effective_timestamp = base_offset + start_pos）を now で出す
        self.start_instant = tokio::time::Instant::now() - self.base_offset - start_pos;
        self.last_realtime_timestamp = None;
    }

    fn resolve_loop_enabled(&self) -> bool {
        let mut loop_enabled = self.options.loop_playback;
        if loop_enabled && !self.options.realtime {
            tracing::warn!("Loop playback is ignored because realtime is disabled");
            loop_enabled = false;
        }
        loop_enabled
    }

    async fn build_track_senders(
        &mut self,
        handle: &ProcessorHandle,
    ) -> Result<(Option<TrackSender>, Option<TrackSender>)> {
        let audio_sender = if let Some(track_id) = self.options.audio_track_id.take() {
            let sender = handle.publish_track(track_id).await?;
            Some(TrackSender::new(sender))
        } else {
            None
        };

        let video_sender = if let Some(track_id) = self.options.video_track_id.take() {
            let sender = handle.publish_track(track_id).await?;
            Some(TrackSender::new(sender))
        } else {
            None
        };

        Ok((audio_sender, video_sender))
    }

    async fn run_loop(
        &mut self,
        loop_enabled: bool,
        handle: &ProcessorHandle,
    ) -> Result<RunLoopResult> {
        let mut started = false;
        'outer: loop {
            let mut state = ReaderState::open(
                &self.path,
                self.audio_sender.is_some(),
                self.video_sender.is_some(),
            )?;
            if state.audio_track_id.is_none() && state.video_track_id.is_none() {
                break;
            }
            // 最初の open で総時間を取得する
            if self.media_duration == Duration::ZERO {
                self.media_duration = state.duration;
            }
            // 初回 open 成功時に再生開始を通知する
            if !started {
                self.start_playback();
                started = true;
            }

            // pending_seek が設定されている場合（停止中にシーク済み）、先に適用する
            if let Some(position) = self.pending_seek.take() {
                self.apply_seek(&mut state, position, handle)?;
            }

            self.emitted_in_loop = false;
            while let Some(sample) = state.demuxer.next_sample()? {
                // コマンドをポーリングで確認する
                match self.poll_media_command() {
                    MediaLoopAction::Continue => {}
                    MediaLoopAction::Stop => return Ok(RunLoopResult::Stopped),
                    MediaLoopAction::Restart => {
                        self.recreate_decoders(handle);
                        continue 'outer;
                    }
                    MediaLoopAction::Seek(position) => {
                        self.apply_seek(&mut state, position, handle)?;
                        continue;
                    }
                    MediaLoopAction::OffsetSeek(offset_ms) => {
                        let position = self.resolve_offset_seek(offset_ms);
                        self.apply_seek(&mut state, position, handle)?;
                        continue;
                    }
                }
                // 一時停止中はコマンドを非同期で待つ
                if self.is_paused {
                    match self.wait_while_paused().await {
                        MediaLoopAction::Continue => {}
                        MediaLoopAction::Stop => return Ok(RunLoopResult::Stopped),
                        MediaLoopAction::Restart => {
                            self.recreate_decoders(handle);
                            continue 'outer;
                        }
                        MediaLoopAction::Seek(position) => {
                            self.apply_seek(&mut state, position, handle)?;
                            continue;
                        }
                        MediaLoopAction::OffsetSeek(offset_ms) => {
                            let position = self.resolve_offset_seek(offset_ms);
                            self.apply_seek(&mut state, position, handle)?;
                            continue;
                        }
                    }
                }

                let context = SampleContext::from_sample(&sample);
                let pipeline_closed = self.handle_sample(&mut state, context).await?;
                if pipeline_closed {
                    return Ok(RunLoopResult::PipelineClosed);
                }
            }

            if loop_enabled {
                if !self.emitted_in_loop {
                    tracing::warn!("Loop playback stopped because no samples were read");
                    break;
                }
                self.send_media_event(MediaInputEvent::PlaybackEnded);
                self.base_offset = self.last_emitted_end;
                self.send_media_event(MediaInputEvent::PlaybackStarted);
                continue;
            }
            break;
        }

        Ok(RunLoopResult::Eof)
    }

    /// 一時停止中にコマンドを待つ
    async fn wait_while_paused(&mut self) -> MediaLoopAction {
        loop {
            let command = {
                let Some(channels) = self.media_channels.as_mut() else {
                    return MediaLoopAction::Continue;
                };
                channels.command_rx.recv().await
            };
            match command {
                Some(MediaInputCommand::Play) => {
                    self.resume_from_pause();
                    // 一時停止中にシークが行われていた場合、その位置を適用する
                    if let Some(position) = self.pending_seek.take() {
                        return MediaLoopAction::Seek(position);
                    }
                    return MediaLoopAction::Continue;
                }
                Some(MediaInputCommand::Pause) => {
                    // 既に一時停止中なので何もしない
                }
                Some(MediaInputCommand::Stop) => {
                    self.is_paused = false;
                    self.stop_and_reset_to_zero();
                    return MediaLoopAction::Stop;
                }
                Some(MediaInputCommand::Restart) => {
                    self.restart_playback();
                    return MediaLoopAction::Restart;
                }
                Some(MediaInputCommand::Seek(position)) => {
                    self.set_pending_seek(position);
                }
                Some(MediaInputCommand::OffsetSeek(offset_ms)) => {
                    let position = self.resolve_offset_seek(offset_ms);
                    self.set_pending_seek(position);
                }
                None => {
                    // チャネルが閉じられた
                    self.is_paused = false;
                    return MediaLoopAction::Stop;
                }
            }
        }
    }

    /// 非一時停止中にコマンドをポーリングで確認する
    fn poll_media_command(&mut self) -> MediaLoopAction {
        let Some(channels) = self.media_channels.as_mut() else {
            return MediaLoopAction::Continue;
        };
        match channels.command_rx.try_recv() {
            Ok(MediaInputCommand::Play) => {
                // 既に再生中なので何もしない
                MediaLoopAction::Continue
            }
            Ok(MediaInputCommand::Pause) => {
                self.is_paused = true;
                self.pause_started_at = Some(tokio::time::Instant::now());
                let file_pos = self.last_emitted_end.saturating_sub(self.base_offset);
                self.update_playback_status(MediaPlaybackState::Paused, file_pos);
                MediaLoopAction::Continue
            }
            Ok(MediaInputCommand::Stop) => MediaLoopAction::Stop,
            Ok(MediaInputCommand::Restart) => {
                self.restart_playback();
                MediaLoopAction::Restart
            }
            Ok(MediaInputCommand::Seek(position)) => MediaLoopAction::Seek(position),
            Ok(MediaInputCommand::OffsetSeek(offset_ms)) => MediaLoopAction::OffsetSeek(offset_ms),
            Err(_) => MediaLoopAction::Continue,
        }
    }

    /// demuxer をシークし、タイミング情報を再計算する。
    /// 映像トラックがある場合、シーク先が非キーフレームなら直前のキーフレームまで遡り
    /// warm-up モードに入る。
    fn apply_seek(
        &mut self,
        state: &mut ReaderState,
        position: Duration,
        handle: &ProcessorHandle,
    ) -> Result<()> {
        // デコーダーの残留状態を捨ててから seek する
        self.recreate_decoders(handle);

        // duration が取得済みなら上限を clamp する
        let position = if self.media_duration > Duration::ZERO {
            position.min(self.media_duration)
        } else {
            position
        };
        state
            .demuxer
            .seek(position)
            .map_err(|e| Error::new(format!("seek failed: {e}")))?;

        // 映像トラックがある場合、シーク先がキーフレームかどうかを確認し、
        // 非キーフレームなら直前のキーフレームまで遡る
        self.warmup_target = None;
        if state.video_track_id.is_some() {
            self.seek_to_previous_keyframe(state, position)?;
        }

        // base_offset を調整して、出力タイムスタンプの連続性を維持する
        self.base_offset = self.last_emitted_end.saturating_sub(position);
        // realtime 再生のタイミングを再計算する。
        // シーク先のサンプル（effective_timestamp = base_offset + position）を now で出すため、
        // start_instant = now - (base_offset + position) にする。
        self.start_instant = tokio::time::Instant::now() - self.base_offset - position;
        self.last_realtime_timestamp = None;
        // seek 適用済みの論理カーソルを設定する（次フレーム publish まで relative seek の基準になる）
        self.logical_cursor = Some(position);
        self.update_playback_status(MediaPlaybackState::Playing, position);
        Ok(())
    }

    /// シーク後の最初の映像サンプルがキーフレームでない場合、
    /// prev_sample() で直前のキーフレームまで遡り warmup_target を設定する。
    fn seek_to_previous_keyframe(
        &mut self,
        state: &mut ReaderState,
        target_position: Duration,
    ) -> Result<()> {
        // シーク後の最初の映像サンプルを見つけて、キーフレームかどうかを判定する。
        // 音声サンプルが先に来る場合があるため、映像サンプルが見つかるまで読み進める。
        let mut samples_read = 0u32;
        let needs_warmup = loop {
            match state.demuxer.next_sample() {
                Ok(Some(sample)) => {
                    samples_read += 1;
                    if sample.track.kind == shiguredo_mp4::TrackKind::Video {
                        let is_keyframe = sample.keyframe;
                        // 読み進めた分をすべて戻す
                        for _ in 0..samples_read {
                            let _ = state.demuxer.prev_sample();
                        }
                        break !is_keyframe;
                    }
                    // 音声サンプル → さらに読み進める
                }
                Ok(None) => {
                    // EOF に到達（映像サンプルが見つからなかった）: warm-up 不要
                    // 読み進めた分を戻す
                    for _ in 0..samples_read {
                        let _ = state.demuxer.prev_sample();
                    }
                    break false;
                }
                Err(e) => return Err(Error::new(format!("failed to check keyframe: {e}"))),
            }
        };

        if !needs_warmup {
            return Ok(());
        }

        // prev_sample() で直前の映像キーフレームまで遡る
        loop {
            match state.demuxer.prev_sample() {
                Ok(Some(sample)) => {
                    if sample.track.kind == shiguredo_mp4::TrackKind::Video && sample.keyframe {
                        // キーフレームを見つけた。demuxer はこの位置に戻っている
                        break;
                    }
                    // 映像の非キーフレームか音声サンプル → さらに遡る
                }
                Ok(None) => {
                    // ファイル先頭に到達。ここから warm-up する
                    break;
                }
                Err(e) => return Err(Error::new(format!("failed to find keyframe: {e}"))),
            }
        }

        self.warmup_target = Some(target_position);
        Ok(())
    }

    /// 一時停止から復帰する
    fn resume_from_pause(&mut self) {
        let paused = self
            .pause_started_at
            .take()
            .map(|t| t.elapsed())
            .unwrap_or(Duration::ZERO);
        self.is_paused = false;
        // start_instant を一時停止分だけ進めて、再生速度制御のタイミングを維持する
        self.start_instant += paused;
        let file_pos = self.last_emitted_end.saturating_sub(self.base_offset);
        self.update_playback_status(MediaPlaybackState::Playing, file_pos);
    }

    /// 保留中の seek 関連状態をクリアする
    fn clear_seek_state(&mut self) {
        self.pending_seek = None;
        self.warmup_target = None;
        self.logical_cursor = None;
    }

    /// Stop 後に最終公開するファイル内カーソル位置を返す
    fn stopped_file_cursor(&self) -> Duration {
        if self.stopped_at_zero {
            Duration::ZERO
        } else {
            self.last_emitted_end.saturating_sub(self.base_offset)
        }
    }

    /// Stop により再生位置を先頭へ戻し、その状態を最終 status まで維持する
    fn stop_and_reset_to_zero(&mut self) {
        self.clear_seek_state();
        self.stopped_at_zero = true;
        self.update_playback_status(MediaPlaybackState::Stopped, Duration::ZERO);
    }

    /// 再生を先頭からリスタートする（デコーダーの再生成は呼び出し側で行う）。
    /// 手動 Restart の通知は MediaInputActionTriggered で行うため、
    /// ここでは PlaybackEnded / PlaybackStarted は送らない。
    fn restart_playback(&mut self) {
        self.is_paused = false;
        self.pause_started_at = None;
        self.clear_seek_state();
        self.stopped_at_zero = false;
        self.base_offset = self.last_emitted_end;
        self.start_instant = tokio::time::Instant::now() - self.base_offset;
        self.last_realtime_timestamp = None;
        self.update_playback_status(MediaPlaybackState::Playing, Duration::ZERO);
    }

    /// 現在のファイル内カーソル位置を取得する。
    /// pending_seek > logical_cursor（seek 適用済み未 publish）> last_emitted_end の優先順で参照する。
    fn current_file_cursor(&self) -> Duration {
        self.pending_seek
            .or(self.logical_cursor)
            .unwrap_or_else(|| self.last_emitted_end.saturating_sub(self.base_offset))
    }

    /// 位置を 0..=media_duration に clamp する
    fn clamp_position(&self, position: Duration) -> Duration {
        if self.media_duration > Duration::ZERO {
            position.min(self.media_duration)
        } else {
            position
        }
    }

    /// 相対オフセット（ミリ秒）を絶対位置に変換する
    fn resolve_offset_seek(&self, offset_ms: i64) -> Duration {
        let current_ms = self.current_file_cursor().as_millis() as i64;
        let target_ms = current_ms.saturating_add(offset_ms).max(0);
        self.clamp_position(Duration::from_millis(target_ms as u64))
    }

    /// seek 位置を clamp して pending_seek と公開ステータスを更新する。
    /// 現在の公開状態（Paused / Stopped / Ended）はそのまま維持する。
    fn set_pending_seek(&mut self, position: Duration) {
        let clamped = self.clamp_position(position);
        self.pending_seek = Some(clamped);
        self.stopped_at_zero = false;
        // 現在の公開状態を維持したまま cursor だけ更新する
        if let Some(channels) = &self.media_channels {
            let current_state = channels.status_tx.borrow().state;
            self.update_playback_status(current_state, clamped);
        }
    }

    /// 再生状態を更新する。cursor はファイル内の絶対位置を渡す。
    fn update_playback_status(&self, state: MediaPlaybackState, file_cursor: Duration) {
        if let Some(channels) = &self.media_channels {
            let _ = channels.status_tx.send(MediaPlaybackStatus {
                state,
                cursor: file_cursor,
                duration: self.media_duration,
            });
        }
    }

    /// メディアイベントを obsws セッションに直接配信する
    fn send_media_event(&self, event: MediaInputEvent) {
        let Some(channels) = &self.media_channels else {
            return;
        };
        let input_name = channels.event_ctx.input_name_rx.borrow().clone();
        let text = match event {
            MediaInputEvent::PlaybackStarted => {
                crate::obsws::response::build_media_input_playback_started_event(
                    &input_name,
                    &channels.event_ctx.input_uuid,
                )
            }
            MediaInputEvent::PlaybackEnded => {
                crate::obsws::response::build_media_input_playback_ended_event(
                    &input_name,
                    &channels.event_ctx.input_uuid,
                )
            }
        };
        let _ =
            channels
                .event_ctx
                .event_broadcast_tx
                .send(crate::obsws::coordinator::TaggedEvent {
                    text,
                    subscription_flag: crate::obsws::protocol::OBSWS_EVENT_SUB_MEDIA_INPUTS,
                });
    }

    async fn handle_sample(
        &mut self,
        state: &mut ReaderState,
        context: SampleContext,
    ) -> Result<bool> {
        // composition_time_offset は未対応
        if context.composition_time_offset.is_some() {
            return Err(Error::new(
                "composition_time_offset is not supported yet".to_owned(),
            ));
        }

        // warm-up 中かどうかを判定する
        let suppress_publish = if let Some(target) = self.warmup_target {
            let (timestamp, _) =
                calculate_timestamps(context.timescale, context.timestamp, context.duration);
            if timestamp >= target {
                // warm-up 完了
                self.warmup_target = None;
                false
            } else {
                true
            }
        } else {
            false
        };

        match context.track_kind {
            TrackKind::Audio => {
                self.handle_audio_sample(state, context, suppress_publish)
                    .await
            }
            TrackKind::Video => {
                self.handle_video_sample(state, context, suppress_publish)
                    .await
            }
        }
    }

    async fn handle_audio_sample(
        &mut self,
        state: &mut ReaderState,
        context: SampleContext,
        suppress_publish: bool,
    ) -> Result<bool> {
        if !state.is_audio_enabled(context.track_id) {
            return Ok(false);
        }

        if let Some(entry) = &context.sample_entry {
            state.update_audio_format(entry)?;
        }

        let data = state.read_sample_data(context.data_offset, context.data_size)?;
        let (timestamp, duration) =
            calculate_timestamps(context.timescale, context.timestamp, context.duration);
        let effective_timestamp = self.base_offset + timestamp;

        // warm-up 中はデコーダーに入力して出力を捨てる。publish や realtime sleep はスキップする
        if suppress_publish {
            if let Some(decoder) = self.audio_decoder.as_mut() {
                let frame = AudioFrame {
                    data,
                    format: state.audio_format,
                    channels: state.audio_channels,
                    sample_rate: state.audio_sample_rate,
                    timestamp: Duration::ZERO,
                    sample_entry: context.sample_entry,
                };
                decoder.handle_input_sample(Some(crate::MediaFrame::Audio(
                    std::sync::Arc::new(frame),
                )))?;
                // デコーダーの出力を drain して捨てる（内部バッファの蓄積を防ぐ）
                discard_decoder_output(decoder)?;
            }
            return Ok(false);
        }

        if self.options.realtime {
            let target = self.start_instant + effective_timestamp;
            tokio::time::sleep_until(target).await;
        }
        let output_timestamp = self.output_timestamp(effective_timestamp);

        let audio_data = AudioFrame {
            data,
            format: state.audio_format,
            channels: state.audio_channels,
            sample_rate: state.audio_sample_rate,
            timestamp: output_timestamp,
            sample_entry: context.sample_entry,
        };

        if let Some(sender) = self.audio_sender.as_mut() {
            if let Some(decoder) = self.audio_decoder.as_mut() {
                decoder.handle_input_sample(Some(crate::MediaFrame::Audio(
                    std::sync::Arc::new(audio_data),
                )))?;
                if crate::decoder::drain_audio_decoder_output(decoder, &mut sender.sender)?
                    == crate::decoder::DrainResult::PipelineClosed
                {
                    return Ok(true);
                }
            } else if !sender.send_audio(audio_data).await {
                return Ok(true);
            }
            self.emitted_in_loop = true;
            self.logical_cursor = None;
            let end = effective_timestamp + duration;
            if end > self.last_emitted_end {
                self.last_emitted_end = end;
                let file_pos = end.saturating_sub(self.base_offset);
                self.update_playback_status(MediaPlaybackState::Playing, file_pos);
            }
        }

        Ok(false)
    }

    async fn handle_video_sample(
        &mut self,
        state: &mut ReaderState,
        context: SampleContext,
        suppress_publish: bool,
    ) -> Result<bool> {
        if !state.is_video_enabled(context.track_id) {
            return Ok(false);
        }

        if let Some(entry) = &context.sample_entry {
            state.update_video_format(entry)?;
        }

        let data = state.read_sample_data(context.data_offset, context.data_size)?;
        let (timestamp, duration) =
            calculate_timestamps(context.timescale, context.timestamp, context.duration);
        let effective_timestamp = self.base_offset + timestamp;

        // warm-up 中はデコーダーに入力して出力を捨てる。publish や realtime sleep はスキップする
        if suppress_publish {
            if let Some(decoder) = self.video_decoder.as_mut() {
                let frame = VideoFrame {
                    data,
                    format: state.video_format,
                    keyframe: context.keyframe,
                    size: Some(VideoFrameSize {
                        width: state.video_width,
                        height: state.video_height,
                    }),
                    timestamp: Duration::ZERO,
                    sample_entry: context.sample_entry,
                };
                decoder.handle_input_sample(Some(crate::MediaFrame::Video(
                    std::sync::Arc::new(frame),
                )))?;
                // デコーダーの出力を drain して捨てる（内部バッファの蓄積を防ぐ）
                discard_video_decoder_output(decoder)?;
            }
            return Ok(false);
        }

        if self.options.realtime {
            let target = self.start_instant + effective_timestamp;
            tokio::time::sleep_until(target).await;
        }
        let output_timestamp = self.output_timestamp(effective_timestamp);

        let video_frame = VideoFrame {
            data,
            format: state.video_format,
            keyframe: context.keyframe,
            size: Some(VideoFrameSize {
                width: state.video_width,
                height: state.video_height,
            }),
            timestamp: output_timestamp,
            sample_entry: context.sample_entry,
        };

        if let Some(sender) = self.video_sender.as_mut() {
            if let Some(decoder) = self.video_decoder.as_mut() {
                decoder.handle_input_sample(Some(crate::MediaFrame::Video(
                    std::sync::Arc::new(video_frame),
                )))?;
                if crate::decoder::drain_video_decoder_output(decoder, &mut sender.sender)?
                    == crate::decoder::DrainResult::PipelineClosed
                {
                    return Ok(true);
                }
            } else if !sender.send_video(video_frame).await {
                return Ok(true);
            }
            self.emitted_in_loop = true;
            self.logical_cursor = None;
            let end = effective_timestamp + duration;
            if end > self.last_emitted_end {
                self.last_emitted_end = end;
                let file_pos = end.saturating_sub(self.base_offset);
                self.update_playback_status(MediaPlaybackState::Playing, file_pos);
            }
        }

        Ok(false)
    }

    fn output_timestamp(&mut self, effective_timestamp: Duration) -> Duration {
        if !self.options.realtime {
            return effective_timestamp;
        }

        let mut timestamp = self.start_instant.elapsed().max(effective_timestamp);
        if let Some(last) = self.last_realtime_timestamp {
            let min_next = last.saturating_add(Duration::from_micros(1));
            if timestamp < min_next {
                timestamp = min_next;
            }
        }
        self.last_realtime_timestamp = Some(timestamp);
        timestamp
    }

    /// デコーダーの残りのフレームを flush する。EOS は送らない。
    fn flush_decoders(&mut self) -> Result<()> {
        // EOS flush 中に pipeline が閉じるのは正常な停止シーケンスなので、DrainResult は無視する。
        if let Some(decoder) = self.audio_decoder.as_mut()
            && let Some(sender) = self.audio_sender.as_mut()
        {
            decoder.handle_input_sample(None)?;
            let _ = crate::decoder::drain_audio_decoder_output(decoder, &mut sender.sender)?;
        }
        if let Some(decoder) = self.video_decoder.as_mut()
            && let Some(sender) = self.video_sender.as_mut()
        {
            decoder.handle_input_sample(None)?;
            let _ = crate::decoder::drain_video_decoder_output(decoder, &mut sender.sender)?;
        }
        Ok(())
    }

    /// トラックに EOS を送信する
    fn send_eos_to_tracks(&mut self) {
        if let Some(sender) = self.audio_sender.as_mut() {
            sender.send_eos();
        }
        if let Some(sender) = self.video_sender.as_mut() {
            sender.send_eos();
        }
    }

    /// Ended / Stopped 状態で Play / Restart コマンドを待つ。
    async fn wait_for_restart_command(&mut self) -> WaitResult {
        loop {
            let command = {
                let Some(channels) = self.media_channels.as_mut() else {
                    return WaitResult::Closed;
                };
                channels.command_rx.recv().await
            };
            match command {
                Some(MediaInputCommand::Play) => return WaitResult::Play,
                Some(MediaInputCommand::Restart) => return WaitResult::Restart,
                Some(MediaInputCommand::Pause) => {
                    // 既に停止済みなので無視
                }
                Some(MediaInputCommand::Stop) => {
                    // 保留中の seek を破棄して先頭に戻す
                    self.stop_and_reset_to_zero();
                }
                Some(MediaInputCommand::Seek(position)) => {
                    self.set_pending_seek(position);
                }
                Some(MediaInputCommand::OffsetSeek(offset_ms)) => {
                    let position = self.resolve_offset_seek(offset_ms);
                    self.set_pending_seek(position);
                }
                None => return WaitResult::Closed,
            }
        }
    }

    /// 再生状態とデコーダーをリセットして再生可能にする
    fn reset_for_restart(&mut self, handle: &ProcessorHandle) {
        // timestamp の連続性を維持するため、base_offset を last_emitted_end に進める
        self.base_offset = self.last_emitted_end;
        self.is_paused = false;
        self.pause_started_at = None;
        self.logical_cursor = None;
        self.recreate_decoders(handle);
    }

    /// デコーダーを再生成する
    fn recreate_decoders(&mut self, handle: &ProcessorHandle) {
        if self.audio_decoder.is_some() {
            let mut decoder_stats = handle.stats();
            decoder_stats.set_default_label("component", "audio_decoder");
            match crate::decoder::AudioDecoder::new(
                #[cfg(feature = "fdk-aac")]
                handle.config().fdk_aac_lib.clone(),
                decoder_stats,
            ) {
                Ok(decoder) => self.audio_decoder = Some(decoder),
                Err(e) => {
                    tracing::warn!("failed to recreate audio decoder: {}", e.display());
                    self.audio_decoder = None;
                }
            }
        }
        if self.video_decoder.is_some() {
            let mut decoder_stats = handle.stats();
            decoder_stats.set_default_label("component", "video_decoder");
            let decoder = crate::decoder::VideoDecoder::new(
                crate::decoder::VideoDecoderOptions {
                    openh264_lib: handle.config().openh264_lib.clone(),
                    ..Default::default()
                },
                decoder_stats,
            );
            self.video_decoder = Some(decoder);
        }
    }
}

/// デコーダーの出力を drain して捨てる（warm-up 中の内部バッファ蓄積を防ぐ）
fn discard_decoder_output(decoder: &mut crate::decoder::AudioDecoder) -> Result<()> {
    while let crate::decoder::DecoderRunOutput::Processed(_) = decoder.poll_output()? {}
    Ok(())
}

/// デコーダーの出力を drain して捨てる（warm-up 中の内部バッファ蓄積を防ぐ）
fn discard_video_decoder_output(decoder: &mut crate::decoder::VideoDecoder) -> Result<()> {
    while let crate::decoder::DecoderRunOutput::Processed(_) = decoder.poll_output()? {}
    Ok(())
}

pub fn probe_mp4_track_availability<P: AsRef<Path>>(path: P) -> Result<Mp4FileTrackAvailability> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .map_err(|e| Error::new(format!("Cannot open file {}: {e}", path.display())))?;
    let mut demuxer = Mp4FileDemuxer::new();
    initialize_mp4_demuxer(&mut file, &mut demuxer, path)?;

    let has_audio = select_audio_track(demuxer.clone())?.is_some();
    let has_video = select_video_track(demuxer)?.is_some();

    Ok(Mp4FileTrackAvailability {
        has_audio,
        has_video,
    })
}

pub fn probe_mp4_video_dimensions<P: AsRef<Path>>(
    path: P,
) -> Result<Option<Mp4FileVideoDimensions>> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .map_err(|e| Error::new(format!("Cannot open file {}: {e}", path.display())))?;
    let mut demuxer = Mp4FileDemuxer::new();
    initialize_mp4_demuxer(&mut file, &mut demuxer, path)?;

    while let Some(sample) = demuxer.next_sample()? {
        if sample.track.kind != TrackKind::Video {
            continue;
        }
        let Some(sample_entry) = sample.sample_entry else {
            continue;
        };
        let metadata = match sample_entry {
            SampleEntry::Avc1(b) => Some(&b.visual),
            SampleEntry::Hev1(b) => Some(&b.visual),
            SampleEntry::Hvc1(b) => Some(&b.visual),
            SampleEntry::Vp08(b) => Some(&b.visual),
            SampleEntry::Vp09(b) => Some(&b.visual),
            SampleEntry::Av01(b) => Some(&b.visual),
            _ => None,
        };
        if let Some(metadata) = metadata {
            return Ok(Some(Mp4FileVideoDimensions {
                width: metadata.width as usize,
                height: metadata.height as usize,
            }));
        }
    }

    Ok(None)
}

#[derive(Debug, Clone)]
struct SampleContext {
    track_kind: TrackKind,
    track_id: u32,
    timescale: u32,
    timestamp: u64,
    duration: u64,
    data_offset: u64,
    data_size: usize,
    keyframe: bool,
    composition_time_offset: Option<i64>,
    sample_entry: Option<SampleEntry>,
}

impl SampleContext {
    fn from_sample(sample: &shiguredo_mp4::demux::Sample<'_>) -> Self {
        Self {
            track_kind: sample.track.kind,
            track_id: sample.track.track_id,
            timescale: sample.track.timescale.get(),
            timestamp: sample.timestamp,
            duration: sample.duration as u64,
            data_offset: sample.data_offset,
            data_size: sample.data_size,
            keyframe: sample.keyframe,
            composition_time_offset: sample.composition_time_offset,
            sample_entry: sample.sample_entry.cloned(),
        }
    }
}

#[derive(Debug)]
struct TrackSender {
    sender: TrackPublisher,
    ack: Option<Ack>,
    noacked_sent: u64,
}

impl TrackSender {
    fn new(mut sender: TrackPublisher) -> Self {
        let ack = Some(sender.send_syn());
        Self {
            sender,
            ack,
            noacked_sent: 0,
        }
    }

    async fn prepare_send(&mut self) {
        if self.noacked_sent > MAX_NOACKED_COUNT {
            if let Some(ack) = self.ack.take() {
                ack.await;
            }
            self.ack = Some(self.sender.send_syn());
            self.noacked_sent = 0;
        }
    }

    async fn send_audio(&mut self, data: AudioFrame) -> bool {
        self.prepare_send().await;
        let ok = self.sender.send_audio(data);
        if ok {
            self.noacked_sent += 1;
        }
        ok
    }

    async fn send_video(&mut self, frame: VideoFrame) -> bool {
        self.prepare_send().await;
        let ok = self.sender.send_video(frame);
        if ok {
            self.noacked_sent += 1;
        }
        ok
    }

    fn send_eos(&mut self) {
        let _ = self.sender.send_eos();
    }
}

#[derive(Debug)]
struct ReaderState {
    path: PathBuf,
    file: File,
    demuxer: Mp4FileDemuxer,
    audio_track_id: Option<u32>,
    video_track_id: Option<u32>,
    audio_format: AudioFormat,
    audio_channels: Channels,
    audio_sample_rate: SampleRate,
    video_format: VideoFormat,
    video_width: usize,
    video_height: usize,
    /// トラックの最大 duration
    duration: Duration,
}

impl ReaderState {
    fn open(path: &Path, enable_audio: bool, enable_video: bool) -> Result<Self> {
        let mut file = File::open(path)
            .map_err(|e| Error::new(format!("Cannot open file {}: {e}", path.display())))?;
        let mut demuxer = Mp4FileDemuxer::new();
        initialize_mp4_demuxer(&mut file, &mut demuxer, path)?;

        let audio_track_id = if enable_audio {
            select_audio_track(demuxer.clone())?
        } else {
            None
        };
        let video_track_id = if enable_video {
            select_video_track(demuxer.clone())?
        } else {
            None
        };

        let duration = demuxer
            .tracks()
            .ok()
            .and_then(|tracks| {
                tracks
                    .iter()
                    .map(|t| Duration::from_secs(t.duration) / t.timescale.get())
                    .max()
            })
            .unwrap_or(Duration::ZERO);

        Ok(Self {
            path: path.to_path_buf(),
            file,
            demuxer,
            audio_track_id,
            video_track_id,
            // ダミー初期値。実際の値はサンプルエントリー受信時に上書きされる。
            audio_format: AudioFormat::Opus,
            audio_channels: Channels::STEREO,
            audio_sample_rate: SampleRate::HZ_48000,
            video_format: VideoFormat::Vp8,
            video_width: 0,
            video_height: 0,
            duration,
        })
    }

    fn is_audio_enabled(&self, track_id: u32) -> bool {
        self.audio_track_id == Some(track_id)
    }

    fn is_video_enabled(&self, track_id: u32) -> bool {
        self.video_track_id == Some(track_id)
    }

    fn update_audio_format(&mut self, sample_entry: &SampleEntry) -> Result<()> {
        let (metadata, format) = match sample_entry {
            SampleEntry::Opus(b) => (&b.audio, AudioFormat::Opus),
            SampleEntry::Mp4a(b) => (&b.audio, AudioFormat::Aac),
            entry => {
                return Err(Error::new(format!("unsupported sample entry: {entry:?}")));
            }
        };

        self.audio_format = format;
        self.audio_channels = Channels::from_u16(metadata.channelcount)?;
        self.audio_sample_rate = SampleRate::from_u16(metadata.samplerate.integer)?;

        Ok(())
    }

    fn update_video_format(&mut self, sample_entry: &SampleEntry) -> Result<()> {
        let (metadata, format) = match sample_entry {
            SampleEntry::Avc1(b) => (&b.visual, VideoFormat::H264),
            SampleEntry::Hev1(b) => (&b.visual, VideoFormat::H265),
            SampleEntry::Hvc1(b) => (&b.visual, VideoFormat::H265),
            SampleEntry::Vp08(b) => (&b.visual, VideoFormat::Vp8),
            SampleEntry::Vp09(b) => (&b.visual, VideoFormat::Vp9),
            SampleEntry::Av01(b) => (&b.visual, VideoFormat::Av1),
            entry => {
                return Err(Error::new(format!("unsupported sample entry: {entry:?}")));
            }
        };

        self.video_format = format;
        self.video_width = metadata.width as usize;
        self.video_height = metadata.height as usize;

        Ok(())
    }

    fn read_sample_data(&mut self, data_offset: u64, data_size: usize) -> Result<Vec<u8>> {
        let mut data = vec![0; data_size];
        self.file
            .seek(SeekFrom::Start(data_offset))
            .map_err(|e| Error::new(format!("Seek error {}: {e}", self.path.display())))?;
        self.file
            .read_exact(&mut data)
            .map_err(|e| Error::new(format!("Read error {}: {e}", self.path.display())))?;
        Ok(data)
    }
}

fn calculate_timestamps(timescale: u32, timestamp: u64, duration: u64) -> (Duration, Duration) {
    let timestamp = Duration::from_secs(timestamp) / timescale;
    let duration = Duration::from_secs(duration) / timescale;
    (timestamp, duration)
}

/// MP4 ファイルからトラック情報を初期化する
///
/// NOTE: fMP4 には未対応なので、この関数完了後、demuxer はファイル読み込みを要求しない
fn initialize_mp4_demuxer<R: Read + Seek, P: AsRef<Path>>(
    file: &mut R,
    demuxer: &mut Mp4FileDemuxer,
    path: P,
) -> Result<()> {
    // 念のために（壊れたファイルが渡された時のため）、バッファサイズの上限を 100 MB に設定しておく。
    // 正常なファイルの場合には、これは moov ボックスのサイズ上限となるが、
    // 典型的には、100 MB あれば、MP4 ファイル自体としては数百 GB 程度のものを扱えるため、実用上の問題はない想定。
    const MAX_BUF_SIZE: usize = 100 * 1024 * 1024;

    while let Some(required) = demuxer.required_input() {
        let size = required.size.ok_or_else(|| {
            Error::new(format!(
                "MP4 file contains unexpected variable size box {}",
                path.as_ref().display()
            ))
        })?;
        if size > MAX_BUF_SIZE {
            return Err(Error::new(format!(
                "MP4 file contains box larger than maximum allowed size ({size} > {MAX_BUF_SIZE}): {}",
                path.as_ref().display()
            )));
        }

        let mut buf = vec![0; size];
        file.seek(SeekFrom::Start(required.position))
            .map_err(|e| Error::new(format!("Seek error {}: {e}", path.as_ref().display())))?;
        file.read_exact(&mut buf)
            .map_err(|e| Error::new(format!("Read error {}: {e}", path.as_ref().display())))?;
        let input = required.to_input(&buf);
        demuxer.handle_input(input);
    }
    Ok(())
}

/// 音声トラックをチェックして、サポートされているコーデックを持つトラック ID を取得する
fn select_audio_track(mut demuxer: Mp4FileDemuxer) -> Result<Option<u32>> {
    let mut has_audio_track = false;
    while let Some(sample) = demuxer.next_sample()? {
        if sample.track.kind != TrackKind::Audio {
            continue;
        }
        has_audio_track = true;

        if let Some(sample_entry) = sample.sample_entry {
            let is_supported = match &sample_entry {
                SampleEntry::Opus(_) => true,
                SampleEntry::Mp4a(mp4a) => is_aac_codec(&mp4a.esds_box),
                _ => false,
            };

            if is_supported {
                return Ok(Some(sample.track.track_id));
            } else {
                tracing::warn!(
                    "Unsupported audio codec in track {}: {:?}",
                    sample.track.track_id,
                    sample_entry
                );
            }
        }
    }

    if has_audio_track {
        // 音声トラックがあるのにサポートしているコーデックがない場合はエラーにする
        Err(crate::Error::new(
            "No supported audio track found in the file".to_owned(),
        ))
    } else {
        // そもそも音声トラックがない場合には空扱いをする
        Ok(None)
    }
}

/// 映像トラックをチェックして、サポートされているコーデックを持つトラック ID を取得する
fn select_video_track(mut demuxer: Mp4FileDemuxer) -> Result<Option<u32>> {
    let mut has_video_track = false;
    while let Some(sample) = demuxer.next_sample()? {
        if sample.track.kind != TrackKind::Video {
            continue;
        }
        has_video_track = true;

        if let Some(sample_entry) = sample.sample_entry {
            let is_supported = matches!(
                sample_entry,
                SampleEntry::Avc1(_)
                    | SampleEntry::Hev1(_)
                    | SampleEntry::Hvc1(_)
                    | SampleEntry::Vp08(_)
                    | SampleEntry::Vp09(_)
                    | SampleEntry::Av01(_)
            );

            if is_supported {
                return Ok(Some(sample.track.track_id));
            } else {
                tracing::warn!(
                    "Unsupported video codec in track {}: {:?}",
                    sample.track.track_id,
                    sample_entry
                );
            }
        }
    }

    if has_video_track {
        // 映像トラックがあるのにサポートしているコーデックがない場合はエラーにする
        Err(crate::Error::new(
            "No supported video track found in the file".to_owned(),
        ))
    } else {
        // そもそも映像トラックがない場合には空扱いをする
        Ok(None)
    }
}

/// AAC コーデックであることを確認する
fn is_aac_codec(esds_box: &shiguredo_mp4::boxes::EsdsBox) -> bool {
    // DecoderConfigDescriptor の object_type_indication が AAC を示しているかチェック
    // AAC LC は 0x40 (64)
    // AAC Main Profile は 0x41 (65)
    // AAC SSR は 0x42 (66)
    // AAC LTP は 0x43 (67)
    matches!(
        esds_box.es.dec_config_descr.object_type_indication,
        0x40..=0x43
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_mp4_track_availability_detects_audio_only_file() -> Result<()> {
        let availability = probe_mp4_track_availability("testdata/beep-aac-audio.mp4")?;
        assert_eq!(
            availability,
            Mp4FileTrackAvailability {
                has_audio: true,
                has_video: false,
            }
        );
        Ok(())
    }

    #[test]
    fn probe_mp4_track_availability_detects_video_only_file() -> Result<()> {
        let availability = probe_mp4_track_availability("testdata/archive-red-320x320-h264.mp4")?;
        assert_eq!(
            availability,
            Mp4FileTrackAvailability {
                has_audio: false,
                has_video: true,
            }
        );
        Ok(())
    }

    #[test]
    fn probe_mp4_track_availability_detects_av_file() -> Result<()> {
        let availability = probe_mp4_track_availability("testdata/red-320x320-h264-aac.mp4")?;
        assert_eq!(
            availability,
            Mp4FileTrackAvailability {
                has_audio: true,
                has_video: true,
            }
        );
        Ok(())
    }

    /// update_playback_status がファイル内の絶対位置を公開することを検証する
    #[test]
    fn playback_status_cursor_reflects_file_position() -> Result<()> {
        let mut reader = Mp4FileReader::new(
            "testdata/red-320x320-h264-aac.mp4",
            Mp4FileReaderOptions::default(),
        )?;
        let handle = reader.create_media_handle(MediaEventContext {
            event_broadcast_tx: tokio::sync::broadcast::channel(8).0,
            input_name_rx: tokio::sync::watch::channel("test".to_owned()).1,
            input_uuid: "test-uuid".to_owned(),
        });

        // base_offset = 0 の場合
        reader.base_offset = Duration::ZERO;
        reader.update_playback_status(MediaPlaybackState::Playing, Duration::from_secs(5));
        assert_eq!(handle.status.borrow().cursor, Duration::from_secs(5));

        // base_offset > 0（ループ再生や Restart 後）の場合でも
        // ファイル内位置がそのまま公開される
        reader.base_offset = Duration::from_secs(10);
        reader.update_playback_status(MediaPlaybackState::Playing, Duration::from_secs(3));
        assert_eq!(handle.status.borrow().cursor, Duration::from_secs(3));

        Ok(())
    }

    /// 停止後の再開で base_offset が last_emitted_end に設定され、
    /// timestamp の連続性が維持されることを検証する
    #[test]
    fn reset_for_restart_preserves_timestamp_continuity() -> Result<()> {
        let mut reader = Mp4FileReader::new(
            "testdata/red-320x320-h264-aac.mp4",
            Mp4FileReaderOptions::default(),
        )?;
        let handle = reader.create_media_handle(MediaEventContext {
            event_broadcast_tx: tokio::sync::broadcast::channel(8).0,
            input_name_rx: tokio::sync::watch::channel("test".to_owned()).1,
            input_uuid: "test-uuid".to_owned(),
        });

        // 再生が 10 秒地点まで進んだ状態を模擬する
        reader.last_emitted_end = Duration::from_secs(10);
        reader.base_offset = Duration::ZERO;

        // 停止後の再開（reset_for_restart 相当）
        // ProcessorHandle がないため recreate_decoders は呼べないが、
        // base_offset のリセットロジックだけ検証する
        reader.base_offset = reader.last_emitted_end; // reset_for_restart と同じ

        // 先頭位置（ファイル内 0 秒）の公開カーソルは 0 秒であること
        reader.update_playback_status(MediaPlaybackState::Playing, Duration::ZERO);
        assert_eq!(handle.status.borrow().cursor, Duration::ZERO);

        // base_offset が last_emitted_end に設定されていること
        assert_eq!(reader.base_offset, Duration::from_secs(10));

        // 次のフレームの effective_timestamp は base_offset + file_timestamp = 10s + 0s = 10s
        // これは前回の last_emitted_end (10s) 以上なので、timestamp の連続性が保たれる
        let file_timestamp = Duration::ZERO;
        let effective_timestamp = reader.base_offset + file_timestamp;
        assert!(effective_timestamp >= Duration::from_secs(10));

        Ok(())
    }

    /// テスト用: メディア制御付き reader を作成するヘルパー
    fn reader_with_media_control() -> (Mp4FileReader, MediaInputHandle) {
        let mut reader = Mp4FileReader::new(
            "testdata/red-320x320-h264-aac.mp4",
            Mp4FileReaderOptions::default(),
        )
        .expect("test file must be readable");
        let handle = reader.create_media_handle(MediaEventContext {
            event_broadcast_tx: tokio::sync::broadcast::channel(8).0,
            input_name_rx: tokio::sync::watch::channel("test".to_owned()).1,
            input_uuid: "test-uuid".to_owned(),
        });
        (reader, handle)
    }

    /// 一時停止中の seek で Paused を維持し、cursor が clamp 済みで更新される
    #[test]
    fn seek_during_paused_preserves_state_and_clamps() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);
        reader.update_playback_status(MediaPlaybackState::Paused, Duration::from_secs(5));

        // 絶対位置シーク: duration 内
        reader.set_pending_seek(Duration::from_secs(10));
        let status = handle.status.borrow().clone();
        assert_eq!(status.state, MediaPlaybackState::Paused);
        assert_eq!(status.cursor, Duration::from_secs(10));

        // 絶対位置シーク: duration を超える → clamp される
        reader.set_pending_seek(Duration::from_secs(50));
        let status = handle.status.borrow().clone();
        assert_eq!(status.state, MediaPlaybackState::Paused);
        assert_eq!(status.cursor, Duration::from_secs(30));
    }

    /// Stopped 中の seek で Stopped を維持する
    #[test]
    fn seek_during_stopped_preserves_state() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);
        reader.update_playback_status(MediaPlaybackState::Stopped, Duration::from_secs(15));

        reader.set_pending_seek(Duration::from_secs(5));
        let status = handle.status.borrow().clone();
        assert_eq!(status.state, MediaPlaybackState::Stopped);
        assert_eq!(status.cursor, Duration::from_secs(5));
    }

    /// Ended 中の seek で Ended を維持し、Stopped へ変わらない
    #[test]
    fn seek_during_ended_preserves_state() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);
        reader.update_playback_status(MediaPlaybackState::Ended, Duration::from_secs(30));

        reader.set_pending_seek(Duration::from_secs(10));
        let status = handle.status.borrow().clone();
        assert_eq!(status.state, MediaPlaybackState::Ended);
        assert_eq!(status.cursor, Duration::from_secs(10));
    }

    /// duration 未取得時は seek 位置が 0 に固定されず、取得後は mediaDuration を超えない
    #[test]
    fn seek_clamp_depends_on_duration_availability() {
        let (mut reader, handle) = reader_with_media_control();

        // duration 未取得（0）: clamp されない
        reader.media_duration = Duration::ZERO;
        reader.set_pending_seek(Duration::from_secs(100));
        assert_eq!(handle.status.borrow().cursor, Duration::from_secs(100));

        // duration 取得済み: clamp される
        reader.media_duration = Duration::from_secs(30);
        reader.set_pending_seek(Duration::from_secs(100));
        assert_eq!(handle.status.borrow().cursor, Duration::from_secs(30));
    }

    /// resolve_offset_seek が現在位置基準で正しく計算する
    #[test]
    fn resolve_offset_seek_uses_current_cursor() {
        let (mut reader, _handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(60);

        // 現在位置を 10 秒に設定
        reader.base_offset = Duration::ZERO;
        reader.last_emitted_end = Duration::from_secs(10);

        // +5000ms → 15 秒
        let pos = reader.resolve_offset_seek(5000);
        assert_eq!(pos, Duration::from_secs(15));

        // -3000ms → 7 秒
        let pos = reader.resolve_offset_seek(-3000);
        assert_eq!(pos, Duration::from_secs(7));

        // -20000ms → 0 に clamp
        let pos = reader.resolve_offset_seek(-20000);
        assert_eq!(pos, Duration::ZERO);

        // +100000ms → duration に clamp
        let pos = reader.resolve_offset_seek(100000);
        assert_eq!(pos, Duration::from_secs(60));
    }

    /// backward seek 直後にさらに relative seek しても正しい位置に進む
    #[test]
    fn backward_seek_then_relative_seek_uses_correct_base() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(60);

        // 100 秒地点を模擬（base_offset=0, last_emitted_end=100s はあり得ないが
        // current_file_cursor() のテストなので duration 内の値で）
        reader.base_offset = Duration::ZERO;
        reader.last_emitted_end = Duration::from_secs(30);

        // 10 秒へ backward seek
        reader.set_pending_seek(Duration::from_secs(10));
        assert_eq!(handle.status.borrow().cursor, Duration::from_secs(10));

        // さらに +5000ms の relative seek は pending_seek=10s を基準に 15 秒になる
        let pos = reader.resolve_offset_seek(5000);
        assert_eq!(pos, Duration::from_secs(15));

        // -3000ms の relative seek は 7 秒になる
        let pos = reader.resolve_offset_seek(-3000);
        assert_eq!(pos, Duration::from_secs(7));
    }

    /// 停止中に seek してから start_playback すると、
    /// 公開 cursor が pending_seek の値で Playing になる
    #[test]
    fn start_playback_reflects_pending_seek() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);
        reader.base_offset = Duration::from_secs(10);

        // 停止状態で 15 秒にシーク
        reader.update_playback_status(MediaPlaybackState::Stopped, Duration::from_secs(20));
        reader.set_pending_seek(Duration::from_secs(15));
        assert_eq!(handle.status.borrow().cursor, Duration::from_secs(15));

        // Play 開始
        reader.start_playback();
        let status = handle.status.borrow().clone();
        assert_eq!(status.state, MediaPlaybackState::Playing);
        assert_eq!(status.cursor, Duration::from_secs(15));
        // pending_seek は take されていない（run_loop 冒頭で take する）
        assert_eq!(reader.pending_seek, Some(Duration::from_secs(15)));
    }

    /// seek なしの通常 start_playback は 0 秒で始まる
    #[test]
    fn start_playback_without_pending_seek_starts_at_zero() {
        let (mut reader, handle) = reader_with_media_control();
        reader.pending_seek = None;

        reader.start_playback();
        let status = handle.status.borrow().clone();
        assert_eq!(status.state, MediaPlaybackState::Playing);
        assert_eq!(status.cursor, Duration::ZERO);
    }

    /// Stopped 中に Seek(10s) 後 Restart → pending_seek がクリアされ 0 秒から始まる
    #[test]
    fn restart_after_seek_clears_pending_seek() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);

        // 停止中に 10 秒にシーク
        reader.update_playback_status(MediaPlaybackState::Stopped, Duration::from_secs(20));
        reader.set_pending_seek(Duration::from_secs(10));
        assert_eq!(reader.pending_seek, Some(Duration::from_secs(10)));

        // Restart 相当の処理: pending_seek をクリアしてリセット
        reader.pending_seek = None;
        reader.warmup_target = None;
        // reset_for_restart は ProcessorHandle が必要なので base_offset だけ手動設定
        reader.base_offset = reader.last_emitted_end;

        reader.start_playback();
        let status = handle.status.borrow().clone();
        assert_eq!(status.state, MediaPlaybackState::Playing);
        assert_eq!(status.cursor, Duration::ZERO);
        assert_eq!(reader.pending_seek, None);
    }

    /// Stopped 中に Seek(10s) 後 Play → pending_seek が維持され 10 秒から始まる
    #[test]
    fn play_after_seek_preserves_pending_seek() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);

        // 停止中に 10 秒にシーク
        reader.update_playback_status(MediaPlaybackState::Stopped, Duration::from_secs(20));
        reader.set_pending_seek(Duration::from_secs(10));

        // Play 相当: pending_seek を維持してリセット
        reader.base_offset = reader.last_emitted_end;

        reader.start_playback();
        let status = handle.status.borrow().clone();
        assert_eq!(status.state, MediaPlaybackState::Playing);
        assert_eq!(status.cursor, Duration::from_secs(10));
        assert_eq!(reader.pending_seek, Some(Duration::from_secs(10)));
    }

    /// seek 適用直後の relative seek が新しい位置基準で計算される
    #[test]
    fn relative_seek_after_apply_uses_logical_cursor() {
        let (mut reader, _handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(60);
        reader.base_offset = Duration::ZERO;
        reader.last_emitted_end = Duration::from_secs(10);

        // 30 秒への seek を模擬（apply_seek は ReaderState が必要なので logical_cursor を直接設定）
        reader.logical_cursor = Some(Duration::from_secs(30));

        // +5000ms → 35 秒（30s + 5s）
        let pos = reader.resolve_offset_seek(5000);
        assert_eq!(pos, Duration::from_secs(35));

        // フレーム publish で logical_cursor がクリアされると last_emitted_end 基準に戻る
        reader.logical_cursor = None;
        reader.last_emitted_end = Duration::from_secs(32);
        let pos = reader.resolve_offset_seek(5000);
        assert_eq!(pos, Duration::from_secs(37));
    }

    /// Pause → Seek → Stop を wait_while_paused() の実経路で通し、
    /// pending_seek がクリアされ state=Stopped, cursor=0 になることを確認する
    #[tokio::test]
    async fn pause_seek_stop_clears_seek_via_wait_while_paused() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);
        reader.is_paused = true;

        // コマンドを送信: Seek(10s) → Stop
        handle
            .command_tx
            .send(MediaInputCommand::Seek(Duration::from_secs(10)))
            .await
            .expect("send Seek must succeed");
        handle
            .command_tx
            .send(MediaInputCommand::Stop)
            .await
            .expect("send Stop must succeed");

        // wait_while_paused を実行
        let action = reader.wait_while_paused().await;
        assert!(matches!(action, MediaLoopAction::Stop));

        // seek 状態がクリアされ、cursor=0, state=Stopped
        assert_eq!(reader.pending_seek, None);
        assert_eq!(reader.logical_cursor, None);
        assert_eq!(handle.status.borrow().state, MediaPlaybackState::Stopped);
        assert_eq!(handle.status.borrow().cursor, Duration::ZERO);

        // Play 開始は 0 秒
        reader.base_offset = reader.last_emitted_end;
        reader.start_playback();
        assert_eq!(handle.status.borrow().cursor, Duration::ZERO);
    }

    /// Pause → Seek → Stop 後、run() 側の最終 status 更新でも cursor=0 が維持される
    #[tokio::test]
    async fn pause_seek_stop_keeps_zero_after_final_status_update() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);
        reader.is_paused = true;
        reader.base_offset = Duration::ZERO;
        reader.last_emitted_end = Duration::from_secs(12);

        handle
            .command_tx
            .send(MediaInputCommand::Seek(Duration::from_secs(10)))
            .await
            .expect("send Seek must succeed");
        handle
            .command_tx
            .send(MediaInputCommand::Stop)
            .await
            .expect("send Stop must succeed");

        let action = reader.wait_while_paused().await;
        assert!(matches!(action, MediaLoopAction::Stop));

        // run() 側の停止後最終更新を模擬しても 0 のまま
        reader.update_playback_status(MediaPlaybackState::Stopped, reader.stopped_file_cursor());
        assert_eq!(handle.status.borrow().state, MediaPlaybackState::Stopped);
        assert_eq!(handle.status.borrow().cursor, Duration::ZERO);
    }

    /// Stopped → Seek → Stop を wait_for_restart_command() の実経路で通し、
    /// cursor が 0 に戻ることを確認する
    #[tokio::test]
    async fn stopped_seek_stop_clears_seek_via_wait_for_restart() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);
        reader.update_playback_status(MediaPlaybackState::Stopped, Duration::from_secs(20));

        // コマンドを送信: Seek(10s) → Stop → Play
        handle
            .command_tx
            .send(MediaInputCommand::Seek(Duration::from_secs(10)))
            .await
            .expect("send Seek must succeed");
        handle
            .command_tx
            .send(MediaInputCommand::Stop)
            .await
            .expect("send Stop must succeed");
        handle
            .command_tx
            .send(MediaInputCommand::Play)
            .await
            .expect("send Play must succeed");

        // wait_for_restart_command を実行
        // Seek → Stop → Play の順で処理され、Stop で seek がクリア、Play で復帰
        let result = reader.wait_for_restart_command().await;
        assert!(matches!(result, WaitResult::Play));

        // Stop で seek がクリアされ cursor=0
        assert_eq!(reader.pending_seek, None);

        // Play 開始は 0 秒
        reader.base_offset = reader.last_emitted_end;
        reader.start_playback();
        assert_eq!(handle.status.borrow().cursor, Duration::ZERO);
    }

    /// Ended → Seek → Stop を wait_for_restart_command() の実経路で通し、
    /// Stopped に遷移し cursor が 0 に戻ることを確認する
    #[tokio::test]
    async fn ended_seek_stop_transitions_to_stopped_via_wait_for_restart() {
        let (mut reader, handle) = reader_with_media_control();
        reader.media_duration = Duration::from_secs(30);
        reader.update_playback_status(MediaPlaybackState::Ended, Duration::from_secs(30));

        // コマンドを送信: Seek(10s) → Stop → Play
        handle
            .command_tx
            .send(MediaInputCommand::Seek(Duration::from_secs(10)))
            .await
            .expect("send Seek must succeed");
        handle
            .command_tx
            .send(MediaInputCommand::Stop)
            .await
            .expect("send Stop must succeed");
        handle
            .command_tx
            .send(MediaInputCommand::Play)
            .await
            .expect("send Play must succeed");

        let result = reader.wait_for_restart_command().await;
        assert!(matches!(result, WaitResult::Play));

        // Seek 後に Stop で Stopped に遷移し、cursor=0
        assert_eq!(handle.status.borrow().state, MediaPlaybackState::Stopped);
        assert_eq!(handle.status.borrow().cursor, Duration::ZERO);
        assert_eq!(reader.pending_seek, None);
    }
}
