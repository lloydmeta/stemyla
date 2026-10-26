//! The count of live read-ahead buffers and its high-water mark, which the
//! page's test of one buffer at a time reads (D9 of M6, review round 2,
//! finding 2-7).  It has its own test binary, so no other test makes a
//! buffer while it counts.

use engine::SharedParams;
use engine::readahead::{LaneFormat, ReadAhead};
use std::sync::Arc;

fn buffer() -> ReadAhead {
    let params = Arc::new(SharedParams::new(1));
    ReadAhead::new(&[LaneFormat::Pcm16], 44_100, params).expect("fits")
}

#[test]
fn live_buffers_and_their_high_water_are_counted() {
    assert_eq!(ReadAhead::live(), 0);
    let first = buffer();
    assert_eq!(ReadAhead::live(), 1);
    let second = buffer();
    assert_eq!(ReadAhead::live(), 2);
    drop(first);
    drop(second);
    assert_eq!(ReadAhead::live(), 0);
    assert_eq!(ReadAhead::take_high_water(), 2, "two at once");
    drop(buffer());
    assert_eq!(ReadAhead::take_high_water(), 1, "one at a time");
    assert_eq!(ReadAhead::take_high_water(), 0, "none since");
}
