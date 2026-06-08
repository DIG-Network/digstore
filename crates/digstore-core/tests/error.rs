use digstore_core::ErrorCode;

#[test]
fn error_code_discriminants_match_spec() {
    assert_eq!(ErrorCode::GeneralError as i32, -1);
    assert_eq!(ErrorCode::InvalidParameter as i32, -2);
    assert_eq!(ErrorCode::BufferTooSmall as i32, -3);
    assert_eq!(ErrorCode::NoSession as i32, -100);
    assert_eq!(ErrorCode::SessionExpired as i32, -101);
    assert_eq!(ErrorCode::AttestationFailed as i32, -102);
    assert_eq!(ErrorCode::NetworkError as i32, -200);
    assert_eq!(ErrorCode::Timeout as i32, -203);
    assert_eq!(ErrorCode::NotFound as i32, -300);
    assert_eq!(ErrorCode::ValidationFailed as i32, -301);
}

#[test]
fn error_code_from_i32_roundtrips() {
    for code in [
        ErrorCode::GeneralError,
        ErrorCode::NoSession,
        ErrorCode::Timeout,
        ErrorCode::ValidationFailed,
    ] {
        assert_eq!(ErrorCode::from_i32(code as i32), Some(code));
    }
    assert_eq!(ErrorCode::from_i32(42), None);
}
