use adapter_core::{LoadoutSnapshot, ProviderId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CcSwitchBridge {
    endpoint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CcSwitchHealth {
    pub ok: bool,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CcSwitchStatus {
    pub provider: ProviderId,
    #[serde(default)]
    pub model: Option<String>,
    pub config_fingerprint: String,
    pub hooks_fingerprint: String,
    pub skills_fingerprint: String,
    pub plugins_fingerprint: String,
    pub mcp_fingerprint: String,
}

impl CcSwitchStatus {
    pub fn loadout(&self) -> LoadoutSnapshot {
        LoadoutSnapshot {
            provider: self.provider,
            model: self.model.clone(),
            config_fingerprint: self.config_fingerprint.clone(),
            hooks_fingerprint: self.hooks_fingerprint.clone(),
            skills_fingerprint: self.skills_fingerprint.clone(),
            plugins_fingerprint: self.plugins_fingerprint.clone(),
            mcp_fingerprint: self.mcp_fingerprint.clone(),
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum BridgeError {
    #[error("CC Switch endpoint must use http://")]
    UnsupportedScheme,
    #[error("CC Switch endpoint must be loopback")]
    NonLoopback,
    #[error("CC Switch endpoint must include an explicit port")]
    MissingPort,
    #[error("CC Switch endpoint path is not allowlisted")]
    PathNotAllowed,
    #[error("CC Switch endpoint is malformed")]
    MalformedEndpoint,
    #[error("CC Switch request timed out")]
    Timeout,
    #[error("CC Switch connection failed: {0}")]
    Io(String),
    #[error("CC Switch returned HTTP status {0}")]
    HttpStatus(u16),
    #[error("CC Switch redirected instead of returning JSON")]
    Redirect,
    #[error("CC Switch response is too large")]
    ResponseTooLarge,
    #[error("CC Switch response must be JSON")]
    NotJson,
    #[error("CC Switch response has an invalid JSON schema: {0}")]
    Schema(String),
}

impl CcSwitchBridge {
    pub fn new(endpoint: impl Into<String>) -> Result<Self, BridgeError> {
        let endpoint = endpoint.into();
        let parsed = parse_endpoint(&endpoint)?;
        if !parsed.ip.is_loopback() {
            return Err(BridgeError::NonLoopback);
        }
        Ok(Self { endpoint })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn health(&self) -> Result<CcSwitchHealth, BridgeError> {
        let bytes = self.request("/health").await?;
        parse_json(&bytes)
    }

    pub async fn status(&self) -> Result<CcSwitchStatus, BridgeError> {
        let bytes = self.request("/status").await?;
        parse_json(&bytes)
    }

    async fn request(&self, path: &str) -> Result<Vec<u8>, BridgeError> {
        let endpoint = parse_endpoint(&self.endpoint)?;
        if path != "/health" && path != "/status" {
            return Err(BridgeError::PathNotAllowed);
        }
        let operation = async {
            let mut stream = TcpStream::connect(endpoint.socket_addr())
                .await
                .map_err(|error| BridgeError::Io(error.to_string()))?;
            let host = endpoint.host_header();
            let request = format!(
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|error| BridgeError::Io(error.to_string()))?;
            let mut response = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                let read = stream
                    .read(&mut chunk)
                    .await
                    .map_err(|error| BridgeError::Io(error.to_string()))?;
                if read == 0 {
                    break;
                }
                if response.len() + read > MAX_RESPONSE_BYTES {
                    return Err(BridgeError::ResponseTooLarge);
                }
                response.extend_from_slice(&chunk[..read]);
            }
            parse_http_response(&response)
        };
        tokio::time::timeout(REQUEST_TIMEOUT, operation)
            .await
            .map_err(|_| BridgeError::Timeout)?
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedEndpoint {
    ip: IpAddr,
    port: u16,
    path: String,
}

impl ParsedEndpoint {
    fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }

    fn host_header(&self) -> String {
        match self.ip {
            IpAddr::V6(_) => format!("[{}]:{}", self.ip, self.port),
            IpAddr::V4(_) => format!("{}:{}", self.ip, self.port),
        }
    }
}

fn parse_endpoint(endpoint: &str) -> Result<ParsedEndpoint, BridgeError> {
    let rest = endpoint
        .strip_prefix("http://")
        .ok_or(BridgeError::UnsupportedScheme)?;
    if rest.contains('?')
        || rest.contains('#')
        || rest.contains('/') && !rest.contains("/health") && !rest.contains("/status")
    {
        return Err(BridgeError::PathNotAllowed);
    }
    let (authority, path) = rest.split_once('/').ok_or(BridgeError::PathNotAllowed)?;
    if path != "health" && path != "status" {
        return Err(BridgeError::PathNotAllowed);
    }
    let path = format!("/{path}");
    let (host, port) = if let Some(value) = authority.strip_prefix('[') {
        let (host, port) = value.split_once("]:").ok_or(BridgeError::MissingPort)?;
        (host, port)
    } else {
        authority.rsplit_once(':').ok_or(BridgeError::MissingPort)?
    };
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return Err(BridgeError::NonLoopback);
    }
    let ip = host
        .parse::<IpAddr>()
        .map_err(|_| BridgeError::MalformedEndpoint)?;
    let port = port
        .parse::<u16>()
        .map_err(|_| BridgeError::MalformedEndpoint)?;
    Ok(ParsedEndpoint { ip, port, path })
}

fn parse_http_response(response: &[u8]) -> Result<Vec<u8>, BridgeError> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| BridgeError::Schema("missing HTTP headers".to_owned()))?;
    let header = std::str::from_utf8(&response[..header_end])
        .map_err(|_| BridgeError::Schema("invalid HTTP headers".to_owned()))?;
    let mut lines = header.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| BridgeError::Schema("missing HTTP status".to_owned()))?;
    if (300..400).contains(&status) {
        return Err(BridgeError::Redirect);
    }
    if status != 200 {
        return Err(BridgeError::HttpStatus(status));
    }
    let mut json_content = false;
    let mut content_length = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-type") {
                json_content = value
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("application/json");
            }
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| BridgeError::Schema("invalid content length".to_owned()))?,
                );
            }
        }
    }
    if !json_content {
        return Err(BridgeError::NotJson);
    }
    let body = &response[header_end + 4..];
    if content_length.is_some_and(|length| length > MAX_RESPONSE_BYTES || length > body.len()) {
        return Err(BridgeError::ResponseTooLarge);
    }
    Ok(body.to_vec())
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, BridgeError> {
    serde_json::from_slice::<Value>(body)
        .map_err(|error| BridgeError::Schema(error.to_string()))
        .and_then(|value| {
            serde_json::from_value(value).map_err(|error| BridgeError::Schema(error.to_string()))
        })
}

