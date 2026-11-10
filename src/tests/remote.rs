use super::*;

/// The protocol enum carries values serde never deserializes (the skipped
/// `None` marker) and floats JSON cannot carry (non-finite). The decoder must
/// refuse all of them and keep a finite float a real number.
#[test]
fn the_hrana_decoder_refuses_absent_and_non_finite_values() {
    assert!(matches!(
        decode_value(proto::Value::Float { value: f64::NAN }),
        Err(Error::Database(_))
    ));
    assert!(matches!(
        decode_value(proto::Value::Float {
            value: f64::NEG_INFINITY
        }),
        Err(Error::Database(_))
    ));
    assert!(matches!(
        decode_value(proto::Value::None),
        Err(Error::Database(_))
    ));
    assert!(matches!(
        decode_value(proto::Value::Float { value: 1.5 }),
        Ok(Value::Real(value)) if value == 1.5
    ));
}
