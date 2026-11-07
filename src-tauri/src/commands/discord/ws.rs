use std::time::Duration;

use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::props;

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub async fn connect(url: &str) -> Result<Ws, String> {
    let mut request = url
        .into_client_request()
        .map_err(|e| format!("bad websocket url: {e}"))?;
    let headers = request.headers_mut();
    headers.insert(
        "User-Agent",
        props::user_agent()
            .parse()
            .map_err(|_| "bad user agent".to_string())?,
    );
    headers.insert(
        "Origin",
        props::ORIGIN
            .parse()
            .map_err(|_| "bad origin".to_string())?,
    );
    let (stream, _) = tokio::time::timeout(
        Duration::from_secs(20),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| "network_timeout".to_string())?
    .map_err(|e| format!("network: {e}"))?;
    Ok(stream)
}
