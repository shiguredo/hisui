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
pub struct JsonRpcRequest(nojson::RawJsonOwned);
