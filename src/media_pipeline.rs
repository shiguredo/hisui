type LocalProcessorTask = Box<dyn FnOnce() + Send + 'static>;
type PendingRpcSenderWaiter =
    tokio::sync::oneshot::Sender<Result<ErasedRpcSender, GetProcessorRpcSenderError>>;

pub const PROCESSOR_TYPE_VIDEO_ENCODER: &str = "video_encoder";

#[derive(Clone, Default)]
pub struct MediaPipelineConfig {
    pub openh264_lib: Option<shiguredo_openh264::Openh264Library>,
}

impl std::fmt::Debug for MediaPipelineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaPipelineConfig")
            .field("openh264_lib", &self.openh264_lib.is_some())
            .finish()
    }
}

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
    config: std::sync::Arc<MediaPipelineConfig>,
}

impl MediaPipeline {
    pub fn new() -> crate::Result<Self> {
        Self::new_with_config(MediaPipelineConfig::default())
    }

    pub fn new_with_config(config: MediaPipelineConfig) -> crate::Result<Self> {
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
            config: std::sync::Arc::new(config),
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
            config: self.config.clone(),
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
                metadata,
                reply_tx,
            } => {
                let result = self.handle_register_processor(processor_id, metadata);
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
            MediaPipelineCommand::TriggerStart { reply_tx } => {
                let _ = reply_tx.send(self.handle_complete_initial_processor_registration());
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
            MediaPipelineCommand::RegisterProcessorRpcSender {
                processor_id,
                sender,
                reply_tx,
            } => {
                let result = self.handle_register_processor_rpc_sender(processor_id, sender);
                let _ = reply_tx.send(result);
            }
            MediaPipelineCommand::GetProcessorRpcSender {
                processor_id,
                reply_tx,
            } => {
                self.handle_get_processor_rpc_sender(processor_id, reply_tx);
            }
            MediaPipelineCommand::SetProcessorAbortHandle {
                processor_id,
                abort_handle,
            } => {
                self.handle_set_processor_abort_handle(processor_id, abort_handle);
            }
            MediaPipelineCommand::TerminateProcessor {
                processor_id,
                reply_tx,
            } => {
                let _ = reply_tx.send(self.handle_terminate_processor(processor_id));
            }
            MediaPipelineCommand::FindUpstreamVideoEncoder {
                processor_id,
                reply_tx,
            } => {
                let _ = reply_tx.send(self.handle_find_upstream_video_encoder(processor_id));
            }
        }
    }

    fn handle_complete_initial_processor_registration(&mut self) -> bool {
        if self.registration_closed {
            return false;
        }
        self.registration_closed = true;

        for (processor_id, state) in &mut self.processors {
            state.is_initial_member = true;
            if !state.notified_ready {
                self.pending_initial_processors.insert(processor_id.clone());
            }
        }
        self.try_open_initial_ready();
        true
    }

