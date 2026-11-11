-- Cost/pricing is not tracked. Token usage remains.
ALTER TABLE usage_events DROP COLUMN cost_micros;
