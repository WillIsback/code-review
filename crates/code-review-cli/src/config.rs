use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub vllm_base_url: String,
    pub vllm_api_key: String,
    pub connect_timeout_secs: u64,
    pub vllm_timeout_secs: u64,
    pub vllm_retries: u32,
    pub review_max_context: usize,
    /// Extra HTTP headers added to every vLLM request (gateway/auth headers, e.g.
    /// Cloudflare Access service tokens). Parsed from `EXTRA_HEADERS`.
    pub extra_headers: Vec<(String, String)>,
}

/// Parse `EXTRA_HEADERS`: one `Name: Value` per line; blank lines and lines
/// starting with `#` are ignored. Lines without `:` are skipped.
pub fn parse_extra_headers(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        })
        .filter(|(name, value)| !name.is_empty() && !value.is_empty())
        .collect()
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            vllm_base_url: env::var("VLLM_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:30000/v1".to_string()),
            vllm_api_key: env::var("VLLM_API_KEY").unwrap_or_else(|_| "none".to_string()),
            connect_timeout_secs: 5,
            vllm_timeout_secs: env::var("VLLM_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120),
            vllm_retries: env::var("VLLM_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2),
            review_max_context: env::var("REVIEW_MAX_CONTEXT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100_000),
            extra_headers: parse_extra_headers(&env::var("EXTRA_HEADERS").unwrap_or_default()),
        }
    }

    /// Build a reusable HTTP client with the configured timeout.
    pub fn http_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.vllm_timeout_secs))
            .build()
            .expect("valid client")
    }

    /// Build a reusable HTTP client with the connect timeout (for health checks).
    pub fn connect_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.connect_timeout_secs))
            .build()
            .expect("valid client")
    }

    /// Normalise base URL and return /v1/models endpoint.
    pub fn models_url(&self) -> String {
        let url = self.vllm_base_url.trim_end_matches('/');
        let url = url.strip_suffix("/v1").unwrap_or(url);
        format!("{url}/v1/models")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn defaults_when_env_absent() {
        unsafe {
            std::env::remove_var("VLLM_BASE_URL");
        }
        let cfg = Config::from_env();
        assert_eq!(cfg.vllm_base_url, "http://localhost:30000/v1");
    }

    #[test]
    #[serial]
    fn reads_env_vars() {
        unsafe {
            std::env::set_var("VLLM_BASE_URL", "http://custom:9000/v1");
        }
        let cfg = Config::from_env();
        assert_eq!(cfg.vllm_base_url, "http://custom:9000/v1");
        unsafe {
            std::env::remove_var("VLLM_BASE_URL");
        }
    }

    #[test]
    fn models_url_strips_v1_suffix() {
        let mut cfg = Config::from_env();
        cfg.vllm_base_url = "http://host:30000/v1".to_string();
        assert_eq!(cfg.models_url(), "http://host:30000/v1/models");
    }

    #[test]
    fn parse_extra_headers_reads_lines() {
        let raw = "CF-Access-Client-Id: id123\nCF-Access-Client-Secret: secret456\n";
        assert_eq!(
            parse_extra_headers(raw),
            vec![
                ("CF-Access-Client-Id".to_string(), "id123".to_string()),
                (
                    "CF-Access-Client-Secret".to_string(),
                    "secret456".to_string()
                ),
            ]
        );
    }

    #[test]
    fn parse_extra_headers_ignores_blanks_comments_and_junk() {
        let raw = "\n# a comment\nNoColonHere\nX-Api-Key:   k\n  Y : v  \n";
        assert_eq!(
            parse_extra_headers(raw),
            vec![
                ("X-Api-Key".to_string(), "k".to_string()),
                ("Y".to_string(), "v".to_string()),
            ]
        );
    }

    #[test]
    #[serial]
    fn reads_extra_headers_env() {
        unsafe {
            std::env::set_var("EXTRA_HEADERS", "X-Test: yes\n");
        }
        let cfg = Config::from_env();
        assert_eq!(
            cfg.extra_headers,
            vec![("X-Test".to_string(), "yes".to_string())]
        );
        unsafe {
            std::env::remove_var("EXTRA_HEADERS");
        }
    }

    #[test]
    #[serial]
    fn extra_headers_default_empty() {
        unsafe {
            std::env::remove_var("EXTRA_HEADERS");
        }
        let cfg = Config::from_env();
        assert!(cfg.extra_headers.is_empty());
    }
}
