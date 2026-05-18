use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::Serialize;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

const OPENAI_PRICING_URL: &str = "https://openai.com/api/pricing/";
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Serialize)]
pub struct PricingCatalog {
    pub provider: String,
    pub source_url: String,
    pub fetched_at: String,
    pub status: String,
    pub tier: String,
    pub context: String,
    pub currency: String,
    pub unit: String,
    pub prices: Vec<ModelTokenPrice>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelTokenPrice {
    pub model: String,
    pub aliases: Vec<String>,
    pub input_per_million: f64,
    pub cached_input_per_million: Option<f64>,
    pub output_per_million: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CostEstimate {
    pub total_usd: f64,
    pub input_usd: f64,
    pub cached_input_usd: f64,
    pub output_usd: f64,
    pub priced_tokens: i64,
    pub model: String,
    pub price_source: String,
}

#[derive(Debug, Clone)]
struct CachedCatalog {
    catalog: PricingCatalog,
    loaded_at: SystemTime,
}

static CATALOG_CACHE: OnceLock<Mutex<Option<CachedCatalog>>> = OnceLock::new();

pub fn openai_pricing_catalog() -> PricingCatalog {
    let cache = CATALOG_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some(cached) = guard.as_ref() {
            if cached
                .loaded_at
                .elapsed()
                .is_ok_and(|elapsed| elapsed < CACHE_TTL)
            {
                return cached.catalog.clone();
            }
        }
    }

    let catalog = fetch_openai_pricing_catalog().unwrap_or_else(|err| {
        let mut fallback = fallback_openai_catalog();
        fallback.status = "fallback".to_string();
        fallback.error = Some(err.to_string());
        fallback
    });

    if let Ok(mut guard) = cache.lock() {
        *guard = Some(CachedCatalog {
            catalog: catalog.clone(),
            loaded_at: SystemTime::now(),
        });
    }

    catalog
}

pub fn estimate_cost(
    model: Option<&str>,
    input_tokens: i64,
    cached_tokens: i64,
    output_tokens: i64,
    _reasoning_tokens: i64,
    catalog: &PricingCatalog,
) -> Option<CostEstimate> {
    let model = model?;
    let price = catalog.price_for(model)?;
    let input_tokens = input_tokens.max(0);
    let cached_tokens = cached_tokens.max(0).min(input_tokens);
    let output_tokens = output_tokens.max(0);
    let cached_rate = price.cached_input_per_million;
    // Cached input is a subset of input, and reasoning output is a subset of output.
    let input_usd = if cached_rate.is_some() {
        cost_for(input_tokens - cached_tokens, price.input_per_million)
    } else {
        cost_for(input_tokens, price.input_per_million)
    };
    let cached_input_usd = cached_rate
        .map(|rate| cost_for(cached_tokens, rate))
        .unwrap_or(0.0);
    let output_usd = cost_for(output_tokens, price.output_per_million);
    Some(CostEstimate {
        total_usd: round_usd(input_usd + cached_input_usd + output_usd),
        input_usd: round_usd(input_usd),
        cached_input_usd: round_usd(cached_input_usd),
        output_usd: round_usd(output_usd),
        priced_tokens: input_tokens + output_tokens,
        model: price.model.clone(),
        price_source: price.source.clone(),
    })
}

impl PricingCatalog {
    fn price_for(&self, model: &str) -> Option<&ModelTokenPrice> {
        let wanted = normalize_model(model);
        self.prices.iter().find(|price| {
            normalize_model(&price.model) == wanted
                || price
                    .aliases
                    .iter()
                    .any(|alias| normalize_model(alias) == wanted)
        })
    }
}

fn fetch_openai_pricing_catalog() -> Result<PricingCatalog> {
    let body = fetch_openai_pricing_page()?;
    let mut catalog = fallback_openai_catalog();
    let live_prices = parse_openai_pricing_page(&body);
    if live_prices.is_empty() {
        bail!("OpenAI pricing page did not contain parseable token prices");
    }

    for live_price in live_prices {
        upsert_price(&mut catalog.prices, live_price);
    }

    catalog.status = "live".to_string();
    catalog.error = None;
    catalog.fetched_at = Utc::now().to_rfc3339();
    Ok(catalog)
}

fn fetch_openai_pricing_page() -> Result<String> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--compressed",
            "--max-time",
            "8",
            "-A",
            "Mozilla/5.0 (atrail)",
            OPENAI_PRICING_URL,
        ])
        .output()
        .context("run curl for OpenAI pricing")?;
    if !output.status.success() {
        return Err(anyhow!(
            "OpenAI pricing fetch failed with status {}",
            output.status
        ));
    }
    String::from_utf8(output.stdout).context("decode OpenAI pricing response")
}

