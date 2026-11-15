//! S1: the zone primitive renders ISO 8601 and finds the next local slot.

use hearth_core::Zone;

fn micros(seconds: i64) -> i64 {
    seconds * 1_000_000
}

#[test]
fn a_fixed_offset_renders_iso_8601() {
    let zone = Zone::fixed(8 * 3600);
    assert_eq!(zone.iso(micros(0)), "1970-01-01T08:00:00+08:00");
    assert_eq!(zone.iso(micros(-1)), "1970-01-01T07:59:59+08:00");
}

#[test]
fn utc_renders_an_explicit_zero_offset() {
    let zone = Zone::utc();
    assert_eq!(zone.name(), "UTC");
    assert_eq!(zone.iso(micros(0)), "1970-01-01T00:00:00+00:00");
}

#[test]
fn an_unknown_or_empty_name_falls_back_to_utc() {
    for name in ["Nowhere/Else", "", "   "] {
        let zone = Zone::resolve(name);
        assert_eq!(zone.name(), "UTC", "name was {name:?}");
        assert_eq!(zone.iso(micros(0)), "1970-01-01T00:00:00+00:00");
    }
}

#[test]
fn next_at_rolls_to_tomorrow_once_todays_slot_has_passed() {
    let zone = Zone::fixed(8 * 3600);
    let before = micros(1_767_204_000);
    assert_eq!(zone.iso(before), "2026-01-01T02:00:00+08:00");
    let next = zone.next_at(before, 3, 30);
    assert_eq!(next, micros(1_767_209_400));
    assert_eq!(zone.iso(next), "2026-01-01T03:30:00+08:00");

    let after = micros(1_767_211_200);
    assert_eq!(zone.iso(after), "2026-01-01T04:00:00+08:00");
    let next = zone.next_at(after, 3, 30);
    assert_eq!(next, micros(1_767_295_800));
    assert_eq!(zone.iso(next), "2026-01-02T03:30:00+08:00");
}

#[test]
fn local_parts_render_clock_and_date() {
    let utc = Zone::utc().local(micros(0));
    assert_eq!(utc.date(), "1970-01-01");
    assert_eq!(utc.clock(), "00:00:00");
    assert_eq!(utc.offset_seconds, 0);

    let west = Zone::fixed(-5 * 3600).local(micros(0));
    assert_eq!(west.date(), "1969-12-31");
    assert_eq!(west.clock(), "19:00:00");
    assert_eq!(west.offset_seconds, -5 * 3600);
}
