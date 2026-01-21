use crate::cli::TanType;
use crate::comdirect::{
    extract_holder_name, format_challenge, Account, AccountTransaction, ComdirectClient,
    ComdirectCredentials, Session,
};
use crate::config::{ComdirectConfig, Config, OpConfig, YnabConfig};
use crate::op;
use crate::paths::Paths;
use crate::prompt;
use crate::state::{ReferenceEntry, State};
use crate::ynab::{AccountSummary, BudgetSummary, Transaction, YnabClient};
use anyhow::{bail, Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use tracing::{info, warn};

pub async fn run_init(paths: &Paths) -> Result<()> {
    std::fs::create_dir_all(&paths.base_dir).with_context(|| {
        format!(
            "failed to create config directory {}",
            paths.base_dir.display()
        )
    })?;

    println!("comdirect-ynab init");
    let user_id = prompt::prompt_default("comdirect user id (or 'user')", "user")?;
    let iban = prompt_required("comdirect IBAN")?;

    let client_id = prompt_op_reference("comdirect client_id reference")?;
    let client_secret = prompt_op_reference("comdirect client_secret reference")?;
    let username = prompt_op_reference("comdirect username reference")?;
    let pin = prompt_op_reference("comdirect pin reference")?;

    let ynab_token = prompt_op_reference("YNAB token reference")?;

    let op_config = OpConfig {
        service_account_token_env: "OP_SERVICE_ACCOUNT_TOKEN".to_string(),
    };

    let resolved_ynab = op::read_secret(&ynab_token, &op_config.service_account_token_env)?;
    let ynab_client = YnabClient::new(resolved_ynab)?;
    let budgets = ynab_client.list_budgets().await?;
    let budget_index = prompt_select_budget(&budgets)?;
    let budget = budgets[budget_index].clone();

    let accounts = ynab_client.list_accounts(&budget.id).await?;
    let open_accounts: Vec<AccountSummary> = accounts.into_iter().filter(|a| !a.closed).collect();
    let account_index = prompt_select_account(&open_accounts)?;
    let account = open_accounts[account_index].clone();

    let config = Config {
        comdirect: ComdirectConfig {
            user_id,
            iban,
            client_id,
            client_secret,
            username,
            pin,
            tan_method: "M_TAN".to_string(),
        },
        ynab: YnabConfig {
            token: ynab_token,
            budget_id: budget.id,
            account_id: account.id,
        },
        sync: crate::config::SyncConfig { lookback_days: 30 },
        op: op_config,
    };

    config.save(&paths.config)?;
    println!("Wrote config to {}", paths.config.display());
    println!("Next: run `comdirect-ynab sync` to import transactions.");
    Ok(())
}

pub async fn run_accounts(paths: &Paths) -> Result<()> {
    let config = Config::load(&paths.config)?;
    let ynab_token = op::read_secret(&config.ynab.token, &config.op.service_account_token_env)?;
    let ynab_client = YnabClient::new(ynab_token)?;
    let budgets = ynab_client.list_budgets().await?;
    println!("YNAB budgets:");
    for budget in &budgets {
        println!("- {} ({})", budget.name, budget.id);
    }
    if !config.ynab.budget_id.is_empty() {
        let accounts = ynab_client.list_accounts(&config.ynab.budget_id).await?;
        println!("YNAB accounts for budget {}:", config.ynab.budget_id);
        for account in accounts {
            let status = if account.closed { "closed" } else { "open" };
            println!("- {} ({}, {})", account.name, account.id, status);
        }
    }

    let comdirect = ComdirectClient::new()?;
    let credentials = resolve_comdirect_credentials(&config.comdirect, &config.op)?;
    let primary = comdirect.primary_token(&credentials).await?;
    let session = comdirect
        .session_status(&config.comdirect.user_id, &primary.access_token)
        .await?;
    if !session.session_tan_active {
        println!("comdirect session TAN inactive, run `comdirect-ynab auth`.");
        return Ok(());
    }
    let secondary = comdirect
        .secondary_token(&credentials, &primary.access_token)
        .await?;
    let accounts = list_accounts_with_fallback(
        &comdirect,
        &config.comdirect.user_id,
        &secondary.access_token,
    )
    .await?;
    println!("comdirect accounts:");
    for account in accounts {
        println!(
            "- {} ({:?}, {:?})",
            account.account_id, account.iban, account.account_type
        );
    }
    Ok(())
}

pub async fn run_auth(paths: &Paths, tan_type: Option<TanType>) -> Result<()> {
    let config = Config::load(&paths.config)?;
    let comdirect = ComdirectClient::new()?;
    let credentials = resolve_comdirect_credentials(&config.comdirect, &config.op)?;
    let primary = comdirect.primary_token(&credentials).await?;
    let session = comdirect
        .session_status(&config.comdirect.user_id, &primary.access_token)
        .await?;
    if session.session_tan_active {
        println!("Session TAN already active.");
        return Ok(());
    }

    let tan_type_value = tan_type
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| config.comdirect.tan_method.clone());
    let updated = activate_session_tan(
        &comdirect,
        &config.comdirect.user_id,
        &primary.access_token,
        &session,
        &tan_type_value,
    )
    .await?;
    if updated.session_tan_active {
        println!("Session TAN activated.");
    } else {
        println!("Session TAN activation response received, but not active.");
    }
    Ok(())
}