fn parse_openai_pricing_page(body: &str) -> Vec<ModelTokenPrice> {
    let text = html_to_text(body);
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut prices = Vec::new();
    let mut idx = 0;

    while idx < lines.len() {
        let line = lines[idx];
        if !looks_like_model_heading(line) {
            idx += 1;
            continue;
        }

        let model = normalize_heading_to_model(line);
        let mut input = None;
        let mut cached = None;
        let mut output = None;
        let mut scan = idx + 1;
        while scan < lines.len() && !looks_like_model_heading(lines[scan]) {
            let label = lines[scan].to_ascii_lowercase();
            if label.starts_with("input") {
                input = parse_usd_per_million(lines[scan])
                    .or_else(|| find_price_after(&lines, scan + 1));
            } else if label.starts_with("cached input") {
                cached = parse_usd_per_million(lines[scan])
                    .or_else(|| find_price_after(&lines, scan + 1));
            } else if label.starts_with("output") {
                output = parse_usd_per_million(lines[scan])
                    .or_else(|| find_price_after(&lines, scan + 1));
            }
            scan += 1;
        }

        if let (Some(input_per_million), Some(output_per_million)) = (input, output) {
            prices.push(ModelTokenPrice {
                model,
                aliases: Vec::new(),
                input_per_million,
                cached_input_per_million: cached,
                output_per_million,
                source: "live".to_string(),
            });
        }
        idx = scan;
    }

    prices
}

fn fallback_openai_catalog() -> PricingCatalog {
    PricingCatalog {
        provider: "openai".to_string(),
        source_url: OPENAI_PRICING_URL.to_string(),
        fetched_at: Utc::now().to_rfc3339(),
        status: "fallback".to_string(),
        tier: "standard".to_string(),
        context: "short".to_string(),
        currency: "USD".to_string(),
        unit: "1M tokens".to_string(),
        prices: vec![
            price("gpt-5.5", &["gpt-5.5-chat-latest"], 5.0, Some(0.5), 30.0),
            price("gpt-5.4", &["gpt-5.4-chat-latest"], 2.5, Some(0.25), 15.0),
            price("gpt-5.4-mini", &["gpt-5.4 mini"], 0.75, Some(0.075), 4.5),
            price("gpt-5.4-nano", &["gpt-5.4 nano"], 0.20, Some(0.02), 1.25),
            price("gpt-5.4-pro", &["gpt-5.4 pro"], 30.0, None, 180.0),
            price("gpt-5.3-codex", &["gpt-5.3 codex"], 1.75, Some(0.175), 14.0),
            price("gpt-5.2", &["gpt-5.2-chat-latest"], 1.75, Some(0.175), 14.0),
            price("gpt-5.2-codex", &["gpt-5.2 codex"], 1.75, Some(0.175), 14.0),
            price(
                "gpt-5.1-codex",
                &["gpt-5.1 codex", "gpt-5.1-codex-max"],
                1.25,
                Some(0.125),
                10.0,
            ),
            price("gpt-5-codex", &["gpt-5 codex"], 1.25, Some(0.125), 10.0),
            price("gpt-5", &["gpt-5-chat-latest"], 1.25, Some(0.125), 10.0),
            price("gpt-5-mini", &["gpt-5 mini"], 0.25, Some(0.025), 2.0),
            price("gpt-5-nano", &["gpt-5 nano"], 0.05, Some(0.005), 0.4),
            price("gpt-4.1", &[], 2.0, Some(0.5), 8.0),
            price("gpt-4.1-mini", &[], 0.4, Some(0.1), 1.6),
            price("gpt-4.1-nano", &[], 0.1, Some(0.025), 0.4),
            price("gpt-4o", &[], 2.5, Some(1.25), 10.0),
            price("gpt-4o-mini", &[], 0.15, Some(0.075), 0.6),
        ],
        error: None,
    }
}

