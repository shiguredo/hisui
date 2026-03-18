use shiguredo_websocket::CloseCode;

pub const OBSWS_SUBPROTOCOL: &str = "obswebsocket.json";
pub const OBSWS_VERSION: &str = "5.7.2";
pub const OBSWS_RPC_VERSION: u32 = 1;
/// 互換性のある OBS Studio のバージョン。
/// OBS WebSocket 5.7.2 は OBS Studio 31.0.2 に対応する。
pub const OBS_STUDIO_VERSION: &str = "31.0.2";
pub const OBSWS_DEFAULT_SCENE_NAME: &str = "Scene";
pub const OBSWS_OP_HELLO: i64 = 0;
pub const OBSWS_OP_IDENTIFY: i64 = 1;
pub const OBSWS_OP_IDENTIFIED: i64 = 2;
pub const OBSWS_OP_REIDENTIFY: i64 = 3;
pub const OBSWS_OP_EVENT: i64 = 5;
pub const OBSWS_OP_REQUEST: i64 = 6;
pub const OBSWS_OP_REQUEST_RESPONSE: i64 = 7;
pub const OBSWS_OP_REQUEST_BATCH: i64 = 8;
pub const OBSWS_OP_REQUEST_BATCH_RESPONSE: i64 = 9;

pub const OBSWS_EVENT_SUB_GENERAL: u32 = 1 << 0;
pub const OBSWS_EVENT_SUB_SCENES: u32 = 1 << 2;
pub const OBSWS_EVENT_SUB_INPUTS: u32 = 1 << 3;
pub const OBSWS_EVENT_SUB_OUTPUTS: u32 = 1 << 6;
pub const OBSWS_EVENT_SUB_SCENE_ITEMS: u32 = 1 << 7;

/// OBS WebSocket プロトコルにおける EventSubscription::All のデフォルト値。
/// Identify の eventSubscriptions が省略された場合に使用する。
/// InputVolumeMeters (1 << 16) と InputActiveStateChanged (1 << 17) は除外されている。
pub const OBSWS_EVENT_SUB_ALL: u32 = (1 << 10) - 1;

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
pub const REQUEST_STATUS_REQUEST_PROCESSING_FAILED: i64 = 205;
pub const REQUEST_STATUS_MISSING_REQUEST_FIELD: i64 = 300;
pub const REQUEST_STATUS_MISSING_REQUEST_DATA: i64 = 301;
pub const REQUEST_STATUS_INVALID_REQUEST_FIELD: i64 = 400;
pub const REQUEST_STATUS_OUTPUT_RUNNING: i64 = 500;
pub const REQUEST_STATUS_OUTPUT_NOT_RUNNING: i64 = 501;
pub const REQUEST_STATUS_STREAM_RUNNING: i64 = 502;
pub const REQUEST_STATUS_STREAM_NOT_RUNNING: i64 = 503;
pub const REQUEST_STATUS_STUDIO_MODE_NOT_ACTIVE: i64 = 506;
pub const REQUEST_STATUS_RESOURCE_NOT_FOUND: i64 = 601;
pub const REQUEST_STATUS_RESOURCE_ALREADY_EXISTS: i64 = 602;
