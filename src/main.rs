use reqwest::Client;
use serde::{Deserialize, Serialize};
use prometheus::{Encoder, TextEncoder, GaugeVec, Opts, Registry, CounterVec};
use hyper::{Request, Response, StatusCode};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder;
use http_body_util::Full;
use hyper::body::Bytes;
use std::convert::Infallible;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tokio::net::TcpListener;

#[derive(Debug, Serialize, Deserialize)]
struct FundingInfo {
    #[serde(rename = "fundingRate")]
    funding_rate: Option<String>,
    #[serde(rename = "nextFundingTime")]
    next_funding_time: Option<u64>,
}

impl FundingInfo {
    /// Converts the hourly funding rate to an annualized percentage
    fn get_annualized_rate(&self) -> Option<f64> {
        self.funding_rate.as_ref().and_then(|rate_str| {
            rate_str.parse::<f64>().ok().map(|hourly_rate| {
                // Convert hourly rate to annualized rate
                // hourly_rate is already a percentage, so we multiply by 24*365
                hourly_rate * 24.0 * 365.0
            })
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PredictedFundingRequest {
    r#type: String,
}

type PredictedFundingResponse = Vec<(String, Vec<(String, Option<FundingInfo>)>)>;

struct HyperLiquidClient {
    client: Client,
    base_url: String,
}

impl HyperLiquidClient {
    fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: "https://api.hyperliquid.xyz".to_string(),
        }
    }

    async fn get_predicted_fundings(&self) -> Result<PredictedFundingResponse, Box<dyn std::error::Error>> {
        let request_body = PredictedFundingRequest {
            r#type: "predictedFundings".to_string(),
        };

        let response = self
            .client
            .post(&format!("{}/info", self.base_url))
            .json(&request_body)
            .send()
            .await?;

        let funding_data: PredictedFundingResponse = response.json().await?;
        Ok(funding_data)
    }
}

struct MetricsCollector {
    registry: Registry,
    funding_rate_gauge: GaugeVec,
    funding_rate_changes: CounterVec,
    previous_rates: Arc<tokio::sync::Mutex<HashMap<String, f64>>>,
}

impl MetricsCollector {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();
        
        let funding_rate_gauge = GaugeVec::new(
            Opts::new("hyperliquid_funding_rate", "Current funding rate for each coin and venue"),
            &["coin", "venue"]
        )?;
        
        let funding_rate_changes = CounterVec::new(
            Opts::new("hyperliquid_funding_rate_changes_total", "Total number of funding rate changes"),
            &["coin", "venue", "direction"]
        )?;
        
        registry.register(Box::new(funding_rate_gauge.clone()))?;
        registry.register(Box::new(funding_rate_changes.clone()))?;
        
        Ok(Self {
            registry,
            funding_rate_gauge,
            funding_rate_changes,
            previous_rates: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        })
    }
    
    async fn update_metrics(&self, funding_data: &PredictedFundingResponse) {
        let mut previous_rates = self.previous_rates.lock().await;
        
        for (coin, venues) in funding_data {
            for (venue, funding_info) in venues {
                if let Some(funding_info) = funding_info {
                    if let Some(annualized_rate) = funding_info.get_annualized_rate() {
                        let key = format!("{}:{}", coin, venue);
                        
                        self.funding_rate_gauge
                            .with_label_values(&[coin, venue])
                            .set(annualized_rate);
                        
                        if let Some(&previous_rate) = previous_rates.get(&key) {
                            if annualized_rate != previous_rate {
                                let direction = if annualized_rate > previous_rate { "up" } else { "down" };
                                self.funding_rate_changes
                                    .with_label_values(&[coin, venue, direction])
                                    .inc();
                            }
                        }
                        
                        previous_rates.insert(key, annualized_rate);
                    }
                }
            }
        }
    }
    
    fn render_metrics(&self) -> Result<String, Box<dyn std::error::Error>> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8(buffer)?)
    }
}

async fn metrics_handler(
    _req: Request<Incoming>,
    metrics: Arc<MetricsCollector>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    match metrics.render_metrics() {
        Ok(metrics_text) => Ok(Response::new(Full::new(Bytes::from(metrics_text)))),
        Err(_) => Ok(Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from("Error rendering metrics")))
            .unwrap()),
    }
}

async fn start_metrics_server(metrics: Arc<MetricsCollector>) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    println!("Metrics server running on http://0.0.0.0:8080/metrics");
    
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let metrics = metrics.clone();
        
        tokio::task::spawn(async move {
            let service = service_fn(move |req| {
                metrics_handler(req, metrics.clone())
            });
            
            if let Err(err) = Builder::new(hyper_util::rt::TokioExecutor::new())
                .serve_connection(io, service)
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

async fn polling_loop(client: HyperLiquidClient, metrics: Arc<MetricsCollector>) {
    let mut interval = interval(Duration::from_secs(30));
    
    loop {
        interval.tick().await;
        
        match client.get_predicted_fundings().await {
            Ok(funding_data) => {
                metrics.update_metrics(&funding_data).await;
                println!("Updated metrics for {} coins", funding_data.len());
            }
            Err(e) => {
                eprintln!("Error fetching funding data: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = HyperLiquidClient::new();
    let metrics = Arc::new(MetricsCollector::new()?);
    
    let metrics_server = start_metrics_server(metrics.clone());
    let polling_task = polling_loop(client, metrics);
    
    tokio::select! {
        result = metrics_server => {
            if let Err(e) = result {
                eprintln!("Metrics server error: {}", e);
            }
        }
        _ = polling_task => {
            println!("Polling loop ended");
        }
    }
    
    Ok(())
}
