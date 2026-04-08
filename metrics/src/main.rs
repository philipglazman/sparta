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
    /// Converts the hourly predictive funding rate to an annualized percentage
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

#[derive(Debug, Serialize, Deserialize)]
struct AssetContextsRequest {
    r#type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AssetUniverse {
    name: String,
    #[serde(rename = "szDecimals")]
    sz_decimals: i32,
    #[serde(rename = "maxLeverage")]
    max_leverage: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct AssetContext {
    #[serde(rename = "dayNtlVlm")]
    day_notional_volume: String,
    funding: String,
    #[serde(rename = "impactPxs")]
    impact_prices: Option<[String; 2]>,
    #[serde(rename = "markPx")]
    mark_price: String,
    #[serde(rename = "midPx")]
    mid_price: Option<String>,
    #[serde(rename = "openInterest")]
    open_interest: String,
    #[serde(rename = "oraclePx")]
    oracle_price: String,
    premium: Option<String>,
    #[serde(rename = "prevDayPx")]
    prev_day_price: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct MetaData {
    universe: Vec<AssetUniverse>,
}

type AssetContextsResponse = (MetaData, Vec<AssetContext>);

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

    async fn get_asset_contexts(&self) -> Result<AssetContextsResponse, Box<dyn std::error::Error>> {
        let request_body = AssetContextsRequest {
            r#type: "metaAndAssetCtxs".to_string(),
        };

        let response = self
            .client
            .post(&format!("{}/info", self.base_url))
            .json(&request_body)
            .send()
            .await?;

        // Debug: print the raw response text before attempting to decode
        let response_text = response.text().await?;
        
        let asset_data: AssetContextsResponse = serde_json::from_str(&response_text)?;
        Ok(asset_data)
    }
}

struct MetricsCollector {
    registry: Registry,
    predictive_funding_rate_gauge: GaugeVec,
    predictive_funding_rate_changes: CounterVec,
    current_funding_rate_gauge: GaugeVec,
    daily_volume_gauge: GaugeVec,
    mid_price_gauge: GaugeVec,
    prev_day_price_gauge: GaugeVec,
    previous_rates: Arc<tokio::sync::Mutex<HashMap<String, f64>>>,
}

impl MetricsCollector {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();
        
        let predictive_funding_rate_gauge = GaugeVec::new(
            Opts::new("hyperliquid_predictive_funding_rate", "Current predictive funding rate for each coin and venue"),
            &["coin", "venue"]
        )?;
        
        let predictive_funding_rate_changes = CounterVec::new(
            Opts::new("hyperliquid_predictive_funding_rate_changes_total", "Total number of predictive funding rate changes"),
            &["coin", "venue", "direction"]
        )?;

        let current_funding_rate_gauge = GaugeVec::new(
            Opts::new("hyperliquid_current_funding_rate", "Current funding rate for each coin from asset contexts"),
            &["coin"]
        )?;

        let daily_volume_gauge = GaugeVec::new(
            Opts::new("hyperliquid_daily_volume", "Daily notional volume for each coin"),
            &["coin"]
        )?;

        let mid_price_gauge = GaugeVec::new(
            Opts::new("hyperliquid_mid_price", "Current mid price for each coin"),
            &["coin"]
        )?;

        let prev_day_price_gauge = GaugeVec::new(
            Opts::new("hyperliquid_prev_day_price", "Previous day price for each coin"),
            &["coin"]
        )?;
        
        registry.register(Box::new(predictive_funding_rate_gauge.clone()))?;
        registry.register(Box::new(predictive_funding_rate_changes.clone()))?;
        registry.register(Box::new(current_funding_rate_gauge.clone()))?;
        registry.register(Box::new(daily_volume_gauge.clone()))?;
        registry.register(Box::new(mid_price_gauge.clone()))?;
        registry.register(Box::new(prev_day_price_gauge.clone()))?;
        
        Ok(Self {
            registry,
            predictive_funding_rate_gauge,
            predictive_funding_rate_changes,
            current_funding_rate_gauge,
            daily_volume_gauge,
            mid_price_gauge,
            prev_day_price_gauge,
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
                        
                        self.predictive_funding_rate_gauge
                            .with_label_values(&[coin, venue])
                            .set(annualized_rate);
                        
                        if let Some(&previous_rate) = previous_rates.get(&key) {
                            if annualized_rate != previous_rate {
                                let direction = if annualized_rate > previous_rate { "up" } else { "down" };
                                self.predictive_funding_rate_changes
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

    async fn update_asset_metrics(&self, asset_data: &AssetContextsResponse) {
        let (meta_data, contexts) = asset_data;
        let universe = &meta_data.universe;
        
        for (i, context) in contexts.iter().enumerate() {
            if let Some(coin_info) = universe.get(i) {
                let coin_name = &coin_info.name;
                
                // Parse and set daily volume
                if let Ok(volume) = context.day_notional_volume.parse::<f64>() {
                    self.daily_volume_gauge
                        .with_label_values(&[coin_name])
                        .set(volume);
                }
                
                // Parse and set mid price (handle None case)
                if let Some(ref mid_price_str) = context.mid_price {
                    if let Ok(mid_price) = mid_price_str.parse::<f64>() {
                        self.mid_price_gauge
                            .with_label_values(&[coin_name])
                            .set(mid_price);
                    }
                }
                
                // Parse and set previous day price
                if let Ok(prev_price) = context.prev_day_price.parse::<f64>() {
                    self.prev_day_price_gauge
                        .with_label_values(&[coin_name])
                        .set(prev_price);
                }
                
                // Parse and set current funding rate
                if let Ok(funding_rate) = context.funding.parse::<f64>() {
                    self.current_funding_rate_gauge
                        .with_label_values(&[coin_name])
                        .set(funding_rate);
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
        
        // Fetch predicted funding rates
        match client.get_predicted_fundings().await {
            Ok(funding_data) => {
                metrics.update_metrics(&funding_data).await;
                println!("Updated predictive funding metrics for {} coins", funding_data.len());
            }
            Err(e) => {
                eprintln!("Error fetching funding data: {}", e);
            }
        }

        // Fetch asset contexts (daily volume, prices, etc.)
        match client.get_asset_contexts().await {
            Ok(asset_data) => {
                metrics.update_asset_metrics(&asset_data).await;
                println!("Updated asset context metrics for {} coins", asset_data.1.len());
            }
            Err(e) => {
                eprintln!("Error fetching asset context data: {}", e);
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
