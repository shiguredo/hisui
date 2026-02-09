pub async fn recv_or_pending(rx: &mut Option<crate::MessageReceiver>) -> crate::Message {
    if let Some(rx) = rx {
        rx.recv().await
    } else {
        std::future::pending().await
    }
}
