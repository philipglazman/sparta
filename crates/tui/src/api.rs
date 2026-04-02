use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Deserialize;

/// Returns the unix timestamp (seconds) of the most recent Sunday at 00:00 UTC,
/// relative to the given UTC time.
pub fn most_recent_sunday_midnight_utc(now: DateTime<Utc>) -> i64 {
    let today = now.date_naive();
    let days_since_sunday = today.weekday().num_days_from_sunday();
    let sunday = today - chrono::Duration::days(days_since_sunday as i64);
    let sunday_midnight = sunday.and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    Utc.from_utc_datetime(&sunday_midnight).timestamp()
}

/// Returns the unix timestamp (seconds) of the 1st of the current month at 00:00 UTC,
/// relative to the given UTC time.
pub fn first_of_month_utc(now: DateTime<Utc>) -> i64 {
    let first = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .unwrap()
        .and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    Utc.from_utc_datetime(&first).timestamp()
}

#[derive(Deserialize)]
pub struct CoinPrice {
    pub usd: f64,
}

#[derive(Deserialize)]
pub struct PriceResponse {
    pub bitcoin: CoinPrice,
    pub ethereum: CoinPrice,
}

#[derive(Deserialize)]
pub struct MarketChartResponse {
    pub prices: Vec<(f64, f64)>, // (timestamp_ms, price)
}

/// Returns the unix timestamp (seconds) of today at 00:00 UTC.
pub fn today_midnight_utc(now: DateTime<Utc>) -> i64 {
    let today = now.date_naive();
    let midnight = today.and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    Utc.from_utc_datetime(&midnight).timestamp()
}

use crate::app::AppData;

const BASE_URL: &str = "https://api.coingecko.com/api/v3";

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request failed: {}", e))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read body failed: {}", e))?;

    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, &body[..body.len().min(200)]));
    }

    serde_json::from_str(&body)
        .map_err(|e| format!("JSON parse error: {} (body: {})", e, &body[..body.len().min(200)]))
}

pub async fn fetch_data(client: &reqwest::Client) -> Result<AppData, String> {
    let now = Utc::now();

    // Fetch current prices (BTC + ETH)
    let price_url = format!(
        "{}/simple/price?ids=bitcoin,ethereum&vs_currencies=usd",
        BASE_URL
    );
    let price_resp: PriceResponse = get_json(client, &price_url).await?;

    let today_ts = today_midnight_utc(now);
    let sunday_ts = most_recent_sunday_midnight_utc(now);
    let month_ts = first_of_month_utc(now);

    // Fetch BTC daily open
    let btc_daily_url = format!(
        "{}/coins/bitcoin/market_chart/range?vs_currency=usd&from={}&to={}",
        BASE_URL, today_ts - 3600, today_ts + 3600
    );
    let btc_daily_resp: MarketChartResponse = get_json(client, &btc_daily_url).await?;
    let daily_open = closest_price(&btc_daily_resp.prices, today_ts)
        .ok_or_else(|| "no BTC daily open data".to_string())?;

    // Fetch ETH daily open
    let eth_daily_url = format!(
        "{}/coins/ethereum/market_chart/range?vs_currency=usd&from={}&to={}",
        BASE_URL, today_ts - 3600, today_ts + 3600
    );
    let eth_daily_resp: MarketChartResponse = get_json(client, &eth_daily_url).await?;
    let eth_daily_open = closest_price(&eth_daily_resp.prices, today_ts)
        .ok_or_else(|| "no ETH daily open data".to_string())?;

    // Fetch BTC weekly open
    let weekly_url = format!(
        "{}/coins/bitcoin/market_chart/range?vs_currency=usd&from={}&to={}",
        BASE_URL, sunday_ts - 3600, sunday_ts + 3600
    );
    let weekly_resp: MarketChartResponse = get_json(client, &weekly_url).await?;
    let weekly_open = closest_price(&weekly_resp.prices, sunday_ts)
        .ok_or_else(|| "no weekly price data".to_string())?;

    // Fetch BTC monthly open
    let monthly_url = format!(
        "{}/coins/bitcoin/market_chart/range?vs_currency=usd&from={}&to={}",
        BASE_URL, month_ts - 3600, month_ts + 3600
    );
    let monthly_resp: MarketChartResponse = get_json(client, &monthly_url).await?;
    let monthly_open = closest_price(&monthly_resp.prices, month_ts)
        .ok_or_else(|| "no monthly price data".to_string())?;

    let weekly_open_date = DateTime::from_timestamp(sunday_ts, 0).unwrap_or(now);
    let monthly_open_date = DateTime::from_timestamp(month_ts, 0).unwrap_or(now);

    Ok(AppData {
        price: price_resp.bitcoin.usd,
        daily_open,
        eth_price: price_resp.ethereum.usd,
        eth_daily_open,
        weekly_open,
        weekly_open_date,
        monthly_open,
        monthly_open_date,
        last_updated: now,
    })
}

