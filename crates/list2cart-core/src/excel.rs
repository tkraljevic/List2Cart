use crate::models::ReceiptSummary;
use rust_xlsxwriter::{Color, Format, FormatAlign, Workbook};
use std::path::Path;

pub fn export_receipt_to_xlsx(receipt: &ReceiptSummary, output_path: &Path) -> Result<(), String> {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet
        .set_name("Racun")
        .map_err(|e| format!("Excel naziv lista greska: {e}"))?;

    let title_fmt = Format::new()
        .set_bold()
        .set_font_size(14)
        .set_font_color(Color::RGB(0x182B49));

    let header_fmt = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xEAF1FA))
        .set_font_color(Color::RGB(0x1768D1))
        .set_align(FormatAlign::Center);

    let currency_fmt = Format::new().set_num_format("#,##0.00 \"€\"");
    let bold_currency_fmt = Format::new().set_bold().set_num_format("#,##0.00 \"€\"");
    let bold_fmt = Format::new().set_bold();

    sheet
        .write_string_with_format(
            0,
            0,
            format!("List2Cart Racun — {}", receipt.store_name),
            &title_fmt,
        )
        .map_err(|e| format!("Excel greska: {e}"))?;

    sheet
        .write_string(1, 0, format!("Datum: {}", receipt.created_at))
        .map_err(|e| format!("Excel greska: {e}"))?;

    let headers = [
        "Barkod",
        "Naziv artikla",
        "Kolicina",
        "Jedinica",
        "Cijena / jed",
        "Ukupno",
    ];
    for (col, h) in headers.iter().enumerate() {
        sheet
            .write_string_with_format(3, col as u16, *h, &header_fmt)
            .map_err(|e| format!("Excel greska zaglavlja: {e}"))?;
    }

    for (idx, item) in receipt.items.iter().enumerate() {
        let row = (idx + 4) as u32;
        sheet
            .write_string(row, 0, &item.barcode)
            .map_err(|e| format!("Excel greska: {e}"))?;
        sheet
            .write_string(row, 1, &item.name)
            .map_err(|e| format!("Excel greska: {e}"))?;
        sheet
            .write_number(row, 2, item.quantity)
            .map_err(|e| format!("Excel greska: {e}"))?;
        sheet
            .write_string(row, 3, item.unit.as_str())
            .map_err(|e| format!("Excel greska: {e}"))?;
        sheet
            .write_number_with_format(row, 4, item.unit_price_cents as f64 / 100.0, &currency_fmt)
            .map_err(|e| format!("Excel greska: {e}"))?;
        sheet
            .write_number_with_format(row, 5, item.total_cents as f64 / 100.0, &currency_fmt)
            .map_err(|e| format!("Excel greska: {e}"))?;
    }

    let summary_row = (receipt.items.len() + 5) as u32;
    sheet
        .write_string_with_format(summary_row, 4, "Ukupno u kosarici:", &bold_fmt)
        .map_err(|e| format!("Excel greska: {e}"))?;
    sheet
        .write_number_with_format(
            summary_row,
            5,
            receipt.cart_total_cents as f64 / 100.0,
            &bold_currency_fmt,
        )
        .map_err(|e| format!("Excel greska: {e}"))?;

    sheet
        .write_string_with_format(summary_row + 1, 4, "Iznos na racunu:", &bold_fmt)
        .map_err(|e| format!("Excel greska: {e}"))?;
    sheet
        .write_number_with_format(
            summary_row + 1,
            5,
            receipt.receipt_total_cents as f64 / 100.0,
            &bold_currency_fmt,
        )
        .map_err(|e| format!("Excel greska: {e}"))?;

    sheet
        .write_string_with_format(summary_row + 2, 4, "Razlika:", &bold_fmt)
        .map_err(|e| format!("Excel greska: {e}"))?;
    sheet
        .write_number_with_format(
            summary_row + 2,
            5,
            receipt.difference_cents as f64 / 100.0,
            &bold_currency_fmt,
        )
        .map_err(|e| format!("Excel greska: {e}"))?;

    sheet.autofit();
    workbook
        .save(output_path)
        .map_err(|e| format!("Greska spremanja Excel datoteke: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CartItem, Unit};

    #[test]
    fn exports_valid_xlsx_file() {
        let receipt = ReceiptSummary {
            id: 1,
            sync_id: "test-uuid".to_owned(),
            store_id: 1,
            store_name: "Super Konzum".to_owned(),
            created_at: "2026-11-17T00:00:00Z".to_owned(),
            items: vec![CartItem {
                id: "1".to_owned(),
                barcode: "3850102123".to_owned(),
                name: "Barilla Spageti 500g".to_owned(),
                quantity: 2.0,
                unit: Unit::Kom,
                unit_price_cents: 149,
                total_cents: 298,
                image_data_url: None,
            }],
            cart_total_cents: 298,
            receipt_total_cents: 298,
            difference_cents: 0,
        };

        let out = std::env::temp_dir().join("list2cart_test_export.xlsx");
        export_receipt_to_xlsx(&receipt, &out).expect("export xlsx");
        assert!(out.exists());
        let meta = std::fs::metadata(&out).expect("meta");
        assert!(meta.len() > 1000);
        let _ = std::fs::remove_file(out);
    }
}