    fn handle_notify_ready(&mut self, processor_id: ProcessorId) {
        let Some(state) = self.processors.get_mut(&processor_id) else {
            tracing::warn!("attempt to notify ready from unregistered processor: {processor_id}");
            return;
        };
        state.notified_ready = true;
        let rpc_sender_result = state
            .rpc_sender
            .clone()
            .ok_or(GetProcessorRpcSenderError::SenderNotRegistered);
        for waiter in state.pending_rpc_sender_waiters.drain(..) {
            let _ = waiter.send(rpc_sender_result.clone());
        }
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
        // [NOTE]
        // ここは順序保持を優先して Vec を使う。
        // track 数は通常少数なので、contains() の線形探索コストは許容する。
        if let Some(state) = self.processors.get_mut(&processor_id)
            && !state.subscribed_track_ids.contains(&track_id)
        {
            state.subscribed_track_ids.push(track_id.clone());
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
        track.publisher_processor_id = Some(processor_id.clone());
        // [NOTE]
        // ここも subscribed_track_ids と同様に、順序保持と実装単純性を優先して Vec を使う。
        if let Some(state) = self.processors.get_mut(&processor_id)
            && !state.published_track_ids.contains(&track_id)
        {
            state.published_track_ids.push(track_id.clone());
        }

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

    fn handle_register_processor(
        &mut self,
        processor_id: ProcessorId,
        metadata: ProcessorMetadata,
    ) -> bool {
        tracing::debug!("register processor: {processor_id}");

        if self.processors.contains_key(&processor_id) {
            tracing::warn!("processor already registered: {processor_id}");
            return false;
        }

        self.processors.insert(
            processor_id.clone(),
            ProcessorState {
                processor_type: metadata.processor_type().to_owned(),
                ..ProcessorState::default()
            },
        );
        true
    }

    fn handle_deregister_processor(&mut self, processor_id: ProcessorId) {
        // TODO: トラックが残っている間は deregister 扱いにしない
        tracing::debug!("deregister processor: {processor_id}");
        if let Some(mut state) = self.processors.remove(&processor_id) {
            for published_track_id in state.published_track_ids.drain(..) {
                if let Some(track) = self.tracks.get_mut(&published_track_id)
                    && track.publisher_processor_id.as_ref() == Some(&processor_id)
                {
                    track.publisher_processor_id = None;
                    track.publisher_command_tx = None;
                }
            }
            for waiter in state.pending_rpc_sender_waiters.drain(..) {
                let _ = waiter.send(Err(GetProcessorRpcSenderError::ProcessorNotFound));
            }
            if state.is_initial_member && !state.notified_ready {
                self.pending_initial_processors.remove(&processor_id);
            }
        }
        self.try_open_initial_ready();
    }

    fn handle_find_upstream_video_encoder(&self, processor_id: ProcessorId) -> Option<ProcessorId> {
        let mut queue = std::collections::VecDeque::new();
        let mut visited = std::collections::HashSet::new();
        queue.push_back(processor_id);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            let Some(state) = self.processors.get(&current) else {
                continue;
            };
            for subscribed_track_id in &state.subscribed_track_ids {
                let Some(track) = self.tracks.get(subscribed_track_id) else {
                    continue;
                };
                let Some(publisher_processor_id) = track.publisher_processor_id.as_ref() else {
                    continue;
                };
                let Some(publisher_state) = self.processors.get(publisher_processor_id) else {
                    continue;
                };
                if publisher_state.processor_type == PROCESSOR_TYPE_VIDEO_ENCODER {
                    return Some(publisher_processor_id.clone());
                }
                queue.push_back(publisher_processor_id.clone());
            }
        }

        None
    }

    fn handle_list_tracks(&self) -> Vec<TrackId> {
        self.tracks.keys().cloned().collect()
    }

    fn handle_list_processors(&self) -> Vec<ProcessorId> {
        self.processors.keys().cloned().collect()
    }

    fn handle_register_processor_rpc_sender(
        &mut self,
        processor_id: ProcessorId,
        sender: ErasedRpcSender,
    ) -> Result<(), RegisterProcessorRpcSenderError> {
        let Some(state) = self.processors.get_mut(&processor_id) else {
            tracing::warn!(
                "attempt to register RPC sender from unregistered processor: {processor_id}"
            );
            return Err(RegisterProcessorRpcSenderError::UnregisteredProcessor);
        };
        if state.rpc_sender.is_some() {
            tracing::warn!("RPC sender already registered for processor: {processor_id}");
            return Err(RegisterProcessorRpcSenderError::AlreadyRegistered);
        }
        state.rpc_sender = Some(sender);
        Ok(())
    }

    fn handle_get_processor_rpc_sender(
        &mut self,
        processor_id: ProcessorId,
        reply_tx: PendingRpcSenderWaiter,
    ) {
        let Some(state) = self.processors.get_mut(&processor_id) else {
            let _ = reply_tx.send(Err(GetProcessorRpcSenderError::ProcessorNotFound));
            return;
        };
        if state.notified_ready {
            let _ = reply_tx.send(
                state
                    .rpc_sender
                    .clone()
                    .ok_or(GetProcessorRpcSenderError::SenderNotRegistered),
            );
            return;
        }

        state.pending_rpc_sender_waiters.push(reply_tx);
    }

    fn handle_set_processor_abort_handle(
        &mut self,
        processor_id: ProcessorId,
        abort_handle: tokio::task::AbortHandle,
    ) {
        let Some(state) = self.processors.get_mut(&processor_id) else {
            abort_handle.abort();
            return;
        };
        state.abort_handle = Some(abort_handle);
    }

    fn handle_terminate_processor(&mut self, processor_id: ProcessorId) -> bool {
        let Some(state) = self.processors.get_mut(&processor_id) else {
            return false;
        };
        let Some(abort_handle) = state.abort_handle.take() else {
            return false;
        };
        abort_handle.abort();
        true
    }
}

#[derive(Debug, Clone)]
pub struct MediaPipelineHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<MediaPipelineCommand>,
    local_processor_task_tx: tokio::sync::mpsc::UnboundedSender<LocalProcessorTask>,
    stats: crate::stats::Stats,
    config: std::sync::Arc<MediaPipelineConfig>,
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
        let spawned_processor_id = processor_id.clone();
        let join_handle = tokio::spawn(async move {
            if let Err(e) = f(handle).await {
                error_flag.set(true);
                tracing::error!(
                    "failed to run processor {spawned_processor_id}: {}",
                    e.display()
                );
            }
        });
        self.send(MediaPipelineCommand::SetProcessorAbortHandle {
            processor_id,
            abort_handle: join_handle.abort_handle(),
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
            metadata: metadata.clone(),
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

    pub async fn get_rpc_sender<S>(
        &self,
        processor_id: &ProcessorId,
    ) -> Result<S, GetProcessorRpcSenderError>
    where
        S: Clone + Send + Sync + 'static,
    {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::GetProcessorRpcSender {
            processor_id: processor_id.clone(),
            reply_tx,
        });
        let erased_sender = reply_rx
            .await
            .map_err(|_| GetProcessorRpcSenderError::PipelineTerminated)??;
        erased_sender
            .downcast_clone::<S>()
            .ok_or(GetProcessorRpcSenderError::TypeMismatch)
    }

    /// 初期 processor 登録を完了して開始処理をトリガーする
    ///
    /// 返り値:
    /// - `Ok(true)`: 今回初めて開始処理が実行された
    /// - `Ok(false)`: すでに開始済みだった
    pub async fn trigger_start(&self) -> Result<bool, PipelineTerminated> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::TriggerStart { reply_tx });
        reply_rx.await.map_err(|_| PipelineTerminated)
    }

    pub fn stats(&self) -> crate::stats::Stats {
        self.stats.clone()
    }

    pub fn config(&self) -> std::sync::Arc<MediaPipelineConfig> {
        self.config.clone()
    }

    pub async fn list_processors(&self) -> Result<Vec<ProcessorId>, PipelineTerminated> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::ListProcessors { reply_tx });
        reply_rx.await.map_err(|_| PipelineTerminated)
    }

    pub async fn terminate_processor(
        &self,
        processor_id: ProcessorId,
    ) -> Result<bool, PipelineTerminated> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::TerminateProcessor {
            processor_id,
            reply_tx,
        });
        reply_rx.await.map_err(|_| PipelineTerminated)
    }

    pub async fn find_upstream_video_encoder(
        &self,
        processor_id: &ProcessorId,
    ) -> Result<Option<ProcessorId>, PipelineTerminated> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(MediaPipelineCommand::FindUpstreamVideoEncoder {
            processor_id: processor_id.clone(),
            reply_tx,
        });
        reply_rx.await.map_err(|_| PipelineTerminated)
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
        metadata: ProcessorMetadata,
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
    TriggerStart {
        reply_tx: tokio::sync::oneshot::Sender<bool>,
    },
    NotifyReady {
        processor_id: ProcessorId,
    },
    WaitSubscribersReady {
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<()>,
    },
    RegisterProcessorRpcSender {
        processor_id: ProcessorId,
        sender: ErasedRpcSender,
        reply_tx: tokio::sync::oneshot::Sender<Result<(), RegisterProcessorRpcSenderError>>,
    },
    GetProcessorRpcSender {
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<Result<ErasedRpcSender, GetProcessorRpcSenderError>>,
    },
    SetProcessorAbortHandle {
        processor_id: ProcessorId,
        abort_handle: tokio::task::AbortHandle,
    },
    TerminateProcessor {
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<bool>,
    },
    FindUpstreamVideoEncoder {
        processor_id: ProcessorId,
        reply_tx: tokio::sync::oneshot::Sender<Option<ProcessorId>>,
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
    publisher_processor_id: Option<ProcessorId>,
    pending_subscribers: Vec<tokio::sync::mpsc::UnboundedSender<Message>>,
}