pub async fn run_sync(paths: &Paths) -> Result<()> {
    let config = Config::load(&paths.config)?;
    let mut state = State::load(&paths.state)?;
    state.prune(config.sync.lookback_days);
    let mut counters = state.build_counters();

    let comdirect = ComdirectClient::new()?;
    let credentials = resolve_comdirect_credentials(&config.comdirect, &config.op)?;
    let primary = comdirect.primary_token(&credentials).await?;
    let session = comdirect
        .session_status(&config.comdirect.user_id, &primary.access_token)
        .await?;
    let session = ensure_session_active_or_activate(
        &comdirect,
        &config.comdirect.user_id,
        &primary.access_token,
        session,
        &config.comdirect.tan_method,
    )
    .await?;
    if !session.session_tan_active {
        bail!("session TAN inactive after activation attempt")
    }

    let secondary = comdirect
        .secondary_token(&credentials, &primary.access_token)
        .await?;
    let accounts = list_accounts_with_fallback(
        &comdirect,
        &config.comdirect.user_id,
        &secondary.access_token,
    )
    .await?;
    let account = find_account_by_iban(&accounts, &config.comdirect.iban)?;

    let cutoff = Utc::now().date_naive() - Duration::days(config.sync.lookback_days);
    let transactions = fetch_transactions(
        &comdirect,
        &secondary.access_token,
        &account.account_id,
        cutoff,
    )
    .await?;

    let ynab_token = op::read_secret(&config.ynab.token, &config.op.service_account_token_env)?;
    let ynab_client = YnabClient::new(ynab_token)?;

    let mut pending = Vec::new();
    for tx in transactions {
        let booking_date = match tx.booking_date.as_deref() {
            Some(value) => value,
            None => {
                warn!("Skipping transaction without booking date");
                continue;
            }
        };
        let date = NaiveDate::parse_from_str(booking_date, "%Y-%m-%d")
            .with_context(|| format!("invalid booking date {}", booking_date))?;
        if date < cutoff {
            continue;
        }
        let amount_milli = amount_to_milli(&tx.amount.value)?;
        let reference_key = build_reference_key(&tx, date, amount_milli);
        let existing = state.reference_occurrences.get(&reference_key).cloned();
        let occurrence = existing
            .as_ref()
            .map(|entry| entry.occurrence)
            .unwrap_or_else(|| next_occurrence(&mut counters, date, amount_milli));
        if existing.is_some() {
            continue;
        }

        let import_id = format!("YNAB:{}:{}:{}", amount_milli, date, occurrence);
        let payee_name = pick_payee_name(&tx);
        let memo = build_memo(&tx);
        pending.push(PendingTransaction {
            transaction: Transaction {
                account_id: config.ynab.account_id.clone(),
                date: date.format("%Y-%m-%d").to_string(),
                amount: amount_milli,
                payee_name,
                memo,
                import_id,
                cleared: Some("uncleared".to_string()),
            },
            reference_key,
            reference_entry: ReferenceEntry {
                date,
                amount_milli,
                occurrence,
            },
        });
    }

    if pending.is_empty() {
        info!("No new transactions to import.");
        state.last_synced_at = Some(Utc::now());
        state.save(&paths.state)?;
        return Ok(());
    }

    info!("Importing {} new transactions.", pending.len());
    for chunk in pending.chunks(100) {
        let transactions: Vec<Transaction> =
            chunk.iter().map(|item| item.transaction.clone()).collect();
        ynab_client
            .create_transactions(&config.ynab.budget_id, &transactions)
            .await?;
        for item in chunk {
            state
                .reference_occurrences
                .insert(item.reference_key.clone(), item.reference_entry.clone());
        }
    }

    state.last_synced_at = Some(Utc::now());
    state.save(&paths.state)?;
    Ok(())
}

