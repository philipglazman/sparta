use chrono::{DateTime, Utc};

pub struct AppData {
    pub price: f64,
    pub daily_open: f64,
    pub eth_price: f64,
    pub eth_daily_open: f64,
    pub weekly_open: f64,
    pub weekly_open_date: DateTime<Utc>,
    pub monthly_open: f64,
    pub monthly_open_date: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
}

impl AppData {
    pub fn btc_daily_change_pct(&self) -> f64 {
        if self.daily_open == 0.0 { return 0.0; }
        ((self.price - self.daily_open) / self.daily_open) * 100.0
    }

    pub fn eth_daily_change_pct(&self) -> f64 {
        if self.eth_daily_open == 0.0 { return 0.0; }
        ((self.eth_price - self.eth_daily_open) / self.eth_daily_open) * 100.0
    }
}

pub struct PercentageMove {
    pub percent: f64,
    pub price_up: f64,
    pub price_down: f64,
}

impl AppData {
    pub fn percentage_moves(&self) -> Vec<PercentageMove> {
        [1.0, 2.0, 5.0]
            .iter()
            .map(|&pct| PercentageMove {
                percent: pct,
                price_up: self.price * (1.0 + pct / 100.0),
                price_down: self.price * (1.0 - pct / 100.0),
            })
            .collect()
    }

    pub fn weekly_change_pct(&self) -> f64 {
        if self.weekly_open == 0.0 {
            return 0.0;
        }
        ((self.price - self.weekly_open) / self.weekly_open) * 100.0
    }

