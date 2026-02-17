type LocalProcessorTask = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug)]
pub struct MediaPipeline {
    command_tx: Option<tokio::sync::mpsc::UnboundedSender<MediaPipelineCommand>>,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<MediaPipelineCommand>,
    local_processor_task_tx: Option<tokio::sync::mpsc::UnboundedSender<LocalProcessorTask>>,
    local_processor_thread: std::thread::JoinHandle<()>,
    processors: std::collections::HashSet<ProcessorId>,
    tracks: std::collections::HashMap<TrackId, TrackState>,
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
            processors: std::collections::HashSet::new(),
            tracks: std::collections::HashMap::new(),
        })
    }

    /// このパイプラインを操作するためのハンドルを返す
    ///
    /// 全てのハンドルが（間接的なものも含めて）ドロップしたら、パイプラインも終了する
    pub fn handle(&self) -> MediaPipelineHandle {
        MediaPipelineHandle {
            command_tx: self.command_tx.clone().expect("infallible"),
            local_processor_task_tx: self.local_processor_task_tx.clone().expect("infallible"),
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
        }
    }

    fn handle_subscribe_track(
        &mut self,
        processor_id: ProcessorId,
        track_id: TrackId,
        tx: tokio::sync::mpsc::UnboundedSender<Message>,
    ) {
        tracing::debug!("subscribe track: processor={processor_id}, track={track_id}");

        if !self.processors.contains(&processor_id) {
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

        if let Some(publisher_tx) = &track.publisher_command_tx {
            let _ = publisher_tx.send(TrackCommand::AddSubscriber(tx));
        } else {
            // publisher がまだ登録されていない場合は、subscriber を待機キューに追加
            tracing::debug!("publisher not yet registered for track: {track_id}");
            track.pending_subscribers.push(tx);
        }

        // TODO: 将来的には不要となったトラックの削除の仕組みを追加する
        //
        // 例えば、
        // - 参照していた全ての publisher がいなくなったら削除する
        // - Message{Sender,Receiver} のドロップ時に解除コマンドを送る
        //
        // ただし、現状ではこの対応を入れなくても別に困らないため、実際に削除が必要になるまでは残り続けて構わない
    }

    fn handle_publish_track(
        &mut self,
        processor_id: ProcessorId,
        track_id: TrackId,
    ) -> Result<MessageSender, PublishTrackError> {
        tracing::debug!("publish track: processor={processor_id}, track={track_id}");

        if !self.processors.contains(&processor_id) {
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

        // 既に待機中の subscriber に通知
        for subscriber_tx in track.pending_subscribers.drain(..) {
            let _ = command_tx.send(TrackCommand::AddSubscriber(subscriber_tx));
        }

        Ok(MessageSender {
            rx: command_rx,
            txs: Vec::new(),
            has_first_subscriber: false,
        })
    }

    fn handle_register_processor(&mut self, processor_id: ProcessorId) -> bool {
        tracing::debug!("register processor: {processor_id}");

        if self.processors.contains(&processor_id) {
            tracing::warn!("processor already registered: {processor_id}");
            return false;
        }

        self.processors.insert(processor_id.clone());
        true
    }

    fn handle_deregister_processor(&mut self, processor_id: ProcessorId) {
        // TODO: トラックが残っている間は deregister 扱いにしない
        tracing::debug!("deregister processor: {processor_id}");
        self.processors.remove(&processor_id);
    }

    fn handle_list_tracks(&self) -> Vec<TrackId> {
        self.tracks.keys().cloned().collect()
    }

    fn handle_list_processors(&self) -> Vec<ProcessorId> {
        self.processors.iter().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct MediaPipelineHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<MediaPipelineCommand>,
    local_processor_task_tx: tokio::sync::mpsc::UnboundedSender<LocalProcessorTask>,
}

impl MediaPipelineHandle {
    pub async fn spawn_processor<F, T>(
        &self,
        processor_id: ProcessorId,
        f: F,
    ) -> Result<(), RegisterProcessorError>
    where
        F: FnOnce(ProcessorHandle) -> T + Send + 'static,
        T: Future<Output = crate::Result<()>> + Send,
    {
        let handle = self.register_processor(processor_id.clone()).await?;
        tokio::spawn(async move {
            if let Err(e) = f(handle).await {
                tracing::error!("failed to run processor {processor_id}: {e}");
            }
        });
        Ok(())
    }

    pub async fn spawn_local_processor<F, T>(
        &self,
        processor_id: ProcessorId,
        f: F,
    ) -> Result<(), RegisterProcessorError>
    where
        F: FnOnce(ProcessorHandle) -> T + Send + 'static,
        T: Future<Output = crate::Result<()>> + 'static,
    {
        let handle = self.register_processor(processor_id.clone()).await?;
        let task: LocalProcessorTask = Box::new(move || {
            tokio::task::spawn_local(async move {
                if let Err(e) = f(handle).await {
                    tracing::error!("failed to run processor {processor_id}: {e}");
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
    ) -> Result<ProcessorHandle, RegisterProcessorError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = MediaPipelineCommand::RegisterProcessor {
            processor_id: processor_id.clone(),
            reply_tx,
        };

        // [NOTE] パイプライン終了は次の rx で判定できるのでここでは返り値の考慮は不要
        self.send(command);

        match reply_rx.await {
            Ok(true) => Ok(ProcessorHandle {
                pipeline_handle: self.clone(),
                processor_id,
            }),
            Ok(false) => Err(RegisterProcessorError::DuplicateProcessorId),
            Err(_) => Err(RegisterProcessorError::PipelineTerminated),
        }
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

#[derive(Debug)]
pub struct ProcessorHandle {
    pipeline_handle: MediaPipelineHandle,
    processor_id: ProcessorId,
}

impl ProcessorHandle {
    pub fn processor_id(&self) -> &ProcessorId {
        &self.processor_id
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
    has_first_subscriber: bool,
}

impl MessageSender {
    // MediaPipeline が途中終了した場合は false が返される
    pub async fn send(&mut self, message: Message) -> bool {
        if !self.has_first_subscriber && !self.wait_for_first_subscriber().await {
            return false;
        }
        if !self.drain_track_commands() {
            return false;
        }

        self.txs.retain_mut(|tx| tx.send(message.clone()).is_ok());
        true
    }

    pub async fn send_media(&mut self, sample: crate::MediaSample) -> bool {
        self.send(Message::Media(sample)).await
    }

    pub async fn send_audio(&mut self, data: crate::AudioData) -> bool {
        self.send(Message::Media(crate::MediaSample::new_audio(data)))
            .await
    }

    pub async fn send_video(&mut self, frame: crate::VideoFrame) -> bool {
        self.send(Message::Media(crate::MediaSample::new_video(frame)))
            .await
    }

    pub async fn send_eos(&mut self) -> bool {
        self.send(Message::Eos).await
    }

    pub async fn send_syn(&mut self) -> Ack {
        let (tx, rx) = tokio::sync::mpsc::channel(1); // NOTE: 0 だとエラーになる
        let _ = self.send(Message::Syn(Syn(tx))).await; // NOTE: ここでは false を特別扱いする必要はないので無視する
        Ack(rx)
    }

    async fn wait_for_first_subscriber(&mut self) -> bool {
        if let Some(TrackCommand::AddSubscriber(tx)) = self.rx.recv().await {
            self.txs.push(tx);
            self.has_first_subscriber = true;
            return self.drain_track_commands();
        }

        self.txs.clear();
        false
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

    #[tokio::test]
    async fn spawn_local_processor_accepts_non_send_future() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<usize>();
        handle
            .spawn_local_processor(
                ProcessorId::new("local-non-send"),
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

        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        handle
            .spawn_local_processor(
                ProcessorId::new("duplicate-local"),
                move |_handle| async move {
                    let _ = release_rx.await;
                    Ok(())
                },
            )
            .await
            .expect("first spawn_local_processor must succeed");

        let result = handle
            .spawn_local_processor(
                ProcessorId::new("duplicate-local"),
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
    async fn send_waits_until_first_subscriber_arrives() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let sender = handle
            .register_processor(ProcessorId::new("wait_sender"))
            .await
            .expect("failed to register sender");
        let track_id = TrackId::new("wait-track");
        let mut tx = sender
            .publish_track(track_id.clone())
            .await
            .expect("failed to publish track");

        let send = tx.send_eos();
        tokio::pin!(send);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut send)
                .await
                .is_err(),
            "send_eos must wait until first subscriber arrives"
        );

        let receiver = handle
            .register_processor(ProcessorId::new("wait_receiver"))
            .await
            .expect("failed to register receiver");
        let mut rx = receiver.subscribe_track(track_id);

        assert!(
            tokio::time::timeout(Duration::from_secs(2), &mut send)
                .await
                .expect("send_eos did not finish after subscriber arrived"),
            "send_eos must succeed after first subscriber arrives"
        );

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
    async fn send_syn_waits_until_first_subscriber_and_ack_waits_for_drop() {
        let pipeline = MediaPipeline::new().expect("failed to create test media pipeline");
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());

        let sender = handle
            .register_processor(ProcessorId::new("syn_sender"))
            .await
            .expect("failed to register sender");
        let track_id = TrackId::new("syn-track");
        let mut tx = sender
            .publish_track(track_id.clone())
            .await
            .expect("failed to publish track");

        let send_syn = tx.send_syn();
        tokio::pin!(send_syn);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut send_syn)
                .await
                .is_err(),
            "send_syn must wait until first subscriber arrives"
        );

        let receiver = handle
            .register_processor(ProcessorId::new("syn_receiver"))
            .await
            .expect("failed to register receiver");
        let mut rx = receiver.subscribe_track(track_id);

        let ack = tokio::time::timeout(Duration::from_secs(2), &mut send_syn)
            .await
            .expect("send_syn did not finish after subscriber arrived");
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
}