/// From a list of (timestamp_ms, price) pairs, find the price closest to `target_ts` (seconds).
fn closest_price(prices: &[(f64, f64)], target_ts: i64) -> Option<f64> {
    let target_ms = target_ts as f64 * 1000.0;
    prices
        .iter()
        .min_by_key(|(ts, _)| (ts - target_ms).abs() as i64)
        .map(|(_, price)| *price)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn utc(s: &str) -> DateTime<Utc> {
        let naive = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        Utc.from_utc_datetime(&naive)
    }

    #[test]
    fn test_sunday_midnight_on_a_wednesday() {
        // Wednesday March 25, 2026 15:00 UTC
        let now = utc("2026-03-25 15:00:00");
        let ts = most_recent_sunday_midnight_utc(now);
        // Most recent Sunday is March 22, 00:00 UTC
        let expected = utc("2026-03-22 00:00:00").timestamp();
        assert_eq!(ts, expected);
    }

    #[test]
    fn test_sunday_midnight_on_sunday_afternoon() {
        // Sunday March 29, 2026 12:00 UTC
        let now = utc("2026-03-29 12:00:00");
        let ts = most_recent_sunday_midnight_utc(now);
        // Should be this Sunday March 29 00:00 UTC
        let expected = utc("2026-03-29 00:00:00").timestamp();
        assert_eq!(ts, expected);
    }

    #[test]
    fn test_sunday_midnight_on_monday() {
        // Monday March 30, 2026 01:00 UTC
        let now = utc("2026-03-30 01:00:00");
        let ts = most_recent_sunday_midnight_utc(now);
        // Most recent Sunday is March 29 00:00 UTC
        let expected = utc("2026-03-29 00:00:00").timestamp();
        assert_eq!(ts, expected);
    }

    #[test]
    fn test_first_of_month() {
        let now = utc("2026-03-15 10:30:00");
        let ts = first_of_month_utc(now);
        let expected = utc("2026-03-01 00:00:00").timestamp();
        assert_eq!(ts, expected);
    }

    #[test]
    fn test_first_of_month_on_first() {
        let now = utc("2026-03-01 00:00:01");
        let ts = first_of_month_utc(now);
        let expected = utc("2026-03-01 00:00:00").timestamp();
        assert_eq!(ts, expected);
    }

    #[test]
    fn test_closest_price_exact() {
        let prices = vec![
            (1000000.0, 85000.0),
            (1003600000.0, 85500.0),
            (1007200000.0, 86000.0),
        ];
        let result = super::closest_price(&prices, 1003600);
        assert_eq!(result, Some(85500.0));
    }

    #[test]
    fn test_closest_price_nearest() {
        let prices = vec![
            (1000000000.0, 85000.0),
            (1000060000.0, 85100.0),
        ];
        let result = super::closest_price(&prices, 1000030);
        assert!(result == Some(85000.0) || result == Some(85100.0));
    }

    #[test]
    fn test_closest_price_empty() {
        let prices: Vec<(f64, f64)> = vec![];
        assert_eq!(super::closest_price(&prices, 1000000), None);
    }

    #[test]
    fn test_deserialize_price_response() {
        let json = r#"{"bitcoin":{"usd":87420.0},"ethereum":{"usd":2000.0}}"#;
        let resp: super::PriceResponse = serde_json::from_str(json).unwrap();
        assert!((resp.bitcoin.usd - 87420.0).abs() < 0.01);
        assert!((resp.ethereum.usd - 2000.0).abs() < 0.01);
    }

    #[test]
    fn test_today_midnight_utc() {
        let now = utc("2026-04-01 14:30:00");
        let ts = super::today_midnight_utc(now);
        let expected = utc("2026-04-01 00:00:00").timestamp();
        assert_eq!(ts, expected);
    }

    #[test]
    fn test_deserialize_market_chart_response() {
        let json = r#"{"prices":[[1706000000000.0,42000.0],[1706003600000.0,42100.0]]}"#;
        let resp: super::MarketChartResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.prices.len(), 2);
        assert!((resp.prices[0].1 - 42000.0).abs() < 0.01);
    }
}
