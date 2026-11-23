//! The country of the current network exit, as Google's servers see it.
use serde_json::Value;
use std::time::Duration;

const LOOKUP: &str = "http://ip-api.com/json/?fields=countryCode";

/// Two-letter country code such as `US`, or `None` when the lookup failed.
pub fn country() -> Option<String> {
    let response: Value = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?
        .get(LOOKUP)
        .send()
        .ok()?
        .json()
        .ok()?;
    response["countryCode"].as_str().map(str::to_owned)
}
