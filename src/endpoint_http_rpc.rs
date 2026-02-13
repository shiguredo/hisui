use shiguredo_http11::{Request, Response};

pub async fn handle_request(
    request: &Request,
    pipeline_handle: &crate::MediaPipelineHandle,
) -> Response {
    if request.method != "POST" {
        let mut response = Response::new(405, "Method Not Allowed");
        response.add_header("Allow", "POST");
        return response;
    }

    match pipeline_handle.rpc(&request.body).await {
        Some(response_json) => {
            let mut response = Response::new(200, "OK");
            response.add_header("Content-Type", "application/json");
            response.body = response_json.to_string().into_bytes();
            response
        }
        None => Response::new(204, "No Content"),
    }
}
