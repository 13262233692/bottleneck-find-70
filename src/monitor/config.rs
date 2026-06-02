use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub jaeger_endpoint: String,
    pub webhook_url: Option<String>,
    pub webhook_headers: Option<std::collections::HashMap<String, String>>,
    pub analysis_interval_secs: u64,
    pub bottleneck_threshold: f64,
    pub shift_detection_threshold: f64,
    pub min_traces_per_analysis: usize,
    pub max_bottleneck_history: usize,
    pub enable_simulated_stream: bool,
    pub simulated_services: usize,
    pub simulated_traces_per_second: f64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            jaeger_endpoint: "http://localhost:16686".to_string(),
            webhook_url: None,
            webhook_headers: None,
            analysis_interval_secs: 10,
            bottleneck_threshold: 0.5,
            shift_detection_threshold: 0.3,
            min_traces_per_analysis: 5,
            max_bottleneck_history: 20,
            enable_simulated_stream: true,
            simulated_services: 8,
            simulated_traces_per_second: 2.0,
        }
    }
}

impl MonitorConfig {
    pub fn analysis_interval(&self) -> Duration {
        Duration::from_secs(self.analysis_interval_secs)
    }

    pub fn with_webhook(mut self, url: String) -> Self {
        self.webhook_url = Some(url);
        self
    }

    pub fn with_jaeger_endpoint(mut self, endpoint: String) -> Self {
        self.jaeger_endpoint = endpoint;
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl std::fmt::Display for AlertSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertSeverity::Info => write!(f, "INFO"),
            AlertSeverity::Warning => write!(f, "WARNING"),
            AlertSeverity::Critical => write!(f, "CRITICAL"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum AlertType {
    NewBottleneck,
    BottleneckShift,
    BottleneckResolved,
    LatencySpike,
    ServiceDegradation,
}

impl std::fmt::Display for AlertType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertType::NewBottleneck => write!(f, "NEW_BOTTLENECK"),
            AlertType::BottleneckShift => write!(f, "BOTTLENECK_SHIFT"),
            AlertType::BottleneckResolved => write!(f, "BOTTLENECK_RESOLVED"),
            AlertType::LatencySpike => write!(f, "LATENCY_SPIKE"),
            AlertType::ServiceDegradation => write!(f, "SERVICE_DEGRADATION"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub alert_type: AlertType,
    pub severity: AlertSeverity,
    pub title: String,
    pub message: String,
    pub service_name: Option<String>,
    pub previous_bottleneck: Option<String>,
    pub new_bottleneck: Option<String>,
    pub score_change: Option<f64>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}

impl Alert {
    pub fn new(alert_type: AlertType, title: String, message: String) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            alert_type,
            severity: AlertSeverity::Warning,
            title,
            message,
            service_name: None,
            previous_bottleneck: None,
            new_bottleneck: None,
            score_change: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    pub fn with_severity(mut self, severity: AlertSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_service(mut self, service: String) -> Self {
        self.service_name = Some(service);
        self
    }

    pub fn with_shift_info(mut self, previous: String, new: String, change: f64) -> Self {
        self.previous_bottleneck = Some(previous);
        self.new_bottleneck = Some(new);
        self.score_change = Some(change);
        self
    }

    pub fn add_metadata(&mut self, key: String, value: serde_json::Value) {
        self.metadata.insert(key, value);
    }
}
