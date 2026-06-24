use shiguredo_http11::{Request, Response};

use crate::webrtc::p2p_session::{BootstrapError, WebRtcP2pSessionManager};

fn build_error_response(status: u16, reason: &str) -> Response {
    // 固定の status / reason / ヘッダー名・値で構築するので shiguredo_http11 の
    // バリデーションは通る想定。失敗した場合は実装バグ。
    let response = Response::new(status, reason).expect("infallible: fixed status/reason");
    let response = response
        .header("Content-Type", "text/plain")
        .expect("infallible: fixed header");
    let response = response
        .header("Connection", "close")
        .expect("infallible: fixed header");
    response.body(reason.as_bytes().to_vec())
}

fn build_sdp_response(status: u16, reason: &str, sdp: &str) -> Response {
    // 固定の status / reason / ヘッダー名・値で構築するので shiguredo_http11 の
    // バリデーションは通る想定。失敗した場合は実装バグ。
    let response = Response::new(status, reason).expect("infallible: fixed status/reason");
    let response = response
        .header("Content-Type", "application/sdp")
        .expect("infallible: fixed header");
    let response = response
        .header("Connection", "close")
        .expect("infallible: fixed header");
    response.body(sdp.as_bytes().to_vec())
}

pub struct BootstrapEndpoint {
    session_manager: WebRtcP2pSessionManager,
}

impl BootstrapEndpoint {
    pub fn new(
        handle: crate::MediaPipelineHandle,
        coordinator_handle: crate::obsws::coordinator::ObswsCoordinatorHandle,
    ) -> crate::Result<Self> {
        Ok(Self {
            session_manager: WebRtcP2pSessionManager::new(handle, coordinator_handle)?,
        })
    }

    pub async fn handle_request(&self, request: &Request) -> Response {
        if request.uri() != "/bootstrap" {
            return build_error_response(404, "Not Found");
        }

        if request.method() != "POST" {
            return build_error_response(405, "Method Not Allowed");
        }

        let content_type = request.get_header("content-type").unwrap_or("");
        if !content_type.starts_with("application/sdp") {
            return build_error_response(415, "Unsupported Media Type");
        }

        let body = request.body_bytes().unwrap_or(&[]);
        if body.is_empty() {
            return build_error_response(400, "Bad Request");
        }

        let offer_sdp = String::from_utf8_lossy(body).to_string();
        match self.session_manager.bootstrap(&offer_sdp).await {
            Ok(answer_sdp) => build_sdp_response(201, "Created", &answer_sdp),
            Err(BootstrapError::SessionAlreadyExists) => build_error_response(409, "Conflict"),
            Err(BootstrapError::Internal(e)) => {
                tracing::warn!("Bootstrap error: {}", e.display());
                build_error_response(500, "Internal Server Error")
            }
        }
    }
}
