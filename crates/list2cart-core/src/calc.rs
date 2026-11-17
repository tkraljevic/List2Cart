use crate::models::{CartItem, ReceiptVerification, Unit};

/// Calculates the exact line total in euro cents.
///
/// - For `Unit::Kom`: `quantity * unit_price_cents`
/// - For `Unit::Kg` / `Unit::L`: `quantity * unit_price_cents` (price is per kg / per litre)
/// - For `Unit::G` / `Unit::Ml`: `(quantity / 1000.0) * unit_price_cents` (price is per kg / per litre, quantity in g / ml)
pub fn calculate_line_total_cents(quantity: f64, unit: Unit, unit_price_cents: i64) -> i64 {
    if quantity <= 0.0 || unit_price_cents <= 0 {
        return 0;
    }
    let effective_qty = match unit {
        Unit::Kom | Unit::Kg | Unit::L => quantity,
        Unit::G | Unit::Ml => quantity / 1000.0,
    };
    (effective_qty * unit_price_cents as f64).round() as i64
}

pub fn calculate_cart_total_cents(items: &[CartItem]) -> i64 {
    items.iter().map(|i| i.total_cents).sum()
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
}