#[cfg(test)]
mod tests {
    use super::{BridgeError, CcSwitchBridge, MAX_RESPONSE_BYTES};
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    async fn server(response: impl Into<String>) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listener");
        let address = listener.local_addr().expect("address");
        let response = response.into();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0u8; 512];
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(1), stream.readable()).await;
            let _ = stream.try_read(&mut request);
            stream
                .write_all(response.as_bytes())
                .await
                .expect("response");
        });
        format!("http://{address}/status")
    }

    #[tokio::test]
    async fn accepts_loopback_json_status_and_rejects_unknown_schema() {
        let endpoint = server("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"provider\":\"claude\",\"model\":null,\"config_fingerprint\":\"c\",\"hooks_fingerprint\":\"h\",\"skills_fingerprint\":\"s\",\"plugins_fingerprint\":\"p\",\"mcp_fingerprint\":\"m\"}").await;
        let bridge = CcSwitchBridge::new(endpoint).expect("bridge");
        assert_eq!(
            bridge.status().await.expect("status").provider.to_string(),
            "claude"
        );

        let unknown_endpoint = server("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"provider\":\"claude\",\"unexpected\":true}").await;
        let unknown = CcSwitchBridge::new(unknown_endpoint).expect("bridge");
        assert!(matches!(
            unknown.status().await,
            Err(BridgeError::Schema(_))
        ));
    }

    #[tokio::test]
    async fn health_uses_the_same_allowlisted_loopback_origin() {
        let endpoint = server(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"ok\":true,\"version\":\"3.19.2\"}",
        )
        .await;
        let bridge = CcSwitchBridge::new(endpoint).expect("bridge");
        let health = bridge.health().await.expect("health");
        assert!(health.ok);
        assert_eq!(health.version.as_deref(), Some("3.19.2"));
    }

    #[test]
    fn rejects_non_loopback_missing_port_and_non_allowlisted_paths() {
        assert_eq!(
            CcSwitchBridge::new("http://192.168.1.1:3210/status"),
            Err(BridgeError::NonLoopback)
        );
        assert_eq!(
            CcSwitchBridge::new("https://127.0.0.1:3210/status"),
            Err(BridgeError::UnsupportedScheme)
        );
        assert_eq!(
            CcSwitchBridge::new("http://127.0.0.1:3210/settings"),
            Err(BridgeError::PathNotAllowed)
        );
        assert_eq!(
            CcSwitchBridge::new("http://127.0.0.1/status"),
            Err(BridgeError::MissingPort)
        );
    }

    #[tokio::test]
    async fn rejects_redirects_non_json_and_oversized_responses() {
        let redirect = CcSwitchBridge::new(
            server("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/status\r\nConnection: close\r\n\r\n").await,
        )
        .expect("bridge");
        assert_eq!(redirect.status().await, Err(BridgeError::Redirect));

        let non_json = CcSwitchBridge::new(
            server("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}")
                .await,
        )
        .expect("bridge");
        assert_eq!(non_json.status().await, Err(BridgeError::NotJson));

        let body = "x".repeat(MAX_RESPONSE_BYTES);
        let oversized = CcSwitchBridge::new(server(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        )).await).expect("bridge");
        assert_eq!(oversized.status().await, Err(BridgeError::ResponseTooLarge));
    }

    #[tokio::test]
    async fn times_out_when_loopback_server_stalls() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listener");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        });
        let bridge = CcSwitchBridge::new(format!("http://{address}/status")).expect("bridge");
        assert_eq!(bridge.status().await, Err(BridgeError::Timeout));
    }
}
