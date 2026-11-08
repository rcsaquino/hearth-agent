use hearth_model::{Price, Pricing, Usage};

#[test]
fn a_price_costs_every_token_category() {
    let price = Price {
        input: 2_000_000,
        cache_read: 500_000,
        cache_write: 1_000_000,
        output: 8_000_000,
        reasoning: 4_000_000,
    };
    let usage = Usage {
        input_tokens: 1_000_000,
        cache_read_tokens: 2_000_000,
        cache_write_tokens: 500_000,
        output_tokens: 1_000_000,
        reasoning_tokens: 100_000,
    };

    assert_eq!(price.cost_micros(&usage), 11_900_000);

    let mut pricing = Pricing::new();
    pricing.insert("test-model", price);
    assert_eq!(pricing.price("test-model"), Some(price));
    assert_eq!(pricing.price("unknown-model"), None);
}