struct PendingTransaction {
    transaction: Transaction,
    reference_key: String,
    reference_entry: ReferenceEntry,
}

fn resolve_comdirect_credentials(
    config: &ComdirectConfig,
    op_config: &OpConfig,
) -> Result<ComdirectCredentials> {
    Ok(ComdirectCredentials {
        client_id: op::read_secret(&config.client_id, &op_config.service_account_token_env)?,
        client_secret: op::read_secret(
            &config.client_secret,
            &op_config.service_account_token_env,
        )?,
        username: op::read_secret(&config.username, &op_config.service_account_token_env)?,
        pin: op::read_secret(&config.pin, &op_config.service_account_token_env)?,
    })
}

async fn ensure_session_active_or_activate(
    comdirect: &ComdirectClient,
    user_id: &str,
    access_token: &str,
    session: Session,
    tan_method: &str,
) -> Result<Session> {
    if session.session_tan_active {
        return Ok(session);
    }
    println!("Session TAN inactive. Starting TAN challenge...");
    activate_session_tan(comdirect, user_id, access_token, &session, tan_method).await
}

async fn activate_session_tan(
    comdirect: &ComdirectClient,
    user_id: &str,
    access_token: &str,
    session: &Session,
    tan_method: &str,
) -> Result<Session> {
    let challenge = comdirect
        .validate_session(user_id, access_token, &session.identifier, Some(tan_method))
        .await?;
    println!("{}", format_challenge(&challenge));

    let tan_input = if challenge.tan_type == "P_TAN_PUSH" {
        println!("Approve the push TAN, then press Enter.");
        let _ = prompt::prompt("Press Enter")?;
        None
    } else {
        Some(prompt_required("Enter TAN")?)
    };

    comdirect
        .activate_session(
            user_id,
            access_token,
            &session.identifier,
            &challenge.id,
            tan_input.as_deref(),
        )
        .await
}

async fn fetch_transactions(
    client: &ComdirectClient,
    access_token: &str,
    account_id: &str,
    cutoff: NaiveDate,
) -> Result<Vec<AccountTransaction>> {
    let mut all = Vec::new();
    let mut paging_first = 0;
    loop {
        let page = client
            .list_transactions(access_token, account_id, paging_first)
            .await?;
        if page.values.is_empty() {
            break;
        }
        let mut reached_cutoff = false;
        let values = page.values;
        let values_len = values.len();
        for tx in values {
            if let Some(date) = tx
                .booking_date
                .as_deref()
                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
            {
                if date < cutoff {
                    reached_cutoff = true;
                }
            }
            all.push(tx);
        }
        if reached_cutoff {
            break;
        }
        paging_first += values_len as i32;
        if let Some(paging) = page.paging {
            if let Some(total) = paging.matches {
                if paging_first >= total {
                    break;
                }
            }
        }
    }
    Ok(all)
}

