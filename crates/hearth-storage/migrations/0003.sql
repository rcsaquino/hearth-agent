CREATE INDEX memory_items_active_standing ON memory_items(standing)
    WHERE status = 'active' AND standing <> 'none';

CREATE INDEX memory_items_promotable ON memory_items(importance)
    WHERE status = 'active' AND importance >= 85;
