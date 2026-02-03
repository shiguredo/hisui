#[derive(Debug)]
pub struct ProcessorManager {
    // [NOTE]
    // 今は一つだけだが、将来的には特殊用途向け（e.g., CPU ヘビーなエンコード処理など）に
    // 専用のランタイムを追加する可能性があるため、それを意識した名前となっている
    default_runtime_handle: tokio::runtime::Handle,
}

impl ProcessorManager {
    pub fn new(default_runtime_handle: tokio::runtime::Handle) -> Self {
        Self {
            default_runtime_handle,
        }
    }

    pub fn start(self) -> ProcessorManagerHandle {
        let handle = self.default_runtime_handle.clone();

        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        let runner = ProcessorManagerRunner { command_rx };
        handle.spawn(runner.run());

        ProcessorManagerHandle { command_tx }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessorManagerHandle {
    command_tx: tokio::sync::mpsc::UnboundedSender<Command>,
}

impl ProcessorManagerHandle {
    pub fn spawn_processor<F>(&self, processor_id: ProcessorId, future: F)
    where
        F: Future<Output = Result<(), ProcessorError>> + Send + 'static,
    {
        let command = Command::SpawnProcessor {
            processor_id,
            future: ProcessorFuture(Box::new(future)),
        };
        self.send(command);
    }

    fn send(&self, command: Command) {
        let _ = self.command_tx.send(command);
    }
}

#[derive(Debug)]
struct ProcessorManagerRunner {
    command_rx: tokio::sync::mpsc::UnboundedReceiver<Command>,
}

impl ProcessorManagerRunner {
    async fn run(mut self) {
        while let Some(command) = self.command_rx.recv().await {
            match command {
                Command::SpawnProcessor {
                    processor_id,
                    future,
                } => self.handle_spawn_processor(processor_id, future),
            }
        }

        // 全てのハンドルがいなくなったらここに来る（これ以上何もできないので終了するだけ）
        log::info!("processor manager finished");
    }

    fn handle_spawn_processor(&mut self, _processor_id: ProcessorId, _future: ProcessorFuture) {
        todo!()
    }
}

#[derive(Debug)]
enum Command {
    SpawnProcessor {
        processor_id: ProcessorId,
        future: ProcessorFuture,
    },
}

pub struct ProcessorFuture(Box<dyn Future<Output = Result<(), ProcessorError>> + Send + 'static>);

impl std::fmt::Debug for ProcessorFuture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessorFuture").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct ProcessorError;

#[derive(Debug)]
pub struct JsonRpcRequest(pub nojson::RawJsonOwned);

#[derive(Debug)]
pub struct ProcessorId;

#[derive(Debug)]
pub struct TrackId;

#[derive(Debug)]
pub struct ChannelRegistry {}

impl ChannelRegistry {
    pub fn register(&mut self, _pid: ProcessorId) -> Option<(ChannelSender, ChannelReceiver)> {
        todo!()
    }

    // list_tracks(), list_processors()
}

#[derive(Debug)]
pub struct TrackInfo {}

#[derive(Debug)]
pub struct ChannelSender {}

impl ChannelSender {
    pub fn publish_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId, _info: TrackInfo) {}

    pub fn unpublish_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId) {}

    pub fn send_output(&mut self, _tid: TrackId, _frame: ()) {}

    pub fn send_feedback(&mut self, _tid: TrackId, _feedback: ()) {}
}

#[derive(Debug)]
pub struct SubscribeOptions {
    pub channel_size: usize,
}

#[derive(Debug)]
pub struct ChannelReceiver {}

impl ChannelReceiver {
    pub fn subscribe_track(
        &mut self,
        _reg: &ChannelRegistry,
        _tid: TrackId,
        _options: SubscribeOptions,
    ) {
    }

    pub fn subscribe_processor(
        &mut self,
        _reg: &ChannelRegistry,
        _pid: ProcessorId,
        _options: SubscribeOptions,
    ) {
    }

    pub fn unsubscribe_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId) {}

    pub fn unsubscribe_processor(&mut self, _reg: &ChannelRegistry, _pid: ProcessorId) {}

    pub fn recv(&mut self) -> Recv {
        todo!()
    }
}

#[derive(Debug)]
pub enum Recv {
    MediaFrame {
        pid: ProcessorId,
        tid: TrackId,
        data: (),
    },
    Feedback {
        pid: ProcessorId,
        tid: TrackId,
        data: Feedback,
    },
    Rpc {
        from: (),
        data: (),
    },
}

#[derive(Debug)]
pub enum Feedback {
    KeyFrameRequired,
}
