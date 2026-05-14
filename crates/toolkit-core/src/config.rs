use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub vllm_base_url: String,
    pub vllm_api_key: String,
    pub batch_size:    usize,
    pub connect_timeout_secs: u64,
    pub vllm_timeout_secs: u64,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            vllm_base_url: env::var("VLLM_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:30000/v1".to_string()),
            vllm_api_key: env::var("VLLM_API_KEY")
                .unwrap_or_else(|_| "none".to_string()),
            batch_size: env::var("BATCH_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            connect_timeout_secs: 5,
            vllm_timeout_secs: env::var("VLLM_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120),
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
            std::env::remove_var("BATCH_SIZE");
        }
        let cfg = Config::from_env();
        assert_eq!(cfg.vllm_base_url, "http://localhost:30000/v1");
        assert_eq!(cfg.batch_size, 4);
    }

    #[test]
    #[serial]
    fn reads_env_vars() {
        unsafe {
            std::env::set_var("VLLM_BASE_URL", "http://custom:9000/v1");
            std::env::set_var("BATCH_SIZE", "8");
        }
        let cfg = Config::from_env();
        assert_eq!(cfg.vllm_base_url, "http://custom:9000/v1");
        assert_eq!(cfg.batch_size, 8);
        unsafe {
            std::env::remove_var("VLLM_BASE_URL");
            std::env::remove_var("BATCH_SIZE");
        }
    }

    #[test]
    fn models_url_strips_v1_suffix() {
        let mut cfg = Config::from_env();
        cfg.vllm_base_url = "http://host:30000/v1".to_string();
        assert_eq!(cfg.models_url(), "http://host:30000/v1/models");
    }
}
