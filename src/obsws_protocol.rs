use shiguredo_websocket::CloseCode;

pub const OBSWS_SUBPROTOCOL: &str = "obswebsocket.json";
pub const OBSWS_VERSION: &str = "5.0.0";
pub const OBSWS_RPC_VERSION: u32 = 1;
pub const OBSWS_DEFAULT_SCENE_NAME: &str = "Scene";
pub const OBSWS_OP_HELLO: i64 = 0;
pub const OBSWS_OP_IDENTIFY: i64 = 1;
pub const OBSWS_OP_IDENTIFIED: i64 = 2;
pub const OBSWS_OP_REQUEST: i64 = 6;
pub const OBSWS_OP_REQUEST_RESPONSE: i64 = 7;

pub const OBSWS_CLOSE_UNSUPPORTED_RPC_VERSION: CloseCode = CloseCode(4006);
pub const OBSWS_CLOSE_NOT_IDENTIFIED: CloseCode = CloseCode(4007);
pub const OBSWS_CLOSE_ALREADY_IDENTIFIED: CloseCode = CloseCode(4008);
pub const OBSWS_CLOSE_AUTHENTICATION_FAILED: CloseCode = CloseCode(4009);

pub const OBSWS_SUPPORTED_IMAGE_FORMATS: [&str; 9] = [
    "bmp", "cur", "heic", "jpeg", "jpg", "jxl", "png", "tga", "webp",
];

pub const AUTH_RANDOM_BYTE_LEN: usize = 32;
pub const REQUEST_STATUS_SUCCESS: i64 = 100;
pub const REQUEST_STATUS_MISSING_REQUEST_TYPE: i64 = 203;
pub const REQUEST_STATUS_UNKNOWN_REQUEST_TYPE: i64 = 204;
pub const REQUEST_STATUS_MISSING_REQUEST_FIELD: i64 = 300;
pub const REQUEST_STATUS_INVALID_REQUEST_FIELD: i64 = 400;
pub const REQUEST_STATUS_STREAM_RUNNING: i64 = 502;
pub const REQUEST_STATUS_STREAM_NOT_RUNNING: i64 = 503;
pub const REQUEST_STATUS_RESOURCE_NOT_FOUND: i64 = 601;
pub const REQUEST_STATUS_RESOURCE_ALREADY_EXISTS: i64 = 602;