#[derive(Clone)]
pub(crate) struct ErasedRpcSender {
    inner: std::sync::Arc<dyn std::any::Any + Send + Sync>,
}

impl ErasedRpcSender {
    fn new<S>(sender: S) -> Self
    where
        S: Clone + Send + Sync + 'static,
    {
        Self {
            inner: std::sync::Arc::new(sender),
        }
    }

    fn downcast_clone<S>(&self) -> Option<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        self.inner.downcast_ref::<S>().cloned()
    }
}

impl std::fmt::Debug for ErasedRpcSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ErasedRpcSender(..)")
    }
}

#[derive(Debug)]
struct ProcessorState {
    processor_type: String,
    notified_ready: bool,
    is_initial_member: bool,
    subscribed_track_ids: Vec<TrackId>,
    published_track_ids: Vec<TrackId>,
    rpc_sender: Option<ErasedRpcSender>,
    abort_handle: Option<tokio::task::AbortHandle>,
    pending_rpc_sender_waiters: Vec<PendingRpcSenderWaiter>,
}

impl Default for ProcessorState {
    fn default() -> Self {
        Self {
            processor_type: "unknown".to_owned(),
            notified_ready: false,
            is_initial_member: false,
            subscribed_track_ids: Vec::new(),
            published_track_ids: Vec::new(),
            rpc_sender: None,
            abort_handle: None,
            pending_rpc_sender_waiters: Vec::new(),
        }
    }
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

