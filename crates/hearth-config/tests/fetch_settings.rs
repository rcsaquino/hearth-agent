use hearth_config::Settings;

#[test]
fn web_fetch_defaults_to_jina_then_http() {
    let settings: Settings = serde_json::from_str(r#"{"web":{}}"#).expect("settings");
    let web = settings.web.expect("web");
    assert_eq!(web.fetch.order, vec!["jina".to_owned(), "http".to_owned()]);
    assert!(web.fetch.providers.is_empty());
}

#[test]
fn web_fetch_rejects_an_unknown_provider() {
    let settings: Settings =
        serde_json::from_str(r#"{"web":{"fetch":{"order":["nope"]}}}"#).expect("settings");
    let error = settings.validate().expect_err("unknown provider");
    assert!(error.to_string().contains("web.fetch.order"), "{error}");
}

#[test]
fn web_fetch_accepts_the_allowed_chain() {
    let settings: Settings = serde_json::from_str(
        r#"{"web":{"fetch":{"order":["jina","https","tinyfish","firecrawl","deepseek"],"providers":{"jina":{},"tinyfish":{"api_key":"t"},"firecrawl":{"api_key":"f"},"deepseek":{"api_key":"d"}}}}}"#,
    )
    .expect("settings");
    settings.validate().expect("valid settings");
    assert_eq!(settings.web.expect("web").fetch.order.len(), 5);
}