async fn list_accounts_with_fallback(
    comdirect: &ComdirectClient,
    user_id: &str,
    access_token: &str,
) -> Result<Vec<Account>> {
    match comdirect.list_accounts(user_id, access_token).await {
        Ok(accounts) => Ok(accounts),
        Err(error) if user_id != "user" => {
            warn!(
                "Account list failed for user_id '{}', retrying with 'user': {}",
                user_id, error
            );
            comdirect.list_accounts("user", access_token).await
        }
        Err(error) => Err(error),
    }
}

fn find_account_by_iban(accounts: &[Account], iban: &str) -> Result<Account> {
    accounts
        .iter()
        .find(|account| account.iban.as_deref() == Some(iban))
        .cloned()
        .context("no matching account for IBAN")
}

fn amount_to_milli(value: &str) -> Result<i64> {
    let decimal =
        Decimal::from_str(value).with_context(|| format!("invalid amount value {}", value))?;
    let milli = (decimal * Decimal::new(1000, 0)).round_dp(0);
    milli
        .to_i64()
        .context("failed to convert amount to milliunits")
}

fn next_occurrence(counters: &mut HashMap<String, u32>, date: NaiveDate, amount_milli: i64) -> u32 {
    let key = format!("{}|{}", date, amount_milli);
    let next = counters.get(&key).copied().unwrap_or(0) + 1;
    counters.insert(key, next);
    next
}

fn pick_payee_name(tx: &AccountTransaction) -> Option<String> {
    extract_holder_name(&tx.creditor)
        .or_else(|| extract_holder_name(&tx.debtor))
        .or_else(|| extract_holder_name(&tx.deptor))
        .or_else(|| extract_holder_name(&tx.remitter))
}

fn build_reference_key(tx: &AccountTransaction, date: NaiveDate, amount_milli: i64) -> String {
    if let Some(reference) = tx.reference.as_deref() {
        if !reference.trim().is_empty() {
            return reference.trim().to_string();
        }
    }
    let memo = tx
        .remittance_info
        .as_deref()
        .unwrap_or("")
        .replace(['\n', '\r'], " ");
    format!("{}|{}|{}", date, amount_milli, memo)
}

fn build_memo(tx: &AccountTransaction) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(info) = tx.remittance_info.as_deref() {
        let trimmed = info.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(tx_type) = tx
        .transaction_type
        .as_ref()
        .and_then(|value| value.text.as_deref())
    {
        let trimmed = tx_type.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
    }
}

fn prompt_op_reference(label: &str) -> Result<String> {
    loop {
        let value = prompt_required(label)?;
        if let Err(error) = op::validate_reference(&value) {
            println!("Invalid reference: {}", error);
            continue;
        }
        return Ok(value);
    }
}

fn prompt_required(label: &str) -> Result<String> {
    loop {
        let value = prompt::prompt(label)?;
        if value.trim().is_empty() {
            println!("Value required.");
            continue;
        }
        return Ok(value);
    }
}

fn prompt_select_budget(budgets: &[BudgetSummary]) -> Result<usize> {
    let options: Vec<DisplayBudget> = budgets.iter().map(DisplayBudget::from).collect();
    prompt::prompt_select("YNAB budget", &options)
}

fn prompt_select_account(accounts: &[AccountSummary]) -> Result<usize> {
    let options: Vec<DisplayAccount> = accounts.iter().map(DisplayAccount::from).collect();
    prompt::prompt_select("YNAB account", &options)
}

