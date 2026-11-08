use hearth_config::Settings;

#[test]
fn pricing_defaults_to_empty_and_accepts_a_model() {
    let settings: Settings = serde_json::from_str(
        r#"{"models":{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m","pricing":{"m":{"input":1000000,"cache_read":250000,"cache_write":1250000,"output":2000000,"reasoning":0}}}}"#,
    )
    .expect("settings");
    settings.validate().expect("valid pricing");
    let price = settings.models.expect("models").pricing["m"];
    assert_eq!(price.input, 1_000_000);
    assert_eq!(price.cache_read, 250_000);
    assert_eq!(price.output, 2_000_000);
}

#[test]
fn a_negative_price_is_rejected() {
    let settings: Settings = serde_json::from_str(
        r#"{"models":{"kind":"openai-compatible","base_url":"https://example.com/v1","api_key":"fake","model":"m","pricing":{"m":{"input":-1}}}}"#,
    )
    .expect("settings");
    let error = settings.validate().expect_err("negative price");
    assert!(
        error.to_string().contains("models.pricing.m.input"),
        "{error}"
    );
}
