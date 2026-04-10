use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

/// A PayPal transaction parsed from CSV export.
#[derive(Debug, Clone)]
pub struct PaypalTransaction {
    pub date: NaiveDate,
    pub name: String,
    pub amount_milli: i64,
}

/// Lookup table: (amount_milli, date) -> list of merchant names.
pub type PaypalLookup = HashMap<(i64, NaiveDate), Vec<String>>;

/// Parse a PayPal CSV export file into a lookup table.
/// Auto-detects column positions from headers.
pub fn parse_csv(path: &Path) -> Result<Vec<PaypalTransaction>> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .with_context(|| format!("failed to open {}", path.display()))?;

    let headers = reader.headers().context("missing CSV headers")?.clone();
    let col = detect_columns(&headers)?;

    let mut transactions = Vec::new();
    for result in reader.records() {
        let record = result.context("failed to read CSV row")?;
        let name = record.get(col.name).unwrap_or("").trim().to_string();
        if name.is_empty() {
            continue;
        }
        let date = match parse_date(record.get(col.date).unwrap_or("")) {
            Some(d) => d,
            None => continue,
        };
        let amount_milli = match parse_amount(record.get(col.gross).unwrap_or("")) {
            Some(a) => a,
            None => continue,
        };
        transactions.push(PaypalTransaction {
            date,
            name,
            amount_milli,
        });
    }
    Ok(transactions)
}

/// Build a lookup table from parsed PayPal transactions.
pub fn build_lookup(transactions: &[PaypalTransaction]) -> PaypalLookup {
    let mut lookup = PaypalLookup::new();
    for tx in transactions {
        lookup
            .entry((tx.amount_milli, tx.date))
            .or_default()
            .push(tx.name.clone());
    }
    lookup
}

/// Try to match a Comdirect transaction against the PayPal lookup.
/// Returns the merchant name if a unique match is found.
pub fn match_transaction(
    lookup: &PaypalLookup,
    amount_milli: i64,
    date: NaiveDate,
) -> Option<String> {
    for offset in [0, -1, 1] {
        let check_date = date + Duration::days(offset);
        if let Some(names) = lookup.get(&(amount_milli, check_date)) {
            if names.len() == 1 {
                return Some(names[0].clone());
            }
        }
    }
    None
}

pub const DOWNLOAD_URL: &str = "https://www.paypal.com/reports/dlog";

struct ColumnPositions {
    date: usize,
    name: usize,
    gross: usize,
}

fn detect_columns(headers: &csv::StringRecord) -> Result<ColumnPositions> {
    let mut date_col = None;
    let mut name_col = None;
    let mut gross_col = None;

    for (i, header) in headers.iter().enumerate() {
        let h = header.trim().trim_start_matches('\u{feff}').to_lowercase();
        if date_col.is_none() && matches!(h.as_str(), "datum" | "date") {
            date_col = Some(i);
        }
        if name_col.is_none() && matches!(h.as_str(), "name" | "empfänger" | "recipient") {
            name_col = Some(i);
        }
        if gross_col.is_none()
            && matches!(
                h.as_str(),
                "brutto" | "gross" | "bruttobetrag" | "gross amount"
            )
        {
            gross_col = Some(i);
        }
    }

    let date = date_col.context("CSV missing date column (expected 'Datum' or 'Date')")?;
    let name = name_col.context("CSV missing name column (expected 'Name')")?;
    let gross =
        gross_col.context("CSV missing amount column (expected 'Brutto' or 'Gross')")?;

    Ok(ColumnPositions { date, name, gross })
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    // Try DD.MM.YYYY (German)
    NaiveDate::parse_from_str(s, "%d.%m.%Y")
        // Try DD/MM/YYYY
        .or_else(|_| NaiveDate::parse_from_str(s, "%d/%m/%Y"))
        // Try MM/DD/YYYY (US)
        .or_else(|_| NaiveDate::parse_from_str(s, "%m/%d/%Y"))
        // Try YYYY-MM-DD (ISO)
        .or_else(|_| NaiveDate::parse_from_str(s, "%Y-%m-%d"))
        .ok()
}

