use crate::models::{CartItem, ReceiptVerification, Unit};

pub const MAX_SAFE_CENTS: i64 = 9_007_199_254_740_991;

/// Calculates the exact line total in euro cents.
///
/// - For `Unit::Kom`: `quantity * unit_price_cents`
/// - For `Unit::Kg` / `Unit::L`: `quantity * unit_price_cents` (price is per kg / per litre)
/// - For `Unit::G` / `Unit::Ml`: `(quantity / 1000.0) * unit_price_cents` (price is per kg / per litre, quantity in g / ml)
pub fn calculate_line_total_cents(quantity: f64, unit: Unit, unit_price_cents: i64) -> i64 {
    checked_line_total_cents(quantity, unit, unit_price_cents).unwrap_or(0)
}

pub fn checked_line_total_cents(quantity: f64, unit: Unit, price: i64) -> Result<i64, String> {
    let millis = crate::models::qty_to_millis(quantity)?;
    if (crate::models::millis_to_qty(millis) - quantity).abs() > 0.0000001 {
        return Err("Kolicina smije imati najvise tri decimale".into());
    }
    checked_line_total_millis(millis, unit, price)
}

pub fn checked_line_total_millis(millis: i64, unit: Unit, price: i64) -> Result<i64, String> {
    if !(1..=1_000_000_000_000).contains(&millis) || !(0..=MAX_SAFE_CENTS).contains(&price) {
        return Err("Kolicina ili cijena je izvan dopustenog raspona".into());
    }
    let denominator = match unit {
        Unit::G | Unit::Ml => 1_000_000i128,
        _ => 1000,
    };
    let rounded = (i128::from(millis) * i128::from(price) + denominator / 2) / denominator;
    if rounded > i128::from(MAX_SAFE_CENTS) {
        return Err("Iznos stavke prelazi dopusteni raspon".into());
    }
    Ok(rounded as i64)
}

pub fn calculate_cart_total_cents(items: &[CartItem]) -> i64 {
    items
        .iter()
        .map(|i| calculate_line_total_cents(i.quantity, i.unit, i.unit_price_cents))
        .fold(0, i64::saturating_add)
}

pub fn verify_receipt(cart_total_cents: i64, receipt_total_cents: i64) -> ReceiptVerification {
    let difference_cents = cart_total_cents - receipt_total_cents;
    ReceiptVerification {
        cart_total_cents,
        receipt_total_cents,
        difference_cents,
        is_matched: difference_cents == 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_pieces_correctly() {
        assert_eq!(calculate_line_total_cents(2.0, Unit::Kom, 139), 278);
        assert_eq!(calculate_line_total_cents(1.0, Unit::Kom, 149), 149);
    }

    #[test]
    fn calculates_kilograms_and_grams_correctly() {
        assert_eq!(calculate_line_total_cents(1.25, Unit::Kg, 130), 163);
        assert_eq!(calculate_line_total_cents(500.0, Unit::G, 130), 65);
        assert_eq!(calculate_line_total_cents(1.5, Unit::L, 200), 300);
        assert_eq!(calculate_line_total_cents(250.0, Unit::Ml, 200), 50);
    }

    #[test]
    fn handles_zero_or_negative_safely() {
        assert_eq!(calculate_line_total_cents(0.0, Unit::Kom, 100), 0);
        assert_eq!(calculate_line_total_cents(-2.0, Unit::Kg, 100), 0);
        assert_eq!(calculate_line_total_cents(2.0, Unit::Kom, -50), 0);
    }

    #[test]
    fn verifies_receipt_exact_match() {
        let v = verify_receipt(710, 710);
        assert!(v.is_matched);
        assert_eq!(v.difference_cents, 0);
    }

    #[test]
    fn verifies_receipt_mismatch() {
        let v = verify_receipt(710, 750);
        assert!(!v.is_matched);
        assert_eq!(v.difference_cents, -40);
    }

    #[test]
    fn integer_rounding_is_exact_and_overflow_is_an_error() {
        assert_eq!(
            checked_line_total_cents(1.005, Unit::Kom, 100).unwrap(),
            101
        );
        assert_eq!(checked_line_total_millis(1250, Unit::Kg, 130).unwrap(), 163);
        assert!(checked_line_total_cents(2.0, Unit::Kom, i64::MAX).is_err());
        assert!(checked_line_total_cents(f64::INFINITY, Unit::Kg, 100).is_err());
        assert!(checked_line_total_cents(0.0001, Unit::Kg, 100).is_err());
    }
}
