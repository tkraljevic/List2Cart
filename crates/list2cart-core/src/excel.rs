use crate::models::{ReceiptSummary, ShoppingList, Unit};
use rust_xlsxwriter::{Color, ExcelDateTime, Format, FormatAlign, Workbook, Worksheet};
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

/// A shopping list as a table to print or send: what to buy, how much, the estimated
/// price, and what is already bought. `exported_on` is the date shown at the top.
pub fn export_list_to_xlsx(
    list: &ShoppingList,
    exported_on: &str,
    output_path: &Path,
) -> Result<(), String> {
    let excel = |e: rust_xlsxwriter::XlsxError| format!("Excel greska: {e}");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Popis").map_err(excel)?;

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
    let bought_fmt = Format::new().set_font_color(Color::RGB(0x66758C));
    let center_fmt = Format::new().set_align(FormatAlign::Center);

    sheet
        .write_string_with_format(0, 0, format!("Popis za kupnju: {}", list.name), &title_fmt)
        .map_err(excel)?;
    let mut details = vec![format!("Izvezeno {exported_on}")];
    if let Some(store) = list.store_name.as_deref().filter(|s| !s.is_empty()) {
        details.push(format!("Trgovina: {store}"));
    }
    if list.shared {
        details.push("Zajednički popis".to_owned());
    }
    sheet
        .write_string(1, 0, details.join(" · "))
        .map_err(excel)?;

    let headers = [
        "Kupljeno",
        "Artikl",
        "Količina",
        "Jedinica",
        "Procjena po jedinici",
        "Procjena ukupno",
        "Barkod",
    ];
    for (col, h) in headers.iter().enumerate() {
        sheet
            .write_string_with_format(3, col as u16, *h, &header_fmt)
            .map_err(excel)?;
    }

    let mut total = 0i64;
    let mut remaining = 0i64;
    for (idx, item) in list.items.iter().enumerate() {
        let row = (idx + 4) as u32;
        let line = crate::calc::checked_line_total_cents(
            item.quantity,
            item.unit,
            item.estimated_price_cents,
        )?;
        total += line;
        if !item.checked {
            remaining += line;
        }
        let text_fmt = if item.checked {
            &bought_fmt
        } else {
            &Format::new()
        };
        sheet
            .write_string_with_format(row, 0, if item.checked { "✓" } else { "" }, &center_fmt)
            .map_err(excel)?;
        sheet
            .write_string_with_format(row, 1, &item.name, text_fmt)
            .map_err(excel)?;
        sheet.write_number(row, 2, item.quantity).map_err(excel)?;
        sheet
            .write_string(
                row,
                3,
                if item.unit == Unit::L {
                    "L"
                } else {
                    item.unit.as_str()
                },
            )
            .map_err(excel)?;
        // A price that was never estimated stays empty rather than showing 0,00 €.
        if item.estimated_price_cents > 0 {
            sheet
                .write_number_with_format(
                    row,
                    4,
                    item.estimated_price_cents as f64 / 100.0,
                    &currency_fmt,
                )
                .map_err(excel)?;
            sheet
                .write_number_with_format(row, 5, line as f64 / 100.0, &currency_fmt)
                .map_err(excel)?;
        }
        sheet
            .write_string(row, 6, item.barcode.as_deref().unwrap_or(""))
            .map_err(excel)?;
    }

    let summary = (list.items.len() + 5) as u32;
    for (offset, (label, cents)) in [("Ukupno:", total), ("Preostalo za kupiti:", remaining)]
        .into_iter()
        .enumerate()
    {
        let row = summary + offset as u32;
        sheet
            .write_string_with_format(row, 4, label, &bold_fmt)
            .map_err(excel)?;
        sheet
            .write_number_with_format(row, 5, cents as f64 / 100.0, &bold_currency_fmt)
            .map_err(excel)?;
    }

    sheet.autofit();
    workbook
        .save(output_path)
        .map_err(|e| format!("Greska spremanja Excel datoteke: {e}"))?;
    Ok(())
}

/// The local calendar day of a stored receipt time, or None if it cannot be read.
fn receipt_day(created_at: &str) -> Option<chrono::NaiveDate> {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .ok()
        .map(|t| t.with_timezone(&chrono::Local).date_naive())
}

