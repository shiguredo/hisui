use shiguredo_http11::{Request, Response};

use crate::webrtc::p2p_session::{BootstrapError, WebRtcP2pSessionManager};

fn build_error_response(status: u16, reason: &str) -> Response {
    Response::new(status, reason)
        .header("Content-Type", "text/plain")
        .header("Connection", "close")
        .body(reason.as_bytes().to_vec())
}

fn build_sdp_response(status: u16, reason: &str, sdp: &str) -> Response {
    Response::new(status, reason)
        .header("Content-Type", "application/sdp")
        .header("Connection", "close")
        .body(sdp.as_bytes().to_vec())
}

pub struct BootstrapEndpoint {
    session_manager: WebRtcP2pSessionManager,
}

impl BootstrapEndpoint {
    pub fn new(handle: crate::MediaPipelineHandle) -> crate::Result<Self> {
        Ok(Self {
            session_manager: WebRtcP2pSessionManager::new(handle)?,
        })
    }

    pub async fn handle_request(&self, request: &Request) -> Response {
        if request.uri.as_str() != "/bootstrap" {
            return build_error_response(404, "Not Found");
        }

        if request.method != "POST" {
            return build_error_response(405, "Method Not Allowed");
        }

        let content_type = request
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        if !content_type.starts_with("application/sdp") {
            return build_error_response(415, "Unsupported Media Type");
        }

        if request.body.is_empty() {
            return build_error_response(400, "Bad Request");
        }

        let offer_sdp = String::from_utf8_lossy(&request.body).to_string();
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
