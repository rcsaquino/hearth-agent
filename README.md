# Hearth

Hearth is a single-user personal agent that lives in Telegram and keeps its own durable record in a local SQLite database. It is written in Rust. The full behavior and design are defined in `SPEC.md`; the storage columns are defined in `SCHEMA.md`, with migration `0001` as the executable authority.