fn price(
    model: &str,
    aliases: &[&str],
    input_per_million: f64,
    cached_input_per_million: Option<f64>,
    output_per_million: f64,
) -> ModelTokenPrice {
    ModelTokenPrice {
        model: model.to_string(),
        aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
        input_per_million,
        cached_input_per_million,
        output_per_million,
        source: "fallback".to_string(),
    }
}

fn upsert_price(prices: &mut Vec<ModelTokenPrice>, live: ModelTokenPrice) {
    if let Some(existing) = prices
        .iter_mut()
        .find(|price| normalize_model(&price.model) == normalize_model(&live.model))
    {
        existing.input_per_million = live.input_per_million;
        existing.cached_input_per_million = live.cached_input_per_million;
        existing.output_per_million = live.output_per_million;
        existing.source = live.source;
    } else {
        prices.push(live);
    }
}

fn html_to_text(body: &str) -> String {
    let mut text = String::with_capacity(body.len());
    let mut in_tag = false;
    for ch in body.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push('\n');
            }
            '>' => {
                in_tag = false;
                text.push('\n');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&#x2F;", "/")
        .replace("&quot;", "\"")
}

fn looks_like_model_heading(line: &str) -> bool {
    let normalized = normalize_heading_to_model(line);
    normalized.starts_with("gpt-")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
}

fn normalize_heading_to_model(line: &str) -> String {
    line.trim_matches(|ch: char| {
        ch == '#'
            || ch == '*'
            || ch == '`'
            || ch == ':'
            || ch == ','
            || ch == '.'
            || ch.is_whitespace()
    })
    .trim_start_matches("Model ")
    .to_ascii_lowercase()
    .replace(' ', "-")
}

fn find_price_after(lines: &[&str], start: usize) -> Option<f64> {
    lines
        .iter()
        .skip(start)
        .take(4)
        .find_map(|line| parse_usd_per_million(line))
}

fn parse_usd_per_million(line: &str) -> Option<f64> {
    let dollar = line.find('$')?;
    let number = line[dollar + 1..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.' || *ch == ',')
        .collect::<String>()
        .replace(',', "");
    number.parse::<f64>().ok()
}

fn normalize_model(model: &str) -> String {
    model
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '_'], "-")
        .replace(['‑', '–', '—'], "-")
}

fn cost_for(tokens: i64, per_million: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * per_million
}

fn round_usd(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pricing_text_blocks() {
        let page = r#"
        GPT-5.5
        Input:
        $5.00 / 1M tokens
        Cached input:
        $0.50 / 1M tokens
        Output:
        $30.00 / 1M tokens

        GPT-5.4 mini
        Input:
        $0.75 / 1M tokens
        Cached input:
        $0.075 / 1M tokens
        Output:
        $4.50 / 1M tokens
        "#;

        let prices = parse_openai_pricing_page(page);
        assert_eq!(prices.len(), 2);
        assert_eq!(prices[0].model, "gpt-5.5");
        assert_eq!(prices[0].input_per_million, 5.0);
        assert_eq!(prices[0].cached_input_per_million, Some(0.5));
        assert_eq!(prices[1].model, "gpt-5.4-mini");
        assert_eq!(prices[1].output_per_million, 4.5);
    }

    #[test]
    fn estimates_cost_with_cached_and_reasoning_tokens() {
        let catalog = fallback_openai_catalog();
        let cost = estimate_cost(
            Some("gpt-5.5"),
            1_000_000,
            500_000,
            200_000,
            100_000,
            &catalog,
        )
        .unwrap();
        assert_eq!(cost.total_usd, 8.75);
    }
}