    pub fn config(&self) -> std::sync::Arc<MediaPipelineConfig> {
        self.pipeline_handle.config()
    }

    pub fn pipeline_handle(&self) -> MediaPipelineHandle {
        self.pipeline_handle.clone()
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

    /// Processor 固有の RPC sender を登録する
    pub async fn register_rpc_sender<S>(
        &self,
        sender: S,
    ) -> Result<(), RegisterProcessorRpcSenderError>
    where
        S: Clone + Send + Sync + 'static,
    {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.pipeline_handle
            .send(MediaPipelineCommand::RegisterProcessorRpcSender {
                processor_id: self.processor_id.clone(),
                sender: ErasedRpcSender::new(sender),
                reply_tx,
            });
        reply_rx
            .await
            .map_err(|_| RegisterProcessorRpcSenderError::PipelineTerminated)?
    }
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
    Media(crate::MediaFrame),
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

    pub fn has_subscribers(&mut self) -> bool {
        if !self.drain_track_commands() {
            return false;
        }
        self.txs.retain(|tx| !tx.is_closed());
        !self.txs.is_empty()
    }

    pub fn send_media(&mut self, sample: crate::MediaFrame) -> bool {
        self.send(Message::Media(sample))
    }

    pub fn send_audio(&mut self, frame: crate::AudioFrame) -> bool {
        self.send(Message::Media(crate::MediaFrame::new_audio(frame)))
    }

    pub fn send_video(&mut self, frame: crate::VideoFrame) -> bool {
        self.send(Message::Media(crate::MediaFrame::new_video(frame)))
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
pub enum RegisterProcessorRpcSenderError {
    /// パイプラインが終了している
    PipelineTerminated,
    /// プロセッサーが未登録
    UnregisteredProcessor,
    /// RPC sender がすでに登録済み
    AlreadyRegistered,
}

impl std::fmt::Display for RegisterProcessorRpcSenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PipelineTerminated => write!(f, "Pipeline has terminated"),
            Self::UnregisteredProcessor => write!(f, "Processor is not registered"),
            Self::AlreadyRegistered => write!(f, "RPC sender already registered"),
        }
    }
}

impl std::error::Error for RegisterProcessorRpcSenderError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetProcessorRpcSenderError {
    /// パイプラインが終了している
    PipelineTerminated,
    /// プロセッサーが未登録
    ProcessorNotFound,
    /// RPC sender が未登録
    SenderNotRegistered,
    /// 期待型と登録型が一致しない
    TypeMismatch,
}

impl std::fmt::Display for GetProcessorRpcSenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PipelineTerminated => write!(f, "Pipeline has terminated"),
            Self::ProcessorNotFound => write!(f, "Processor not found"),
            Self::SenderNotRegistered => write!(f, "RPC sender is not registered"),
            Self::TypeMismatch => write!(f, "RPC sender type mismatch"),
        }
    }
}

impl std::error::Error for GetProcessorRpcSenderError {}

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

/// パイプライン操作メソッドのエラー型
#[derive(Debug)]
pub enum PipelineOperationError {
    /// プロセッサーID が重複している
    DuplicateProcessorId(ProcessorId),
    /// パイプラインが終了している
    PipelineTerminated,
    /// パラメータが不正
    InvalidParams(String),
    /// 内部エラー
    InternalError(String),
    /// リクエストが不正
    InvalidRequest(String),
}

