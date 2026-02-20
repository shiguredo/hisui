type LocalProcessorTask = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug)]
pub struct MediaPipeline {
    command_tx: Option<tokio::sync::mpsc::UnboundedSender<MediaPipelineCommand>>,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<MediaPipelineCommand>,
    local_processor_task_tx: Option<tokio::sync::mpsc::UnboundedSender<LocalProcessorTask>>,
    local_processor_thread: std::thread::JoinHandle<()>,
    registration_closed: bool,
    initial_ready_open: bool,
    processors: std::collections::HashMap<ProcessorId, ProcessorState>,
    pending_initial_processors: std::collections::HashSet<ProcessorId>,
    initial_ready_waiters: Vec<tokio::sync::oneshot::Sender<()>>,
    tracks: std::collections::HashMap<TrackId, TrackState>,
    stats: crate::stats::Stats,
}

impl MediaPipeline {
    pub fn new() -> crate::Result<Self> {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let (local_processor_task_tx, local_processor_task_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let local_processor_thread = std::thread::Builder::new()
            .name("media_pipeline_local".to_owned())
            .spawn(move || run_local_processor_runtime_thread(local_processor_task_rx))
            .map_err(|e| {
                crate::Error::new(format!(
                    "failed to spawn media pipeline local runtime thread: {e}"
                ))
            })?;
        Ok(Self {
            command_tx: Some(command_tx),
            command_rx,
            local_processor_task_tx: Some(local_processor_task_tx),
            local_processor_thread,
            registration_closed: false,
            initial_ready_open: false,
            processors: std::collections::HashMap::new(),
            pending_initial_processors: std::collections::HashSet::new(),
            initial_ready_waiters: Vec::new(),
            tracks: std::collections::HashMap::new(),
            stats: crate::stats::Stats::new(),
        })
    }

    /// このパイプラインを操作するためのハンドルを返す
    ///
    /// 全てのハンドルが（間接的なものも含めて）ドロップしたら、パイプラインも終了する
    pub fn handle(&self) -> MediaPipelineHandle {
        MediaPipelineHandle {
            command_tx: self.command_tx.clone().expect("infallible"),
            local_processor_task_tx: self.local_processor_task_tx.clone().expect("infallible"),
            stats: self.stats.clone(),
        }
    }

    pub async fn run(mut self) {
        tracing::debug!("MediaPipeline started");

        self.command_tx = None; // 参照カウントから自分を外すために None にする
        self.local_processor_task_tx = None; // 参照カウントから自分を外すために None にする

        loop {
            tokio::select! {
                Some(command) = self.command_rx.recv() => self.handle_command(command),
                else => break,
            }
        }

        if self.local_processor_thread.join().is_err() {
            tracing::error!("media pipeline local runtime thread panicked");
        }

        tracing::debug!("MediaPipeline stopped");
    }

    fn handle_command(&mut self, command: MediaPipelineCommand) {
        match command {
            MediaPipelineCommand::RegisterProcessor {
                processor_id,
                reply_tx,
            } => {
                let result = self.handle_register_processor(processor_id);
                let _ = reply_tx.send(result); // 応答では受信側がすでに閉じていても問題ないので、結果の確認は不要（以降も同様）
            }
            MediaPipelineCommand::DeregisterProcessor { processor_id } => {
                self.handle_deregister_processor(processor_id);
            }
            MediaPipelineCommand::PublishTrack {
                processor_id,
                track_id,
                reply_tx,
            } => {
                let result = self.handle_publish_track(processor_id, track_id);
                let _ = reply_tx.send(result);
            }
            MediaPipelineCommand::SubscribeTrack {
                processor_id,
                track_id,
                tx,
            } => {
                self.handle_subscribe_track(processor_id, track_id, tx);
            }
            MediaPipelineCommand::ListTracks { reply_tx } => {
                let _ = reply_tx.send(self.handle_list_tracks());
            }
            MediaPipelineCommand::ListProcessors { reply_tx } => {
                let _ = reply_tx.send(self.handle_list_processors());
            }
            MediaPipelineCommand::CompleteInitialProcessorRegistration => {
                self.handle_complete_initial_processor_registration();
            }
            MediaPipelineCommand::NotifyReady { processor_id } => {
                self.handle_notify_ready(processor_id);
            }
            MediaPipelineCommand::WaitSubscribersReady {
                processor_id,
                reply_tx,
            } => {
                self.handle_wait_subscribers_ready(processor_id, reply_tx);
            }
        }
    }

    fn handle_complete_initial_processor_registration(&mut self) {
        if self.registration_closed {
            return;
        }
        self.registration_closed = true;

        for (processor_id, state) in &mut self.processors {
            state.is_initial_member = true;
            if !state.notified_ready {
                self.pending_initial_processors.insert(processor_id.clone());
            }
        }
        self.try_open_initial_ready();
    }

    fn handle_notify_ready(&mut self, processor_id: ProcessorId) {
        let Some(state) = self.processors.get_mut(&processor_id) else {
            tracing::warn!("attempt to notify ready from unregistered processor: {processor_id}");
            return;
        };
        state.notified_ready = true;
        if state.is_initial_member {
            self.pending_initial_processors.remove(&processor_id);
        }
        self.try_open_initial_ready();
    }

    fn handle_wait_subscribers_ready(
        &mut self,
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        if !self.processors.contains_key(&processor_id) {
            tracing::warn!("attempt to wait from unregistered processor: {processor_id}");
            let _ = reply_tx.send(());
            return;
        }

        if self.initial_ready_open {
            let _ = reply_tx.send(());
            return;
        }

        self.initial_ready_waiters.push(reply_tx);
        self.try_open_initial_ready();
    }

    fn try_open_initial_ready(&mut self) {
        if self.initial_ready_open
            || !self.registration_closed
            || !self.pending_initial_processors.is_empty()
        {
            return;
        }
        self.initial_ready_open = true;
        self.flush_pending_subscribers();
        for waiter in self.initial_ready_waiters.drain(..) {
            let _ = waiter.send(());
        }
    }

    fn flush_pending_subscribers(&mut self) {
        for track in self.tracks.values_mut() {
            let Some(publisher_tx) = track.publisher_command_tx.as_ref() else {
                continue;
            };
            for subscriber_tx in track.pending_subscribers.drain(..) {
                let _ = publisher_tx.send(TrackCommand::AddSubscriber(subscriber_tx));
            }
        }
    }

    fn handle_subscribe_track(
        &mut self,
        processor_id: ProcessorId,
        track_id: TrackId,
        tx: tokio::sync::mpsc::UnboundedSender<Message>,
    ) {
        tracing::debug!("subscribe track: processor={processor_id}, track={track_id}");

        if !self.processors.contains_key(&processor_id) {
            tracing::warn!(
                "attempt to subscribe to track from unregistered processor: {processor_id}"
            );
            return;
        }

        // トラックが存在しない場合は新規作成
        let track = self.tracks.entry(track_id.clone()).or_insert_with(|| {
            tracing::debug!("creating new track: {track_id}");
            TrackState::default()
        });

        if self.initial_ready_open {
            if let Some(publisher_tx) = &track.publisher_command_tx {
                let _ = publisher_tx.send(TrackCommand::AddSubscriber(tx));
            } else {
                // publisher がまだ登録されていない場合は、subscriber を待機キューに追加
                tracing::debug!("publisher not yet registered for track: {track_id}");
                track.pending_subscribers.push(tx);

                // TODO: publisher の再登録に対応する
            }
        } else {
            // 初期化が完了するまでは接続せず、subscriber を待機キューに追加
            tracing::debug!("initial barrier not open yet for track: {track_id}");
            track.pending_subscribers.push(tx);
        }
    }

    fn handle_publish_track(
        &mut self,
        processor_id: ProcessorId,
        track_id: TrackId,
    ) -> Result<MessageSender, PublishTrackError> {
        tracing::debug!("publish track: processor={processor_id}, track={track_id}");

        if !self.processors.contains_key(&processor_id) {
            tracing::warn!("attempt to publish track from unregistered processor: {processor_id}");
            return Err(PublishTrackError::UnregisteredProcessor);
        }

        // トラックが存在しない場合は新規作成
        let track = self.tracks.entry(track_id.clone()).or_insert_with(|| {
            tracing::debug!("creating new track: {track_id}");
            TrackState::default()
        });

        if track.publisher_command_tx.is_some() {
            tracing::warn!(
                "publisher conflict for track: processor={processor_id}, track={track_id}"
            );
            return Err(PublishTrackError::DuplicateTrackId);
        }

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        track.publisher_command_tx = Some(command_tx.clone());

        // 初期バリア開放後のみ、待機中の subscriber に通知
        if self.initial_ready_open {
            for subscriber_tx in track.pending_subscribers.drain(..) {
                let _ = command_tx.send(TrackCommand::AddSubscriber(subscriber_tx));
            }
        }

        Ok(MessageSender {
            rx: command_rx,
            txs: Vec::new(),
        })
    }

    fn handle_register_processor(&mut self, processor_id: ProcessorId) -> bool {
        tracing::debug!("register processor: {processor_id}");

        if self.processors.contains_key(&processor_id) {
            tracing::warn!("processor already registered: {processor_id}");
            return false;
        }

        self.processors
            .insert(processor_id.clone(), ProcessorState::default());
        true
    }

    fn handle_deregister_processor(&mut self, processor_id: ProcessorId) {
        // TODO: トラックが残っている間は deregister 扱いにしない
        tracing::debug!("deregister processor: {processor_id}");
        if let Some(state) = self.processors.remove(&processor_id)
            && state.is_initial_member
            && !state.notified_ready
        {
            self.pending_initial_processors.remove(&processor_id);
        }
        self.try_open_initial_ready();
    }

    fn handle_list_tracks(&self) -> Vec<TrackId> {
        self.tracks.keys().cloned().collect()
    }

    fn handle_list_processors(&self) -> Vec<ProcessorId> {
        self.processors.keys().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct MediaPipelineHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<MediaPipelineCommand>,
    local_processor_task_tx: tokio::sync::mpsc::UnboundedSender<LocalProcessorTask>,
    stats: crate::stats::Stats,
}

impl MediaPipelineHandle {
    pub async fn spawn_processor<F, T>(
        &self,
        processor_id: ProcessorId,
        metadata: ProcessorMetadata,
        f: F,
    ) -> Result<(), RegisterProcessorError>
    where
        F: FnOnce(ProcessorHandle) -> T + Send + 'static,
        T: Future<Output = crate::Result<()>> + Send,
    {
        let handle = self
            .register_processor(processor_id.clone(), metadata)
            .await?;
        let error_flag = handle.error_flag.clone();
        tokio::spawn(async move {
            if let Err(e) = f(handle).await {
                error_flag.set(true);
                tracing::error!("failed to run processor {processor_id}: {}", e.display());
            }
        });
        Ok(())
    }

    pub async fn spawn_local_processor<F, T>(
        &self,
        processor_id: ProcessorId,
        metadata: ProcessorMetadata,
        f: F,
    ) -> Result<(), RegisterProcessorError>
    where
        F: FnOnce(ProcessorHandle) -> T + Send + 'static,
        T: Future<Output = crate::Result<()>> + 'static,
    {
        let handle = self
            .register_processor(processor_id.clone(), metadata)
            .await?;
        let error_flag = handle.error_flag.clone();
        let task: LocalProcessorTask = Box::new(move || {
            tokio::task::spawn_local(async move {
                if let Err(e) = f(handle).await {
                    error_flag.set(true);
                    tracing::error!("failed to run processor {processor_id}: {}", e.display());
                }
            });
        });
        self.local_processor_task_tx
            .send(task)
            .map_err(|_| RegisterProcessorError::PipelineTerminated)
    }

    /// [NOTE] こちらは内部寄りなので、可能な限りは spawn_processor() を使うこと
    pub async fn register_processor(
        &self,
        processor_id: ProcessorId,
        metadata: ProcessorMetadata,
    ) -> Result<ProcessorHandle, RegisterProcessorError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = MediaPipelineCommand::RegisterProcessor {
            processor_id: processor_id.clone(),
            reply_tx,
        };

        // [NOTE] パイプライン終了は次の rx で判定できるのでここでは返り値の考慮は不要
        self.send(command);

        match reply_rx.await {
            Ok(true) => {
                let mut stats = self.stats();
                // [NOTE]
                // stats の label は取得時点で固定されるため、
                // ここで default label を先に確定してから最初のメトリクスを取得する。
                stats.set_default_label("processor_id", processor_id.get());
                stats.set_default_label("processor_type", metadata.processor_type());
                // `error` は初期化時に必ず 0 で作っておく（後続タスク側は true への遷移のみ担当）。
                let error_flag = stats.flag("error");
                error_flag.set(false);
                Ok(ProcessorHandle {
                    pipeline_handle: self.clone(),
                    processor_id,
                    stats,
                    error_flag,
                })
            }
            Ok(false) => Err(RegisterProcessorError::DuplicateProcessorId),
            Err(_) => Err(RegisterProcessorError::PipelineTerminated),
        }
    }

    /// 初期 processor の登録が完了したことを通知する
    pub fn complete_initial_processor_registration(&self) {
        self.send(MediaPipelineCommand::CompleteInitialProcessorRegistration);
    }

    pub fn stats(&self) -> crate::stats::Stats {
        self.stats.clone()
    }

    // すでに MediaPipeline が終了している場合には false が返される。
    // なお、通常はこの結果をハンドリングする必要はない。
    // （コマンドの応答を受け取る場合は、その受信側で検知できるし、
    //   応答を受け取らない場合にはそもそもここの成功・失敗に依存するようなコマンドであるべきではないため）
    pub(crate) fn send(&self, command: MediaPipelineCommand) -> bool {
        self.command_tx.send(command).is_ok()
    }
}

fn run_local_processor_runtime_thread(
    mut task_rx: tokio::sync::mpsc::UnboundedReceiver<LocalProcessorTask>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!("failed to create local runtime for media pipeline: {e}");
            return;
        }
    };
    let local = tokio::task::LocalSet::new();
    runtime.block_on(local.run_until(async move {
        while let Some(task) = task_rx.recv().await {
            task();
        }
    }));
}