fn write_day(
    sheet: &mut Worksheet,
    row: u32,
    col: u16,
    created_at: &str,
    date_fmt: &Format,
) -> Result<(), String> {
    use chrono::Datelike;
    let excel = |e: rust_xlsxwriter::XlsxError| format!("Excel greska: {e}");
    match receipt_day(created_at) {
        Some(day) => {
            let date =
                ExcelDateTime::from_ymd(day.year() as u16, day.month() as u8, day.day() as u8)
                    .map_err(excel)?;
            sheet
                .write_datetime_with_format(row, col, &date, date_fmt)
                .map_err(excel)?;
        }
        None => {
            sheet.write_string(row, col, created_at).map_err(excel)?;
        }
    }
    Ok(())
}

/// Every given receipt in one workbook, for following what was spent: a summary by
/// month and by store, one row per receipt, and every item with its receipt's day
/// and store. `selection` says which receipts these are ("Sve trgovine", a search).
pub fn export_receipts_to_xlsx(
    receipts: &[ReceiptSummary],
    selection: &str,
    exported_on: &str,
    output_path: &Path,
) -> Result<(), String> {
    use std::collections::BTreeMap;
    let excel = |e: rust_xlsxwriter::XlsxError| format!("Excel greska: {e}");
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
    let date_fmt = Format::new().set_num_format("d. m. yyyy.");
    let money = |cents: i64| cents as f64 / 100.0;

    let mut workbook = Workbook::new();

    // Summary: what the receipts came to, by month and by store.
    let mut by_month: BTreeMap<String, (usize, i64)> = BTreeMap::new();
    let mut by_store: BTreeMap<String, (usize, i64)> = BTreeMap::new();
    for r in receipts {
        let month = receipt_day(&r.created_at)
            .map(|d| d.format("%Y-%m").to_string())
            .unwrap_or_else(|| "nepoznato".into());
        let m = by_month.entry(month).or_default();
        m.0 += 1;
        m.1 += r.receipt_total_cents;
        let st = by_store.entry(r.store_name.clone()).or_default();
        st.0 += 1;
        st.1 += r.receipt_total_cents;
    }
    let total: i64 = receipts.iter().map(|r| r.receipt_total_cents).sum();
    let summary = workbook.add_worksheet();
    summary.set_name("Sažetak").map_err(excel)?;
    summary
        .write_string_with_format(0, 0, "Potrošnja prema računima", &title_fmt)
        .map_err(excel)?;
    summary
        .write_string(1, 0, format!("Izvezeno {exported_on} · {selection}"))
        .map_err(excel)?;
    summary
        .write_string_with_format(3, 0, "Ukupno računa:", &bold_fmt)
        .map_err(excel)?;
    summary
        .write_number(3, 1, receipts.len() as f64)
        .map_err(excel)?;
    summary
        .write_string_with_format(4, 0, "Ukupno potrošeno:", &bold_fmt)
        .map_err(excel)?;
    summary
        .write_number_with_format(4, 1, money(total), &bold_currency_fmt)
        .map_err(excel)?;
    let mut row = 6u32;
    for (heading, groups, first) in [
        ("Po mjesecu", &by_month, "Mjesec"),
        ("Po trgovini", &by_store, "Trgovina"),
    ] {
        summary
            .write_string_with_format(row, 0, heading, &bold_fmt)
            .map_err(excel)?;
        row += 1;
        for (col, h) in [first, "Računa", "Potrošeno"].iter().enumerate() {
            summary
                .write_string_with_format(row, col as u16, *h, &header_fmt)
                .map_err(excel)?;
        }
        row += 1;
        let mut rows: Vec<_> = groups.iter().collect();
        if first == "Trgovina" {
            rows.sort_by_key(|(_, (_, cents))| std::cmp::Reverse(*cents));
        }
        for (key, (count, cents)) in rows {
            // "2026-11" reads as "11/2026"; a store keeps its name.
            let label = match key.split_once('-') {
                Some((y, m)) if first == "Mjesec" => format!("{}/{y}", m.trim_start_matches('0')),
                _ => key.clone(),
            };
            summary.write_string(row, 0, label).map_err(excel)?;
            summary.write_number(row, 1, *count as f64).map_err(excel)?;
            summary
                .write_number_with_format(row, 2, money(*cents), &currency_fmt)
                .map_err(excel)?;
            row += 1;
        }
        row += 1;
    }
    summary.autofit();

    // One row per receipt.
    let sheet = workbook.add_worksheet();
    sheet.set_name("Računi").map_err(excel)?;
    let headers = [
        "Datum",
        "Trgovina",
        "Stavki",
        "Zbroj stavki",
        "Iznos računa",
        "Razlika",
    ];
    for (col, h) in headers.iter().enumerate() {
        sheet
            .write_string_with_format(0, col as u16, *h, &header_fmt)
            .map_err(excel)?;
    }
    for (i, r) in receipts.iter().enumerate() {
        let row = i as u32 + 1;
        write_day(sheet, row, 0, &r.created_at, &date_fmt)?;
        sheet.write_string(row, 1, &r.store_name).map_err(excel)?;
        sheet
            .write_number(row, 2, r.items.len() as f64)
            .map_err(excel)?;
        for (col, cents) in [
            (3, r.cart_total_cents),
            (4, r.receipt_total_cents),
            (5, r.difference_cents),
        ] {
            sheet
                .write_number_with_format(row, col, money(cents), &currency_fmt)
                .map_err(excel)?;
        }
    }
    let last = receipts.len() as u32 + 2;
    sheet
        .write_string_with_format(last, 3, "Ukupno:", &bold_fmt)
        .map_err(excel)?;
    sheet
        .write_number_with_format(last, 4, money(total), &bold_currency_fmt)
        .map_err(excel)?;
    sheet.autofit();

    // Every item, with the day and store of its receipt.
    let items = workbook.add_worksheet();
    items.set_name("Stavke").map_err(excel)?;
    let headers = [
        "Datum",
        "Trgovina",
        "Artikl",
        "Količina",
        "Jedinica",
        "Cijena po jedinici",
        "Ukupno",
        "Barkod",
    ];
    for (col, h) in headers.iter().enumerate() {
        items
            .write_string_with_format(0, col as u16, *h, &header_fmt)
            .map_err(excel)?;
    }
    let mut row = 1u32;
    for r in receipts {
        for item in &r.items {
            write_day(items, row, 0, &r.created_at, &date_fmt)?;
            items.write_string(row, 1, &r.store_name).map_err(excel)?;
            items.write_string(row, 2, &item.name).map_err(excel)?;
            items.write_number(row, 3, item.quantity).map_err(excel)?;
            items
                .write_string(
                    row,
                    4,
                    if item.unit == Unit::L {
                        "L"
                    } else {
                        item.unit.as_str()
                    },
                )
                .map_err(excel)?;
            items
                .write_number_with_format(row, 5, money(item.unit_price_cents), &currency_fmt)
                .map_err(excel)?;
            items
                .write_number_with_format(row, 6, money(item.total_cents), &currency_fmt)
                .map_err(excel)?;
            items.write_string(row, 7, &item.barcode).map_err(excel)?;
            row += 1;
        }
    }
    items.autofit();

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

    #[test]
    fn exports_a_shopping_list_with_bought_items_and_totals() {
        use crate::models::PlannedItem;
        let item = |id: i64, name: &str, quantity: f64, unit: Unit, price: i64, checked: bool| {
            PlannedItem {
                id,
                sync_id: format!("item-{id}"),
                list_id: 1,
                barcode: (id == 1).then(|| "3850102016372".to_owned()),
                name: name.to_owned(),
                quantity,
                unit,
                estimated_price_cents: price,
                checked,
                updated_at: String::new(),
            }
        };
        let list = ShoppingList {
            id: 1,
            sync_id: "list".to_owned(),
            name: "Tjedna nabavka".to_owned(),
            store_id: None,
            store_name: Some("Konzum".to_owned()),
            items: vec![
                item(1, "Lino Lada 350 g", 2.0, Unit::Kom, 199, false),
                item(2, "Jabuke", 1.5, Unit::Kg, 149, true),
                item(3, "Kruh", 1.0, Unit::Kom, 0, false),
            ],
            total_estimated_cents: 0,
            updated_at: String::new(),
            shared: true,
        };
        let out = std::env::temp_dir().join("list2cart_test_list_export.xlsx");
        export_list_to_xlsx(&list, "19. 11. 2026.", &out).expect("export list");
        let bytes = std::fs::read(&out).expect("read xlsx");
        let _ = std::fs::remove_file(&out);
        // An xlsx is a ZIP; the sheet's text sits in its shared strings.
        let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("xlsx is a zip");
        let mut archive = archive;
        let mut strings = String::new();
        std::io::Read::read_to_string(
            &mut archive
                .by_name("xl/sharedStrings.xml")
                .expect("shared strings"),
            &mut strings,
        )
        .unwrap();
        for text in [
            "Popis za kupnju: Tjedna nabavka",
            "Trgovina: Konzum",
            "Zajednički popis",
            "Lino Lada 350 g",
            "✓",
            "Preostalo za kupiti:",
        ] {
            assert!(strings.contains(text), "missing {text}");
        }
        let mut sheet = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("xl/worksheets/sheet1.xml").expect("sheet"),
            &mut sheet,
        )
        .unwrap();
        // 2 × 1,99 + 1,5 × 1,49 = 6,22 € in all; 3,98 € still to buy.
        assert!(sheet.contains("<v>6.22</v>") || sheet.contains("<v>6.2199999999999998</v>"));
        assert!(sheet.contains("<v>3.98</v>"));
    }

    fn read_part(bytes: &[u8], name: &str) -> String {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("xlsx is a zip");
        let mut text = String::new();
        std::io::Read::read_to_string(&mut archive.by_name(name).expect(name), &mut text).unwrap();
        text
    }

    #[test]
    fn exports_all_receipts_with_summary_by_month_and_store() {
        use crate::models::CartItem;
        let receipt = |id: i64, store: &str, day: &str, cents: i64| ReceiptSummary {
            id,
            sync_id: format!("r{id}"),
            store_id: 1,
            store_name: store.to_owned(),
            created_at: format!("{day}T12:00:00+00:00"),
            items: vec![CartItem {
                id: "1".to_owned(),
                barcode: "3850102016372".to_owned(),
                name: format!("Artikl {id}"),
                quantity: 1.0,
                unit: Unit::Kom,
                unit_price_cents: cents,
                total_cents: cents,
                image_data_url: None,
            }],
            cart_total_cents: cents,
            receipt_total_cents: cents,
            difference_cents: 0,
        };
        let receipts = [
            receipt(1, "Konzum", "2026-10-03", 1250),
            receipt(2, "Lidl", "2026-11-05", 830),
            receipt(3, "Konzum", "2026-11-19", 2000),
        ];
        let out = std::env::temp_dir().join("list2cart_test_receipts_export.xlsx");
        export_receipts_to_xlsx(&receipts, "Sve trgovine", "19. 11. 2026.", &out).expect("export");
        let bytes = std::fs::read(&out).expect("read");
        let _ = std::fs::remove_file(&out);
        let workbook = read_part(&bytes, "xl/workbook.xml");
        for sheet in ["Sažetak", "Računi", "Stavke"] {
            assert!(workbook.contains(sheet), "missing sheet {sheet}");
        }
        let strings = read_part(&bytes, "xl/sharedStrings.xml");
        for text in [
            "Potrošnja prema računima",
            "Sve trgovine",
            "10/2026",
            "11/2026",
            "Konzum",
            "Lidl",
            "Artikl 3",
        ] {
            assert!(strings.contains(text), "missing {text}");
        }
        // 12,50 + 8,30 + 20,00 = 40,80 € in all; Konzum 32,50 €, November 28,30 €.
        let summary = read_part(&bytes, "xl/worksheets/sheet1.xml");
        for value in ["<v>40.8</v>", "<v>32.5</v>", "<v>28.3</v>"] {
            assert!(summary.contains(value), "summary misses {value}");
        }
        // Three receipts and three items, each on its own row under the header.
        assert_eq!(
            read_part(&bytes, "xl/worksheets/sheet3.xml")
                .matches("<row ")
                .count(),
            4
        );
    }
}
