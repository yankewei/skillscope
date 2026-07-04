use crate::api::{
    DoctorResponse, ErrorResponse, InvocationTypeStatsResponse, ScanRequest, ScanResponse,
    SkillStatsResponse,
};
use crate::error::{Result, SkillScopeError};
use std::time::Duration;

pub struct ServiceClient {
    base_url: String,
    agent: ureq::Agent,
}

impl ServiceClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout_read(Duration::from_secs(30))
                .timeout_write(Duration::from_secs(30))
                .build(),
        }
    }

    pub fn scan(&self, request: &ScanRequest) -> Result<ScanResponse> {
        self.post_json("/scan", request)
    }

    pub fn skill_stats(&self, since: Option<&str>) -> Result<SkillStatsResponse> {
        self.get_json(&path_with_since("/stats/skills", since))
    }

    pub fn invocation_type_stats(
        &self,
        since: Option<&str>,
    ) -> Result<InvocationTypeStatsResponse> {
        self.get_json(&path_with_since("/stats/invocation-types", since))
    }

    pub fn doctor(&self) -> Result<DoctorResponse> {
        self.get_json("/doctor")
    }

    pub fn health(&self) -> Result<()> {
        let _: serde_json::Value = self.get_json("/health")?;
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        let _: serde_json::Value = self.post_json("/shutdown", &serde_json::json!({}))?;
        Ok(())
    }

    fn get_json<T>(&self, path: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let body = self.request_get(path)?;
        serde_json::from_str(&body).map_err(Into::into)
    }

    fn post_json<T, U>(&self, path: &str, payload: &T) -> Result<U>
    where
        T: serde::Serialize,
        U: serde::de::DeserializeOwned,
    {
        let body = self.request_post(path, payload)?;
        serde_json::from_str(&body).map_err(Into::into)
    }

    fn request_get(&self, path: &str) -> Result<String> {
        let url = self.url(path)?;
        let response = self.agent.get(&url).call();
        response_body(response, &self.base_url)
    }

    fn request_post<T>(&self, path: &str, payload: &T) -> Result<String>
    where
        T: serde::Serialize,
    {
        let url = self.url(path)?;
        let value = serde_json::to_value(payload)?;
        let response = self.agent.post(&url).send_json(value);
        response_body(response, &self.base_url)
    }

    fn url(&self, path: &str) -> Result<String> {
        if !self.base_url.starts_with("http://") {
            return Err(SkillScopeError::Service(
                "service URL must start with http://".to_string(),
            ));
        }
        Ok(format!("{}{}", self.base_url, path))
    }
}

fn path_with_since(path: &str, since: Option<&str>) -> String {
    match since {
        Some(since) => format!("{path}?since={}", percent_encode(since)),
        None => path.to_string(),
    }
}

fn response_body(
    response: std::result::Result<ureq::Response, ureq::Error>,
    base_url: &str,
) -> Result<String> {
    match response {
        Ok(response) => response.into_string().map_err(Into::into),
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<ErrorResponse>(&body)
                .map(|response| response.error)
                .unwrap_or(body);
            Err(SkillScopeError::Service(format!(
                "service returned HTTP {status}: {message}"
            )))
        }
        Err(err) => Err(SkillScopeError::ServiceUnavailable(format!(
            "could not connect to {base_url}; run `skillscope daemon start` first ({err})"
        ))),
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn rejects_non_http_service_url() {
        let client = ServiceClient::new("https://127.0.0.1:3766".to_string());

        let err = client.url("/health").unwrap_err();

        assert!(err.to_string().contains("service URL"));
    }

    #[test]
    fn percent_encodes_since_query() {
        let path = path_with_since("/stats/skills", Some("2026-07-02T00:00:00+08:00"));

        assert_eq!(
            path,
            "/stats/skills?since=2026-07-02T00%3A00%3A00%2B08%3A00"
        );
    }

    #[test]
    fn preserves_json_error_message_from_http_service() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            let body = r#"{"error":"database down"}"#;
            write!(
                stream,
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let client = ServiceClient::new(format!("http://{addr}"));
        let err = client.health().unwrap_err();

        server.join().unwrap();
        assert!(err.to_string().contains("database down"));
    }
}