fn parse_amount(s: &str) -> Option<i64> {
    let s = s.trim().replace('\u{a0}', ""); // remove non-breaking spaces
    // Handle German format: "1.234,56" -> "1234.56"
    let normalized = if s.contains(',') && (s.rfind(',') > s.rfind('.')) {
        s.replace('.', "").replace(',', ".")
    } else {
        s.replace(',', "")
    };
    let normalized = normalized.replace(' ', "");
    let decimal = Decimal::from_str(&normalized).ok()?;
    let milli = (decimal * Decimal::new(1000, 0)).round_dp(0);
    milli.to_i64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_date_german_format() {
        assert_eq!(
            parse_date("15.03.2026"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 15).unwrap())
        );
    }

    #[test]
    fn parse_date_iso_format() {
        assert_eq!(
            parse_date("2026-03-15"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 15).unwrap())
        );
    }

    #[test]
    fn parse_amount_german_format() {
        assert_eq!(parse_amount("-1.234,56"), Some(-1_234_560));
        assert_eq!(parse_amount("25,50"), Some(25_500));
    }

    #[test]
    fn parse_amount_english_format() {
        assert_eq!(parse_amount("-1234.56"), Some(-1_234_560));
        assert_eq!(parse_amount("25.50"), Some(25_500));
    }

    #[test]
    fn match_transaction_exact_date() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let mut lookup = PaypalLookup::new();
        lookup.insert((-25500, date), vec!["Amazon.de".to_string()]);

        assert_eq!(
            match_transaction(&lookup, -25500, date),
            Some("Amazon.de".to_string())
        );
    }

    #[test]
    fn match_transaction_adjacent_day() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let prev = NaiveDate::from_ymd_opt(2026, 3, 14).unwrap();
        let mut lookup = PaypalLookup::new();
        lookup.insert((-25500, prev), vec!["Amazon.de".to_string()]);

        assert_eq!(
            match_transaction(&lookup, -25500, date),
            Some("Amazon.de".to_string())
        );
    }

    #[test]
    fn match_transaction_ambiguous() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let mut lookup = PaypalLookup::new();
        lookup.insert(
            (-25500, date),
            vec!["Amazon.de".to_string(), "eBay".to_string()],
        );

        assert_eq!(match_transaction(&lookup, -25500, date), None);
    }

    #[test]
    fn match_transaction_no_match() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let lookup = PaypalLookup::new();
        assert_eq!(match_transaction(&lookup, -25500, date), None);
    }

    #[test]
    fn parse_csv_from_string() {
        let csv_data = "Datum,Name,Brutto,Währung\n\
                        15.03.2026,Amazon.de,\"-25,50\",EUR\n\
                        16.03.2026,Netflix,\"-9,99\",EUR\n";
        let dir = std::env::temp_dir();
        let path = dir.join("test_paypal.csv");
        std::fs::write(&path, csv_data).unwrap();

        let transactions = parse_csv(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(transactions.len(), 2);
        assert_eq!(transactions[0].name, "Amazon.de");
        assert_eq!(transactions[0].amount_milli, -25500);
        assert_eq!(
            transactions[0].date,
            NaiveDate::from_ymd_opt(2026, 3, 15).unwrap()
        );
        assert_eq!(transactions[1].name, "Netflix");
    }

    #[test]
    fn detect_columns_handles_bom() {
        let record = csv::StringRecord::from(vec!["\u{feff}Datum", "Name", "Brutto"]);
        let cols = detect_columns(&record).unwrap();
        assert_eq!(cols.date, 0);
        assert_eq!(cols.name, 1);
        assert_eq!(cols.gross, 2);
    }
}
