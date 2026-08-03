//! OBS WebSocket 5.x プロトコル (DataChannel 経由) の定義。

mod auth;
mod protocol;

pub use auth::generate_authentication_string;
pub use protocol::{
    AuthenticationChallenge, ClientMessage, EventData, EventSubscription, HelloData,
    IdentifiedData, IdentifyData, OpCode, ProtocolError, RequestData, RequestResponseData,
    RequestStatus, ServerMessage, parse_server_message, serialize_client_message,
};
