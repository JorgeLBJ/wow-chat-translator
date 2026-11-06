//! Minimal blocking HTTP seam so translators can be tested with fakes.

use std::time::Duration;

/// Status code and body text of an HTTP answer (any status, errors included).
#[derive(Clone, Debug, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

impl HttpResponse {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// `Err` only for transport failures (DNS, connection, timeout); HTTP error
/// statuses come back as `Ok` with their status.
pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str, headers: &[(String, String)], timeout: Duration) -> Result<HttpResponse, String>;
    fn post(&self, url: &str, headers: &[(String, String)], body: &str, timeout: Duration) -> Result<HttpResponse, String>;
}

/// Real client: ureq over rustls.
pub struct UreqClient {
    agent: ureq::Agent,
}

impl UreqClient {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder().http_status_as_error(false).build();
        Self { agent: config.into() }
    }
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

fn read(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<HttpResponse, String> {
    let mut res = result.map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    let body = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(HttpResponse { status, body })
}

impl HttpClient for UreqClient {
    fn get(&self, url: &str, headers: &[(String, String)], timeout: Duration) -> Result<HttpResponse, String> {
        let mut req = self.agent.get(url).config().timeout_global(Some(timeout)).build();
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        read(req.call())
    }

    fn post(&self, url: &str, headers: &[(String, String)], body: &str, timeout: Duration) -> Result<HttpResponse, String> {
        let mut req = self.agent.post(url).config().timeout_global(Some(timeout)).build();
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        read(req.send(body))
    }
}

/// Percent-encodes a query component (everything but RFC 3986 unreserved characters).
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// First `max` characters of an error body, for error messages.
pub fn snippet(body: &str, max: usize) -> String {
    body.chars().take(max).collect()
}