#[derive(Debug)]
pub(crate) enum MediaPipelineCommand {
    RegisterProcessor {
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<bool>,
    },
    DeregisterProcessor {
        processor_id: ProcessorId,
    },
    PublishTrack {
        processor_id: ProcessorId,
        track_id: TrackId,
        reply_tx: tokio::sync::oneshot::Sender<Result<MessageSender, PublishTrackError>>,
    },
    SubscribeTrack {
        processor_id: ProcessorId,
        track_id: TrackId,
        tx: tokio::sync::mpsc::UnboundedSender<Message>,
    },
    ListTracks {
        reply_tx: tokio::sync::oneshot::Sender<Vec<TrackId>>,
    },
    ListProcessors {
        reply_tx: tokio::sync::oneshot::Sender<Vec<ProcessorId>>,
    },
    CompleteInitialProcessorRegistration,
    NotifyReady {
        processor_id: ProcessorId,
    },
    WaitSubscribersReady {
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<()>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProcessorId(String);

impl ProcessorId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn get(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProcessorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl nojson::DisplayJson for ProcessorId {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.get())
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for ProcessorId {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        value.try_into().map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorMetadata {
    processor_type: String,
}

impl ProcessorMetadata {
    pub fn new(processor_type: impl Into<String>) -> Self {
        Self {
            processor_type: processor_type.into(),
        }
    }

    pub fn processor_type(&self) -> &str {
        &self.processor_type
    }
}

impl Default for ProcessorMetadata {
    fn default() -> Self {
        Self::new("unknown")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackId(String);

impl TrackId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn get(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TrackId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl nojson::DisplayJson for TrackId {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.value(self.get())
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for TrackId {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        value.try_into().map(Self)
    }
}

#[derive(Debug, Default)]
struct TrackState {
    publisher_command_tx: Option<tokio::sync::mpsc::UnboundedSender<TrackCommand>>,
    pending_subscribers: Vec<tokio::sync::mpsc::UnboundedSender<Message>>,
}

#[derive(Debug, Clone, Copy, Default)]
struct ProcessorState {
    notified_ready: bool,
    is_initial_member: bool,
}

#[derive(Debug)]
pub struct ProcessorHandle {
    pipeline_handle: MediaPipelineHandle,
    processor_id: ProcessorId,
    stats: crate::stats::Stats,
    error_flag: crate::stats::StatsFlag,
}

impl ProcessorHandle {
    pub fn processor_id(&self) -> &ProcessorId {
        &self.processor_id
    }

    pub fn stats(&self) -> crate::stats::Stats {
        self.stats.clone()
    }

    pub async fn publish_track(
        &self,
        track_id: TrackId,
    ) -> Result<MessageSender, PublishTrackError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = MediaPipelineCommand::PublishTrack {
            processor_id: self.processor_id.clone(),
            track_id,
            reply_tx,
        };
        self.pipeline_handle.send(command);
        match reply_rx.await {
            Ok(result) => result,
            Err(_) => Err(PublishTrackError::PipelineTerminated),
        }
    }

    pub fn subscribe_track(&self, track_id: TrackId) -> MessageReceiver {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let command = MediaPipelineCommand::SubscribeTrack {
            processor_id: self.processor_id.clone(),
            track_id,
            tx,
        };
        self.pipeline_handle.send(command);

        // トラックが存在しなかったりした場合は、すぐに受信側が閉じるだけなので、
        // 上のコマンドの結果はまたない
        MessageReceiver { rx }
    }

    /// 自分自身の準備完了を通知する
    pub fn notify_ready(&self) {
        self.pipeline_handle
            .send(MediaPipelineCommand::NotifyReady {
                processor_id: self.processor_id.clone(),
            });
    }

    /// 初期 processor 群の準備が完了するまで待機する
    pub async fn wait_subscribers_ready(&self) -> Result<(), PipelineTerminated> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.pipeline_handle
            .send(MediaPipelineCommand::WaitSubscribersReady {
                processor_id: self.processor_id.clone(),
                reply_tx,
            });
        reply_rx.await.map_err(|_| PipelineTerminated)
    }

    // TODO: これは実際に必要になったタイミングで実装する
    // （publish / subscribe と同様に RPC 用のチャネルの作成を MediaPipeline に依頼するのが良さそう）
    //
    // pub async fn recv_rpc_request(&mut self) -> JsonRpcRequest {
    //    match self.rpc_rx.recv().await {
    //        Some(request) => request,
    //        None => std::future::pending().await,
    //    }
    // }
}

impl Drop for ProcessorHandle {
    fn drop(&mut self) {
        // 登録を解除する。
        //パイプラインが終了している場合には送信に失敗するが、そもそもその状況ではすでにエントリは削除されているので問題ないため、結果は無視する。
        self.pipeline_handle
            .send(MediaPipelineCommand::DeregisterProcessor {
                processor_id: self.processor_id.clone(),
            });
    }
}

#[derive(Debug, Clone)]
pub struct Syn(#[expect(dead_code)] tokio::sync::mpsc::Sender<()>);

#[derive(Debug)]
pub struct Ack(tokio::sync::mpsc::Receiver<()>);

impl std::future::Future for Ack {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll_recv(cx).map(|_| ())
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Media(crate::MediaSample),
    Eos,

    /// 送信側がメッセージグラフの末端まで到達したか確認するための制御メッセージ。
    /// mpsc チャネルの受信側でクローズを確認することで、メッセージが完全に処理されたこと（= Ack を受け取った）を検知できる。
    Syn(Syn),
}

#[derive(Debug)]
enum TrackCommand {
    AddSubscriber(tokio::sync::mpsc::UnboundedSender<Message>),
}

#[derive(Debug)]
pub struct MessageSender {
    rx: tokio::sync::mpsc::UnboundedReceiver<TrackCommand>,
    txs: Vec<tokio::sync::mpsc::UnboundedSender<Message>>,
}

impl MessageSender {
    // MediaPipeline が途中終了した場合は false が返される
    pub fn send(&mut self, message: Message) -> bool {
        if !self.drain_track_commands() {
            return false;
        }

        self.txs.retain_mut(|tx| tx.send(message.clone()).is_ok());
        true
    }

    pub fn send_media(&mut self, sample: crate::MediaSample) -> bool {
        self.send(Message::Media(sample))
    }

    pub fn send_audio(&mut self, data: crate::AudioData) -> bool {
        self.send(Message::Media(crate::MediaSample::new_audio(data)))
    }

    pub fn send_video(&mut self, frame: crate::VideoFrame) -> bool {
        self.send(Message::Media(crate::MediaSample::new_video(frame)))
    }

    pub fn send_eos(&mut self) -> bool {
        self.send(Message::Eos)
    }

    pub fn send_syn(&mut self) -> Ack {
        let (tx, rx) = tokio::sync::mpsc::channel(1); // NOTE: 0 だとエラーになる
        let _ = self.send(Message::Syn(Syn(tx))); // NOTE: ここでは false を特別扱いする必要はないので無視する
        Ack(rx)
    }

    fn drain_track_commands(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(TrackCommand::AddSubscriber(tx)) => {
                    self.txs.push(tx);
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    return true;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.txs.clear();
                    return false;
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct MessageReceiver {
    rx: tokio::sync::mpsc::UnboundedReceiver<Message>,
}

impl MessageReceiver {
    pub async fn recv(&mut self) -> Message {
        if let Some(m) = self.rx.recv().await {
            m
        } else {
            // MediaPipeline::run() が何らかの理由で途中で終了した場合にはここに来る（EOS 扱いにする）
            Message::Eos
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterProcessorError {
    /// パイプラインが終了している
    PipelineTerminated,
    /// プロセッサーIDが重複している
    DuplicateProcessorId,
}

impl std::fmt::Display for RegisterProcessorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PipelineTerminated => write!(f, "Pipeline has terminated"),
            Self::DuplicateProcessorId => write!(f, "Processor ID already registered"),
        }
    }
}

impl std::error::Error for RegisterProcessorError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineTerminated;

impl std::fmt::Display for PipelineTerminated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pipeline has terminated")
    }
}

impl std::error::Error for PipelineTerminated {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishTrackError {
    /// パイプラインが終了している
    PipelineTerminated,
    /// トラックIDが重複している
    DuplicateTrackId,
    /// プロセッサーが未登録
    UnregisteredProcessor,
}

impl std::fmt::Display for PublishTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PipelineTerminated => write!(f, "Pipeline has terminated"),
            Self::DuplicateTrackId => write!(f, "Track ID already published"),
            Self::UnregisteredProcessor => write!(f, "Processor is not registered"),
        }
    }
}

impl std::error::Error for PublishTrackError {}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::time::Duration;

    use super::*;

    fn metadata(name: &str) -> ProcessorMetadata {
        ProcessorMetadata::new(name)
    }

    #[tokio::test]
    async fn spawn_local_processor_accepts_non_send_future() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        handle.complete_initial_processor_registration();

        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<usize>();
        handle
            .spawn_local_processor(
                ProcessorId::new("local-non-send"),
                metadata("test_local_processor"),
                move |_handle| async move {
                    let value = Rc::new(41usize);
                    let _ = done_tx.send(*value + 1);
                    Ok(())
                },
            )
            .await
            .expect("spawn_local_processor must succeed");

        let value = tokio::time::timeout(Duration::from_secs(5), done_rx)
            .await
            .expect("done signal timed out")
            .expect("done signal channel closed");
        assert_eq!(value, 42);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn spawn_local_processor_rejects_duplicate_processor_id() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        handle.complete_initial_processor_registration();

        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        handle
            .spawn_local_processor(
                ProcessorId::new("duplicate-local"),
                metadata("test_local_processor"),
                move |handle| async move {
                    let _ = release_rx.await;
                    drop(handle);
                    Ok(())
                },
            )
            .await
            .expect("first spawn_local_processor must succeed");

        let result = handle
            .spawn_local_processor(
                ProcessorId::new("duplicate-local"),
                metadata("test_local_processor"),
                move |_handle| async move { Ok(()) },
            )
            .await;
        assert_eq!(result, Err(RegisterProcessorError::DuplicateProcessorId));

        let _ = release_tx.send(());

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn wait_subscribers_ready_waits_for_initial_processors() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        {
            let first = handle
                .register_processor(ProcessorId::new("wait_first"), metadata("test_processor"))
                .await
                .expect("failed to register first processor");
            let second = handle
                .register_processor(ProcessorId::new("wait_second"), metadata("test_processor"))
                .await
                .expect("failed to register second processor");

            first.notify_ready();
            let wait = first.wait_subscribers_ready();
            tokio::pin!(wait);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut wait)
                    .await
                    .is_err(),
                "wait_subscribers_ready must wait before registration close"
            );

            handle.complete_initial_processor_registration();
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut wait)
                    .await
                    .is_err(),
                "wait_subscribers_ready must wait until all initial processors notify ready"
            );

            second.notify_ready();
            tokio::time::timeout(Duration::from_secs(2), &mut wait)
                .await
                .expect("wait_subscribers_ready did not finish after all ready")
                .expect("wait_subscribers_ready returned error");
        }

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn send_after_initial_ready_delivers_to_subscriber() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let sender = handle
            .register_processor(ProcessorId::new("sender"), metadata("test_sender"))
            .await
            .expect("failed to register sender");
        let receiver = handle
            .register_processor(ProcessorId::new("receiver"), metadata("test_receiver"))
            .await
            .expect("failed to register receiver");
        let track_id = TrackId::new("ready-track");
        let mut tx = sender
            .publish_track(track_id.clone())
            .await
            .expect("failed to publish track");
        let mut rx = receiver.subscribe_track(track_id);

