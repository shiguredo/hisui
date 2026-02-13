#[derive(Debug)]
pub struct MediaPipeline {
    command_tx: Option<tokio::sync::mpsc::UnboundedSender<Command>>,
    command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
    processors: std::collections::HashSet<ProcessorId>,
    tracks: std::collections::HashMap<TrackId, TrackState>,
}

impl MediaPipeline {
    pub fn new() -> Self {
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            command_tx: Some(command_tx),
            command_rx,
            processors: std::collections::HashSet::new(),
            tracks: std::collections::HashMap::new(),
        }
    }

    /// このパイプラインを操作するためのハンドルを返す
    ///
    /// 全てのハンドルが（間接的なものも含めて）ドロップしたら、パイプラインも終了する
    pub fn handle(&self) -> MediaPipelineHandle {
        MediaPipelineHandle {
            command_tx: self.command_tx.clone().expect("infallible"),
        }
    }

    pub async fn run(mut self) {
        tracing::debug!("MediaPipeline started");

        self.command_tx = None; // 参照カウントから自分を外すために None にする

        loop {
            tokio::select! {
                Some(command) = self.command_rx.recv() => self.handle_command(command),
                else => break,
            }
        }

        tracing::debug!("MediaPipeline stopped");
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::RegisterProcessor {
                processor_id,
                reply_tx,
            } => {
                let result = self.handle_register_processor(processor_id);
                let _ = reply_tx.send(result); // 応答では受信側がすでに閉じていても問題ないので、結果の確認は不要（以降も同様）
            }
            Command::DeregisterProcessor { processor_id } => {
                self.handle_deregister_processor(processor_id);
            }
            Command::PublishTrack {
                processor_id,
                track_id,
                reply_tx,
            } => {
                let result = self.handle_publish_track(processor_id, track_id);
                let _ = reply_tx.send(result);
            }
            Command::SubscribeTrack {
                processor_id,
                track_id,
                tx,
            } => {
                self.handle_subscribe_track(processor_id, track_id, tx);
            }
            Command::ListTracks { reply_tx } => {
                let _ = reply_tx.send(self.handle_list_tracks());
            }
            Command::ListProcessors { reply_tx } => {
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

impl Default for MediaPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct MediaPipelineHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<Command>,
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

    /// [NOTE] こちらは内部寄りなので、可能な限りは spawn_processor() を使うこと
    pub async fn register_processor(
        &self,
        processor_id: ProcessorId,
    ) -> Result<ProcessorHandle, RegisterProcessorError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = Command::RegisterProcessor {
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

    // JSON-RPC リクエストを処理する
    //
    // 通知の場合は None が、それ以外ならクライアントに返すレスポンス JSON が返される
    pub async fn rpc(&self, request_bytes: &[u8]) -> Option<nojson::RawJsonOwned> {
        let request_json = match crate::jsonrpc::parse_request_bytes(request_bytes) {
            Err(error_response) => return Some(error_response),
            Ok(json) => json,
        };
        let request = request_json.value();

        // parse_request_bytes() の中でバリデーションしているので、ここは常に成功する
        let method = request
            .to_member("method")
            .expect("bug")
            .required()
            .expect("bug")
            .as_string_str()
            .expect("bug");
        let maybe_id = request.to_member("id").ok().and_then(|v| v.get());
        let maybe_params = request.to_member("params").ok().and_then(|v| v.get());

        let result = match method {
            "createMp4FileSource" => self.handle_create_mp4_file_source_rpc(maybe_params).await,
            "createVideoMixer" => self.handle_create_video_mixer_rpc(maybe_params).await,
            "listTracks" => self.handle_list_tracks_rpc().await,
            "listProcessors" => self.handle_list_processors_rpc().await,
            _ => Err((
                crate::jsonrpc::METHOD_NOT_FOUND,
                "Method not found".to_owned(),
            )),
        };

        maybe_id.map(|id| match result {
            Ok(v) => crate::jsonrpc::ok_response(id, v),
            Err((code, e)) => crate::jsonrpc::error_response(id, code, e),
        })
    }

    async fn handle_create_mp4_file_source_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcResult, (i32, String)> {
        let params = maybe_params.ok_or_else(|| {
            (
                crate::jsonrpc::INVALID_PARAMS,
                "Invalid params: params is required".to_owned(),
            )
        })?;

        let source: crate::Mp4FileSource = params.try_into().map_err(|e| {
            (
                crate::jsonrpc::INVALID_PARAMS,
                format!("Invalid params: {e}"),
            )
        })?;
        let processor_id: Option<ProcessorId> = params
            .to_member("processorId")
            .map_err(|e| {
                (
                    crate::jsonrpc::INVALID_PARAMS,
                    format!("Invalid params: {e}"),
                )
            })?
            .try_into()
            .map_err(|e| {
                (
                    crate::jsonrpc::INVALID_PARAMS,
                    format!("Invalid params: {e}"),
                )
            })?;
        let processor_id = processor_id.unwrap_or_else(|| {
            ProcessorId::new(source.path.as_os_str().to_string_lossy().into_owned())
        });

        self.spawn_processor(processor_id.clone(), move |handle| source.run(handle))
            .await
            .map_err(|e| match e {
                RegisterProcessorError::DuplicateProcessorId => (
                    crate::jsonrpc::INVALID_PARAMS,
                    format!("Invalid params: processorId already exists: {processor_id}"),
                ),
                RegisterProcessorError::PipelineTerminated => (
                    crate::jsonrpc::INTERNAL_ERROR,
                    "Internal error: pipeline has terminated".to_owned(),
                ),
            })?;

        Ok(RpcResult::CreateMp4FileSource(
            CreateMp4FileSourceRpcResult { processor_id },
        ))
    }

    async fn handle_create_video_mixer_rpc(
        &self,
        maybe_params: Option<nojson::RawJsonValue<'_, '_>>,
    ) -> Result<RpcResult, (i32, String)> {
        let params = maybe_params.ok_or_else(|| {
            (
                crate::jsonrpc::INVALID_PARAMS,
                "Invalid params: params is required".to_owned(),
            )
        })?;

        let mixer: crate::mixer_realtime_video::VideoRealtimeMixer =
            params.try_into().map_err(|e| {
                (
                    crate::jsonrpc::INVALID_PARAMS,
                    format!("Invalid params: {e}"),
                )
            })?;
        let processor_id: Option<ProcessorId> = params
            .to_member("processorId")
            .map_err(|e| {
                (
                    crate::jsonrpc::INVALID_PARAMS,
                    format!("Invalid params: {e}"),
                )
            })?
            .try_into()
            .map_err(|e| {
                (
                    crate::jsonrpc::INVALID_PARAMS,
                    format!("Invalid params: {e}"),
                )
            })?;
        let processor_id = processor_id.unwrap_or_else(|| ProcessorId::new("videoMixer"));

        self.spawn_processor(processor_id.clone(), move |handle| mixer.run(handle))
            .await
            .map_err(|e| match e {
                RegisterProcessorError::DuplicateProcessorId => (
                    crate::jsonrpc::INVALID_PARAMS,
                    format!("Invalid params: processorId already exists: {processor_id}"),
                ),
                RegisterProcessorError::PipelineTerminated => (
                    crate::jsonrpc::INTERNAL_ERROR,
                    "Internal error: pipeline has terminated".to_owned(),
                ),
            })?;

        Ok(RpcResult::CreateVideoMixer(CreateVideoMixerRpcResult {
            processor_id,
        }))
    }

    async fn handle_list_tracks_rpc(&self) -> Result<RpcResult, (i32, String)> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(Command::ListTracks { reply_tx });

        let track_ids = reply_rx.await.map_err(|_| {
            (
                crate::jsonrpc::INTERNAL_ERROR,
                "Internal error: pipeline has terminated".to_owned(),
            )
        })?;

        Ok(RpcResult::ListTracks(
            track_ids
                .into_iter()
                .map(|track_id| ListTrackRpcItem { track_id })
                .collect(),
        ))
    }

    async fn handle_list_processors_rpc(&self) -> Result<RpcResult, (i32, String)> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.send(Command::ListProcessors { reply_tx });

        let processor_ids = reply_rx.await.map_err(|_| {
            (
                crate::jsonrpc::INTERNAL_ERROR,
                "Internal error: pipeline has terminated".to_owned(),
            )
        })?;

        Ok(RpcResult::ListProcessors(
            processor_ids
                .into_iter()
                .map(|processor_id| ListProcessorRpcItem { processor_id })
                .collect(),
        ))
    }

    // すでに MediaPipeline が終了している場合には false が返される。
    // なお、通常はこの結果をハンドリングする必要はない。
    // （コマンドの応答を受け取る場合は、その受信側で検知できるし、
    //   応答を受け取らない場合にはそもそもここの成功・失敗に依存するようなコマンドであるべきではないため）
    fn send(&self, command: Command) -> bool {
        self.command_tx.send(command).is_ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CreateMp4FileSourceRpcResult {
    processor_id: ProcessorId,
}

impl nojson::DisplayJson for CreateMp4FileSourceRpcResult {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| f.member("processorId", &self.processor_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CreateVideoMixerRpcResult {
    processor_id: ProcessorId,
}

impl nojson::DisplayJson for CreateVideoMixerRpcResult {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| f.member("processorId", &self.processor_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListTrackRpcItem {
    track_id: TrackId,
}

impl nojson::DisplayJson for ListTrackRpcItem {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| f.member("trackId", &self.track_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListProcessorRpcItem {
    processor_id: ProcessorId,
}

impl nojson::DisplayJson for ListProcessorRpcItem {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| f.member("processorId", &self.processor_id))
    }
}

enum RpcResult {
    CreateMp4FileSource(CreateMp4FileSourceRpcResult),
    CreateVideoMixer(CreateVideoMixerRpcResult),
    ListTracks(Vec<ListTrackRpcItem>),
    ListProcessors(Vec<ListProcessorRpcItem>),
}

impl nojson::DisplayJson for RpcResult {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            Self::CreateMp4FileSource(v) => v.fmt(f),
            Self::CreateVideoMixer(v) => v.fmt(f),
            Self::ListTracks(v) => v.fmt(f),
            Self::ListProcessors(v) => v.fmt(f),
        }
    }
}

#[derive(Debug)]
enum Command {
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
        let command = Command::PublishTrack {
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
        let command = Command::SubscribeTrack {
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
        self.pipeline_handle.send(Command::DeregisterProcessor {
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
        loop {
            match self.rx.try_recv() {
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    break;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.txs.clear();
                    return false;
                }
                Ok(TrackCommand::AddSubscriber(tx)) => {
                    self.txs.push(tx);
                }
            }
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
    use std::time::Duration;

    use super::*;

    const TEST_MP4_PATH: &str = "testdata/archive-red-320x320-av1.mp4";

    #[tokio::test]
    async fn create_mp4_file_source_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(error_code(&response), crate::jsonrpc::INVALID_PARAMS);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_validates_mp4_source_params() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource","params":{{"path":"{TEST_MP4_PATH}"}}}}"#
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(error_code(&response), crate::jsonrpc::INVALID_PARAMS);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_uses_path_as_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource","params":{{"path":"{TEST_MP4_PATH}","realtime":false,"loopPlayback":false,"videoTrackId":"video-default-id"}}}}"#
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(result_processor_id(&response), TEST_MP4_PATH);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource","params":{{"path":"{TEST_MP4_PATH}","processorId":"custom-source","realtime":false,"loopPlayback":false,"videoTrackId":"video-custom-id"}}}}"#
        );

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(result_processor_id(&response), "custom-source");

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_mp4_file_source_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createMp4FileSource","params":{{"path":"{TEST_MP4_PATH}","processorId":"duplicate-source","realtime":true,"loopPlayback":false,"videoTrackId":"video-duplicate-id"}}}}"#
        );

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(result_processor_id(&first_response), "duplicate-source");

        let second_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(error_code(&second_response), crate::jsonrpc::INVALID_PARAMS);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_requires_params() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"createVideoMixer"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(error_code(&response), crate::jsonrpc::INVALID_PARAMS);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_validates_params() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request =
            r#"{"jsonrpc":"2.0","id":1,"method":"createVideoMixer","params":{"canvasWidth":640}}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(error_code(&response), crate::jsonrpc::INVALID_PARAMS);

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_uses_default_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let blocker = handle
            .register_processor(ProcessorId::new("video-mixer-blocker"))
            .await
            .expect("register video-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("video-mixer-output"))
            .await
            .expect("publish video-mixer-output");
        let request = create_video_mixer_request("video-mixer-output", None);

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(result_processor_id(&response), "videoMixer");

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_uses_explicit_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let blocker = handle
            .register_processor(ProcessorId::new("video-mixer-blocker"))
            .await
            .expect("register video-mixer-blocker");
        let occupied_sender = blocker
            .publish_track(TrackId::new("video-mixer-output"))
            .await
            .expect("publish video-mixer-output");
        let request = create_video_mixer_request("video-mixer-output", Some("custom-video-mixer"));

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert_eq!(result_processor_id(&response), "custom-video-mixer");

        drop(occupied_sender);
        drop(blocker);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn create_video_mixer_rejects_duplicate_processor_id() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request =
            create_video_mixer_request("video-mixer-output-dup", Some("duplicate-video-mixer"));

        let first_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(
            result_processor_id(&first_response),
            "duplicate-video-mixer"
        );

        let second_response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        assert_eq!(error_code(&second_response), crate::jsonrpc::INVALID_PARAMS);

        drop(handle);
        pipeline_task.abort();
        let _ = pipeline_task.await;
    }

    #[tokio::test]
    async fn list_processors_returns_empty_array_when_no_processors() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"listProcessors"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert!(result_processor_ids(&response).is_empty());

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_processors_returns_registered_processors() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let processor_a = handle
            .register_processor(ProcessorId::new("list-processor-a"))
            .await
            .expect("register list-processor-a");
        let processor_b = handle
            .register_processor(ProcessorId::new("list-processor-b"))
            .await
            .expect("register list-processor-b");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"listProcessors"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        let processor_ids = result_processor_ids(&response);

        assert!(processor_ids.contains(&"list-processor-a".to_owned()));
        assert!(processor_ids.contains(&"list-processor-b".to_owned()));

        drop(processor_a);
        drop(processor_b);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_tracks_returns_empty_array_when_no_tracks() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"listTracks"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");

        assert!(result_track_ids(&response).is_empty());

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_tracks_returns_created_tracks() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let publisher = handle
            .register_processor(ProcessorId::new("list-tracks-publisher"))
            .await
            .expect("register list-tracks-publisher");
        publisher
            .publish_track(TrackId::new("list-track-a"))
            .await
            .expect("publish list-track-a");
        publisher
            .publish_track(TrackId::new("list-track-b"))
            .await
            .expect("publish list-track-b");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"listTracks"}"#;

        let response = handle
            .rpc(request.as_bytes())
            .await
            .expect("response must exist");
        let track_ids = result_track_ids(&response);

        assert!(track_ids.contains(&"list-track-a".to_owned()));
        assert!(track_ids.contains(&"list-track-b".to_owned()));

        drop(publisher);
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    #[tokio::test]
    async fn list_rpcs_ignore_params() {
        let (handle, pipeline_task) = spawn_test_pipeline();
        let list_tracks_request =
            r#"{"jsonrpc":"2.0","id":1,"method":"listTracks","params":{"dummy":1}}"#;
        let list_processors_request =
            r#"{"jsonrpc":"2.0","id":2,"method":"listProcessors","params":["dummy"]}"#;

        let tracks_response = handle
            .rpc(list_tracks_request.as_bytes())
            .await
            .expect("response must exist");
        let processors_response = handle
            .rpc(list_processors_request.as_bytes())
            .await
            .expect("response must exist");

        assert!(tracks_response.value().to_member("result").is_ok());
        assert!(processors_response.value().to_member("result").is_ok());

        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), pipeline_task)
            .await
            .expect("pipeline task timed out")
            .expect("pipeline task failed");
    }

    fn spawn_test_pipeline() -> (MediaPipelineHandle, tokio::task::JoinHandle<()>) {
        let pipeline = MediaPipeline::new();
        let handle = pipeline.handle();
        let pipeline_task = tokio::spawn(pipeline.run());
        (handle, pipeline_task)
    }

    fn error_code(response: &nojson::RawJsonOwned) -> i32 {
        response
            .value()
            .to_member("error")
            .expect("error member")
            .required()
            .expect("error value")
            .to_member("code")
            .expect("error.code member")
            .required()
            .expect("error.code value")
            .try_into()
            .expect("error.code must be i32")
    }

    fn result_processor_id(response: &nojson::RawJsonOwned) -> String {
        response
            .value()
            .to_member("result")
            .expect("result member")
            .required()
            .expect("result value")
            .to_member("processorId")
            .expect("result.processorId member")
            .required()
            .expect("result.processorId value")
            .try_into()
            .expect("result.processorId must be string")
    }

    fn result_track_ids(response: &nojson::RawJsonOwned) -> Vec<String> {
        response
            .value()
            .to_member("result")
            .expect("result member")
            .required()
            .expect("result value")
            .to_array()
            .expect("result must be array")
            .map(|v| {
                v.to_member("trackId")
                    .expect("trackId member")
                    .required()
                    .expect("trackId value")
                    .try_into()
                    .expect("trackId must be string")
            })
            .collect()
    }

    fn result_processor_ids(response: &nojson::RawJsonOwned) -> Vec<String> {
        response
            .value()
            .to_member("result")
            .expect("result member")
            .required()
            .expect("result value")
            .to_array()
            .expect("result must be array")
            .map(|v| {
                v.to_member("processorId")
                    .expect("processorId member")
                    .required()
                    .expect("processorId value")
                    .try_into()
                    .expect("processorId must be string")
            })
            .collect()
    }

    fn create_video_mixer_request(output_track_id: &str, processor_id: Option<&str>) -> String {
        let processor_id_part = processor_id
            .map(|id| format!(r#","processorId":"{id}""#))
            .unwrap_or_default();

        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"createVideoMixer","params":{{"canvasWidth":640,"canvasHeight":480,"frameRate":30,"inputTracks":[{{"trackId":"video-input-track","x":0,"y":0,"z":0}}],"outputTrackId":"{output_track_id}"{processor_id_part}}}}}"#
        )
    }
}