impl std::fmt::Display for PipelineOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateProcessorId(id) => {
                write!(f, "Processor ID already exists: {id}")
            }
            Self::PipelineTerminated => write!(f, "Pipeline has terminated"),
            Self::InvalidParams(msg) => write!(f, "Invalid params: {msg}"),
            Self::InternalError(msg) => write!(f, "Internal error: {msg}"),
            Self::InvalidRequest(msg) => write!(f, "Invalid request: {msg}"),
        }
    }
}

impl From<RegisterProcessorError> for PipelineOperationError {
    fn from(e: RegisterProcessorError) -> Self {
        match e {
            RegisterProcessorError::DuplicateProcessorId => {
                // processor_id が不明な場合は空の ProcessorId を使う
                // 呼び出し元で適切な processor_id を設定すること
                Self::DuplicateProcessorId(ProcessorId::new(""))
            }
            RegisterProcessorError::PipelineTerminated => Self::PipelineTerminated,
        }
    }
}

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
        assert!(
            handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );

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
        assert!(
            handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );

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

            assert!(
                handle
                    .trigger_start()
                    .await
                    .expect("trigger_start must succeed")
            );
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
        assert!(
            handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );
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
        assert!(
            handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );
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
    async fn trigger_start_is_idempotent() {
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
        assert!(
            handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );
        assert!(
            !handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );
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
    async fn register_and_get_rpc_sender_succeeds() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let processor = handle
            .register_processor(
                ProcessorId::new("rpc_processor"),
                metadata("test_rpc_processor"),
            )
            .await
            .expect("failed to register processor");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let processor_id = processor.processor_id().clone();

        processor
            .register_rpc_sender(tx.clone())
            .await
            .expect("failed to register rpc sender");
        {
            let rpc_handle = handle.clone();
            let get_fut = rpc_handle
                .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<String>>(&processor_id);
            tokio::pin!(get_fut);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut get_fut)
                    .await
                    .is_err()
            );
            processor.notify_ready();
            let rpc_tx = tokio::time::timeout(Duration::from_secs(2), &mut get_fut)
                .await
                .expect("timed out waiting rpc sender")
                .expect("failed to get rpc sender");

            rpc_tx
                .send("hello".to_owned())
                .expect("failed to send message via rpc sender");
        }

        let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting rpc message")
            .expect("rpc receiver closed unexpectedly");
        assert_eq!(received, "hello");

        drop(processor);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn get_rpc_sender_returns_sender_not_registered_before_registration() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let processor = handle
            .register_processor(
                ProcessorId::new("rpc_sender_missing"),
                metadata("test_rpc_processor"),
            )
            .await
            .expect("failed to register processor");
        let processor_id = processor.processor_id().clone();

        let result = {
            let rpc_handle = handle.clone();
            let get_fut = rpc_handle
                .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<String>>(&processor_id);
            tokio::pin!(get_fut);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut get_fut)
                    .await
                    .is_err()
            );
            processor.notify_ready();
            tokio::time::timeout(Duration::from_secs(2), &mut get_fut)
                .await
                .expect("timed out waiting sender-not-registered result")
        };
        assert!(matches!(
            result,
            Err(GetProcessorRpcSenderError::SenderNotRegistered)
        ));

        drop(processor);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn register_rpc_sender_rejects_duplicate_registration() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let processor = handle
            .register_processor(
                ProcessorId::new("rpc_sender_duplicate"),
                metadata("test_rpc_processor"),
            )
            .await
            .expect("failed to register processor");

        let (tx1, _rx1) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel::<String>();
        processor
            .register_rpc_sender(tx1)
            .await
            .expect("first rpc sender registration must succeed");
        let result = processor.register_rpc_sender(tx2).await;
        assert_eq!(
            result,
            Err(RegisterProcessorRpcSenderError::AlreadyRegistered)
        );

        drop(processor);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn get_rpc_sender_returns_type_mismatch_for_wrong_type() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let processor = handle
            .register_processor(
                ProcessorId::new("rpc_type_mismatch"),
                metadata("test_rpc_processor"),
            )
            .await
            .expect("failed to register processor");
        let processor_id = processor.processor_id().clone();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        processor
            .register_rpc_sender(tx)
            .await
            .expect("failed to register rpc sender");
        processor.notify_ready();

        let result = handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<u64>>(&processor_id)
            .await;
        assert!(matches!(
            result,
            Err(GetProcessorRpcSenderError::TypeMismatch)
        ));

        drop(processor);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn get_rpc_sender_returns_processor_not_found() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let result = handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<String>>(&ProcessorId::new(
                "unknown_processor",
            ))
            .await;
        assert!(matches!(
            result,
            Err(GetProcessorRpcSenderError::ProcessorNotFound)
        ));

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn rpc_sender_is_removed_on_processor_drop() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let processor_id = ProcessorId::new("rpc_drop_target");
        {
            let processor = handle
                .register_processor(processor_id.clone(), metadata("test_rpc_processor"))
                .await
                .expect("failed to register processor");
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            processor
                .register_rpc_sender(tx)
                .await
                .expect("failed to register rpc sender");
        }

        let mut removed = false;
        for _ in 0..200 {
            match handle
                .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<String>>(&processor_id)
                .await
            {
                Err(GetProcessorRpcSenderError::ProcessorNotFound) => {
                    removed = true;
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        assert!(removed, "processor entry was not removed after drop");

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn register_and_get_rpc_sender_return_error_after_pipeline_terminated() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        let processor = handle
            .register_processor(
                ProcessorId::new("rpc_after_terminated"),
                metadata("test_rpc_processor"),
            )
            .await
            .expect("failed to register processor");
        let processor_id = processor.processor_id().clone();

        pipeline_task.abort();
        let _ = pipeline_task.await;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let register_result = processor.register_rpc_sender(tx).await;
        assert_eq!(
            register_result,
            Err(RegisterProcessorRpcSenderError::PipelineTerminated)
        );
        let get_result = handle
            .get_rpc_sender::<tokio::sync::mpsc::UnboundedSender<String>>(&processor_id)
            .await;
        assert!(matches!(
            get_result,
            Err(GetProcessorRpcSenderError::PipelineTerminated)
        ));
    }

    #[tokio::test]
    async fn find_upstream_video_encoder_returns_nearest_encoder() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let source = handle
            .register_processor(ProcessorId::new("source"), metadata("png_file_source"))
            .await
            .expect("failed to register source");
        let encoder = handle
            .register_processor(
                ProcessorId::new("encoder"),
                metadata(PROCESSOR_TYPE_VIDEO_ENCODER),
            )
            .await
            .expect("failed to register encoder");
        let endpoint = handle
            .register_processor(
                ProcessorId::new("endpoint"),
                metadata("rtmp_outbound_endpoint"),
            )
            .await
            .expect("failed to register endpoint");

        let source_track = TrackId::new("source_track");
        let encoded_track = TrackId::new("encoded_track");
        let _source_tx = source
            .publish_track(source_track.clone())
            .await
            .expect("source publish must succeed");
        let _encoder_rx = encoder.subscribe_track(source_track);
        let _encoder_tx = encoder
            .publish_track(encoded_track.clone())
            .await
            .expect("encoder publish must succeed");
        let _endpoint_rx = endpoint.subscribe_track(encoded_track);

        let found = handle
            .find_upstream_video_encoder(endpoint.processor_id())
            .await
            .expect("find_upstream_video_encoder must succeed");
        assert_eq!(found, Some(encoder.processor_id().clone()));

        drop(endpoint);
        drop(encoder);
        drop(source);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn find_upstream_video_encoder_returns_none_for_best_effort_path() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let source = handle
            .register_processor(ProcessorId::new("source"), metadata("png_file_source"))
            .await
            .expect("failed to register source");
        let endpoint = handle
            .register_processor(
                ProcessorId::new("endpoint"),
                metadata("rtmp_outbound_endpoint"),
            )
            .await
            .expect("failed to register endpoint");

        let source_track = TrackId::new("source_track");
        let _source_tx = source
            .publish_track(source_track.clone())
            .await
            .expect("source publish must succeed");
        let _endpoint_rx = endpoint.subscribe_track(source_track);

        // RTMP 再生開始時のキーフレーム要求は best effort のため、
        // 上流に video encoder が存在しない構成では None を正常系として扱う。
        let found = handle
            .find_upstream_video_encoder(endpoint.processor_id())
            .await
            .expect("find_upstream_video_encoder must succeed");
        assert_eq!(found, None);

        drop(endpoint);
        drop(source);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
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
        assert!(
            handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );

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
        assert!(
            handle
                .trigger_start()
                .await
                .expect("trigger_start must succeed")
        );

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