        sender.notify_ready();
        receiver.notify_ready();
        handle.complete_initial_processor_registration();
        sender
            .wait_subscribers_ready()
            .await
            .expect("wait_subscribers_ready must succeed");

        assert!(tx.send_eos(), "send_eos must succeed");

        let message = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("receiver did not get eos");
        assert!(matches!(message, Message::Eos));

        drop(rx);
        drop(receiver);
        drop(sender);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn send_syn_ack_waits_for_drop_after_initial_ready() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let sender = handle
            .register_processor(ProcessorId::new("syn_sender"), metadata("test_sender"))
            .await
            .expect("failed to register sender");
        let receiver = handle
            .register_processor(ProcessorId::new("syn_receiver"), metadata("test_receiver"))
            .await
            .expect("failed to register receiver");
        let track_id = TrackId::new("syn-track");
        let mut tx = sender
            .publish_track(track_id.clone())
            .await
            .expect("failed to publish track");
        let mut rx = receiver.subscribe_track(track_id);

        sender.notify_ready();
        receiver.notify_ready();
        handle.complete_initial_processor_registration();
        sender
            .wait_subscribers_ready()
            .await
            .expect("wait_subscribers_ready must succeed");

        let ack = tx.send_syn();
        tokio::pin!(ack);
        let message = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("receiver did not get syn");
        assert!(matches!(message, Message::Syn(_)));

        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut ack)
                .await
                .is_err(),
            "ack must stay pending while syn message is held"
        );

        drop(message);
        tokio::time::timeout(Duration::from_secs(2), &mut ack)
            .await
            .expect("ack did not complete after dropping syn message");

        drop(rx);
        drop(receiver);
        drop(sender);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn complete_initial_processor_registration_is_idempotent() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        let processor = handle
            .register_processor(
                ProcessorId::new("idempotent_processor"),
                metadata("test_processor"),
            )
            .await
            .expect("failed to register processor");

        processor.notify_ready();
        handle.complete_initial_processor_registration();
        handle.complete_initial_processor_registration();
        processor
            .wait_subscribers_ready()
            .await
            .expect("wait_subscribers_ready must succeed");

        drop(processor);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn wait_subscribers_ready_returns_error_after_pipeline_terminated() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        let processor = handle
            .register_processor(
                ProcessorId::new("terminated_waiter"),
                metadata("test_processor"),
            )
            .await
            .expect("failed to register processor");

        pipeline_task.abort();
        let _ = pipeline_task.await;

        let result = processor.wait_subscribers_ready().await;
        assert_eq!(result, Err(PipelineTerminated));
    }

    #[tokio::test]
    async fn processor_handle_stats_has_processor_id_label() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let processor = handle
            .register_processor(
                ProcessorId::new("stats_processor"),
                metadata("test_processor"),
            )
            .await
            .expect("failed to register processor");
        let mut stats = processor.stats();
        stats.counter("processed_frames_total").inc();

        let text = handle
            .stats()
            .to_prometheus_text()
            .expect("to_prometheus_text must succeed");
        assert!(text.contains("hisui_processed_frames_total"));
        assert!(text.contains("processor_id=\"stats_processor\""));

        drop(processor);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn spawn_processor_sets_error_flag_on_failure() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        handle.complete_initial_processor_registration();

        handle
            .spawn_processor(
                ProcessorId::new("failing_processor"),
                metadata("test_processor"),
                move |_handle| async move { Err(crate::Error::new("processor failed")) },
            )
            .await
            .expect("spawn_processor must succeed");

        wait_until_metric_contains(
            &handle,
            "hisui_error{processor_id=\"failing_processor\",processor_type=\"test_processor\"} 1",
        )
        .await;

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn spawn_local_processor_sets_error_flag_on_failure() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        handle.complete_initial_processor_registration();

        handle
            .spawn_local_processor(
                ProcessorId::new("failing_local_processor"),
                metadata("test_processor"),
                move |_handle| async move { Err(crate::Error::new("local processor failed")) },
            )
            .await
            .expect("spawn_local_processor must succeed");

        wait_until_metric_contains(
            &handle,
            "hisui_error{processor_id=\"failing_local_processor\",processor_type=\"test_processor\"} 1",
        )
        .await;

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    async fn wait_until_metric_contains(handle: &MediaPipelineHandle, needle: &str) {
        for _ in 0..200 {
            let text = handle
                .stats()
                .to_prometheus_text()
                .expect("to_prometheus_text must succeed");
            if text.contains(needle) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("metric not found within timeout: {needle}");
    }
}
