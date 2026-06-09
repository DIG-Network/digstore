use digstore_guest::request::ValidityWindow;
use digstore_guest::temporal::within_window;

#[test]
fn none_window_is_always_valid() {
    assert!(within_window(&None, 12345));
}

#[test]
fn inside_window_is_valid() {
    let w = Some(ValidityWindow {
        not_before: 100,
        not_after: 200,
    });
    assert!(within_window(&w, 100));
    assert!(within_window(&w, 150));
    assert!(within_window(&w, 200));
}

#[test]
fn outside_window_is_invalid() {
    let w = Some(ValidityWindow {
        not_before: 100,
        not_after: 200,
    });
    assert!(!within_window(&w, 99));
    assert!(!within_window(&w, 201));
}
