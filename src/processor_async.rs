pub trait AsyncProcessor {
    //
}

#[derive(Debug)]
pub struct AsyncProcessorChannel {}

impl AsyncProcessorChannel {
    /// プロセッサに対する RPC 呼び出しを行う
    pub fn invoke_rpc(&self, _req: JsonRpcRequest) {}

    /// 次のプロセッサ群にメディアフレームを伝える
    pub fn send_frame(&self, _frame: ()) {}

    /// 前のプロセッサにフィードバック情報を伝える
    pub fn send_feedback(&self, _feedback: ()) {}
}

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
}

#[derive(Debug)]
pub struct ChannelSender {}

impl ChannelSender {
    pub fn publish_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId) {}

    pub fn unpublish_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId) {}

    pub fn send_output(&mut self, _tid: TrackId, _frame: ()) {}

    pub fn send_feedback(&mut self, _tid: TrackId, _feedback: ()) {}
}

#[derive(Debug)]
pub struct ChannelReceiver {}

impl ChannelReceiver {
    pub fn subscribe_track(&mut self, _reg: &ChannelRegistry, _tid: TrackId) {}

    pub fn subscribe_processor(&mut self, _reg: &ChannelRegistry, _pid: ProcessorId) {}

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