struct DisplayBudget {
    name: String,
    id: String,
}

impl From<&BudgetSummary> for DisplayBudget {
    fn from(value: &BudgetSummary) -> Self {
        Self {
            name: value.name.clone(),
            id: value.id.clone(),
        }
    }
}

impl fmt::Display for DisplayBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.id)
    }
}

struct DisplayAccount {
    name: String,
    id: String,
}

impl From<&AccountSummary> for DisplayAccount {
    fn from(value: &AccountSummary) -> Self {
        Self {
            name: value.name.clone(),
            id: value.id.clone(),
        }
    }
}

impl fmt::Display for DisplayAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comdirect::{AccountInformation, AmountValue, EnumText};

    fn sample_transaction(
        reference: Option<&str>,
        remittance: Option<&str>,
        tx_type: Option<&str>,
    ) -> AccountTransaction {
        AccountTransaction {
            reference: reference.map(|value| value.to_string()),
            booking_status: None,
            booking_date: None,
            amount: AmountValue {
                value: "0".to_string(),
                unit: None,
            },
            creditor: None,
            debtor: None,
            deptor: None,
            remitter: None,
            remittance_info: remittance.map(|value| value.to_string()),
            transaction_type: tx_type.map(|value| EnumText {
                key: None,
                text: Some(value.to_string()),
            }),
        }
    }

    #[test]
    fn amount_to_milli_handles_sign_and_scale() {
        assert_eq!(amount_to_milli("12.345").unwrap(), 12_345);
        assert_eq!(amount_to_milli("-10.10").unwrap(), -10_100);
    }

    #[test]
    fn build_reference_key_prefers_reference() {
        let tx = sample_transaction(Some(" ref-123 "), None, None);
        let date = NaiveDate::from_ymd_opt(2026, 1, 21).unwrap();
        let key = build_reference_key(&tx, date, 1000);
        assert_eq!(key, "ref-123");
    }

    #[test]
    fn build_reference_key_uses_memo_when_missing_reference() {
        let tx = sample_transaction(None, Some("hello\nworld"), None);
        let date = NaiveDate::from_ymd_opt(2026, 1, 21).unwrap();
        let key = build_reference_key(&tx, date, 1000);
        assert_eq!(key, "2026-01-21|1000|hello world");
    }

    #[test]
    fn build_memo_combines_remittance_and_type() {
        let tx = sample_transaction(None, Some("Rent"), Some("Transfer"));
        let memo = build_memo(&tx).unwrap();
        assert_eq!(memo, "Rent | Transfer");
    }

    #[test]
    fn build_memo_handles_missing_parts() {
        let tx = sample_transaction(None, None, Some("Transfer"));
        let memo = build_memo(&tx).unwrap();
        assert_eq!(memo, "Transfer");
    }

    #[test]
    fn next_occurrence_increments_per_amount_date() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 21).unwrap();
        let mut counters = HashMap::new();
        let first = next_occurrence(&mut counters, date, 1000);
        let second = next_occurrence(&mut counters, date, 1000);
        let other = next_occurrence(&mut counters, date, 2000);
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(other, 1);
    }

    #[test]
    fn pick_payee_name_prefers_creditor() {
        let creditor = AccountInformation {
            holder_name: Some("Creditor".to_string()),
            iban: None,
            bic: None,
        };
        let debtor = AccountInformation {
            holder_name: Some("Debtor".to_string()),
            iban: None,
            bic: None,
        };
        let tx = AccountTransaction {
            reference: None,
            booking_status: None,
            booking_date: None,
            amount: AmountValue {
                value: "0".to_string(),
                unit: None,
            },
            creditor: Some(creditor),
            debtor: Some(debtor),
            deptor: None,
            remitter: None,
            remittance_info: None,
            transaction_type: None,
        };
        assert_eq!(pick_payee_name(&tx), Some("Creditor".to_string()));
    }
}