    pub fn monthly_change_pct(&self) -> f64 {
        if self.monthly_open == 0.0 {
            return 0.0;
        }
        ((self.price - self.monthly_open) / self.monthly_open) * 100.0
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Direction {
    Long,
    Short,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PnlField {
    Entry,
    Value,
    Target,
}

impl PnlField {
    pub fn next(self) -> Self {
        match self {
            PnlField::Entry => PnlField::Value,
            PnlField::Value => PnlField::Target,
            PnlField::Target => PnlField::Entry,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            PnlField::Entry => PnlField::Target,
            PnlField::Value => PnlField::Entry,
            PnlField::Target => PnlField::Value,
        }
    }
}

pub struct PnlResult {
    pub label: String,
    pub price: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
}

pub struct PnlCalculator {
    pub direction: Direction,
    pub entry_buf: String,
    pub value_buf: String,
    pub target_buf: String,
    pub focused_field: PnlField,
    pub active: bool,
}

impl PnlCalculator {
    pub fn new() -> Self {
        Self {
            direction: Direction::Long,
            entry_buf: String::new(),
            value_buf: String::new(),
            target_buf: String::new(),
            focused_field: PnlField::Entry,
            active: false,
        }
    }

    fn entry(&self) -> Option<f64> {
        self.entry_buf.parse().ok().filter(|&v: &f64| v > 0.0)
    }

    fn value(&self) -> Option<f64> {
        self.value_buf.parse().ok().filter(|&v: &f64| v > 0.0)
    }

    fn target(&self) -> Option<f64> {
        self.target_buf.parse().ok().filter(|&v: &f64| v > 0.0)
    }

    pub fn calc_pnl(&self, exit_price: f64) -> Option<PnlResult> {
        let entry = self.entry()?;
        let value = self.value()?;
        let qty = value / entry;

        let pnl = match self.direction {
            Direction::Long => qty * (exit_price - entry),
            Direction::Short => qty * (entry - exit_price),
        };
        let pnl_pct = (pnl / value) * 100.0;

        Some(PnlResult {
            label: String::new(),
            price: exit_price,
            pnl,
            pnl_pct,
        })
    }

    /// Returns PNL results at: current price, target, ±1%, ±2%, ±5%
    pub fn results(&self, current_price: f64) -> Vec<PnlResult> {
        let mut results = Vec::new();
        let entry = match self.entry() {
            Some(e) => e,
            None => return results,
        };
        if self.value().is_none() {
            return results;
        }

        // At current price
        if let Some(mut r) = self.calc_pnl(current_price) {
            r.label = "Current".to_string();
            results.push(r);
        }

        // At target
        if let Some(target) = self.target() {
            if let Some(mut r) = self.calc_pnl(target) {
                r.label = "Target".to_string();
                results.push(r);
            }
        }

        // At percentage moves from entry
        for pct in [1.0, 2.0, 5.0] {
            let price_up = entry * (1.0 + pct / 100.0);
            let price_down = entry * (1.0 - pct / 100.0);
            if let Some(mut r) = self.calc_pnl(price_up) {
                r.label = format!("+{}%", pct);
                results.push(r);
            }
            if let Some(mut r) = self.calc_pnl(price_down) {
                r.label = format!("-{}%", pct);
                results.push(r);
            }
        }

        results
    }

    pub fn active_buf_mut(&mut self) -> &mut String {
        match self.focused_field {
            PnlField::Entry => &mut self.entry_buf,
            PnlField::Value => &mut self.value_buf,
            PnlField::Target => &mut self.target_buf,
        }
    }
}

pub enum FetchStatus {
    Loading,
    Ok,
    Error(String),
}

pub struct App {
    pub data: Option<AppData>,
    pub status: FetchStatus,
    pub seconds_until_refresh: u64,
    pub logs: Vec<String>,
    pub pnl: PnlCalculator,
}

impl App {
    pub fn new() -> Self {
        Self {
            data: None,
            status: FetchStatus::Loading,
            seconds_until_refresh: 60,
            logs: vec!["Starting trade-tui...".to_string()],
            pnl: PnlCalculator::new(),
        }
    }

    pub fn log(&mut self, msg: String) {
        self.logs.push(msg);
        // Keep last 50 lines
        if self.logs.len() > 50 {
            self.logs.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_data() -> AppData {
        AppData {
            price: 100_000.0,
            daily_open: 98_000.0,
            eth_price: 2_000.0,
            eth_daily_open: 1_950.0,
            weekly_open: 95_000.0,
            weekly_open_date: Utc::now(),
            monthly_open: 90_000.0,
            monthly_open_date: Utc::now(),
            last_updated: Utc::now(),
        }
    }

    #[test]
    fn test_percentage_moves() {
        let data = sample_data();
        let moves = data.percentage_moves();

        assert_eq!(moves.len(), 3);

        // 1% move
        assert!((moves[0].price_up - 101_000.0).abs() < 0.01);
        assert!((moves[0].price_down - 99_000.0).abs() < 0.01);

        // 2% move
        assert!((moves[1].price_up - 102_000.0).abs() < 0.01);
        assert!((moves[1].price_down - 98_000.0).abs() < 0.01);

        // 5% move
        assert!((moves[2].price_up - 105_000.0).abs() < 0.01);
        assert!((moves[2].price_down - 95_000.0).abs() < 0.01);
    }

    #[test]
    fn test_weekly_change_pct() {
        let data = sample_data();
        let pct = data.weekly_change_pct();
        // (100000 - 95000) / 95000 * 100 = 5.2631...
        assert!((pct - 5.2631).abs() < 0.01);
    }

    #[test]
    fn test_monthly_change_pct() {
        let data = sample_data();
        let pct = data.monthly_change_pct();
        // (100000 - 90000) / 90000 * 100 = 11.1111...
        assert!((pct - 11.1111).abs() < 0.01);
    }

    #[test]
    fn test_pnl_long() {
        let mut calc = PnlCalculator::new();
        calc.direction = Direction::Long;
        calc.entry_buf = "100000".to_string();
        calc.value_buf = "10000".to_string();

        // Price goes to 101000 (+1%), qty = 10000/100000 = 0.1 BTC
        // PNL = 0.1 * (101000 - 100000) = 100
        let r = calc.calc_pnl(101000.0).unwrap();
        assert!((r.pnl - 100.0).abs() < 0.01);
        assert!((r.pnl_pct - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_pnl_short() {
        let mut calc = PnlCalculator::new();
        calc.direction = Direction::Short;
        calc.entry_buf = "100000".to_string();
        calc.value_buf = "10000".to_string();

        // Price drops to 99000 (-1%), qty = 0.1 BTC
        // PNL = 0.1 * (100000 - 99000) = 100
        let r = calc.calc_pnl(99000.0).unwrap();
        assert!((r.pnl - 100.0).abs() < 0.01);
        assert!((r.pnl_pct - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_pnl_losing_long() {
        let mut calc = PnlCalculator::new();
        calc.direction = Direction::Long;
        calc.entry_buf = "100000".to_string();
        calc.value_buf = "10000".to_string();

        // Price drops to 95000 (-5%), qty = 0.1 BTC
        // PNL = 0.1 * (95000 - 100000) = -500
        let r = calc.calc_pnl(95000.0).unwrap();
        assert!((r.pnl - (-500.0)).abs() < 0.01);
    }

    #[test]
    fn test_pnl_empty_inputs() {
        let calc = PnlCalculator::new();
        assert!(calc.calc_pnl(100000.0).is_none());
    }

    #[test]
    fn test_zero_open_does_not_panic() {
        let data = AppData {
            price: 100_000.0,
            daily_open: 100_000.0,
            eth_price: 2_000.0,
            eth_daily_open: 2_000.0,
            weekly_open: 0.0,
            weekly_open_date: Utc::now(),
            monthly_open: 0.0,
            monthly_open_date: Utc::now(),
            last_updated: Utc::now(),
        };
        assert_eq!(data.weekly_change_pct(), 0.0);
        assert_eq!(data.monthly_change_pct(), 0.0);
    }
}
