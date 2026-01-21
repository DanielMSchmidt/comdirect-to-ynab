use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://api.ynab.com/v1";

#[derive(Debug, Clone)]
pub struct YnabClient {
    http: Client,
    token: String,
}

#[derive(Debug, Deserialize)]
pub struct YnabResponse<T> {
    pub data: T,
}

#[derive(Debug, Deserialize)]
pub struct BudgetList {
    pub budgets: Vec<BudgetSummary>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BudgetSummary {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct AccountList {
    pub accounts: Vec<AccountSummary>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AccountSummary {
    pub id: String,
    pub name: String,
    pub closed: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct Transaction {
    pub account_id: String,
    pub date: String,
    pub amount: i64,
    pub payee_name: Option<String>,
    pub memo: Option<String>,
    pub import_id: String,
    pub cleared: Option<String>,
}

#[derive(Debug, Serialize)]
struct TransactionPayload {
    pub transactions: Vec<Transaction>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct TransactionResponse {
    pub transaction_ids: Option<Vec<String>>,
    pub duplicate_import_ids: Option<Vec<String>>,
}

impl YnabClient {
    pub fn new(token: String) -> Result<Self> {
        let http = Client::builder()
            .user_agent("comdirect-ynab/0.1")
            .build()
            .context("failed to build ynab http client")?;
        Ok(Self { http, token })
    }

    pub async fn list_budgets(&self) -> Result<Vec<BudgetSummary>> {
        let url = format!("{}/budgets", API_BASE);
        let response = self
            .http
            .get(url)
            .headers(self.headers())
            .send()
            .await
            .context("failed to list budgets")?
            .error_for_status()
            .context("budget list request failed")?;
        let data: YnabResponse<BudgetList> =
            response.json().await.context("invalid budget response")?;
        Ok(data.data.budgets)
    }

    pub async fn list_accounts(&self, budget_id: &str) -> Result<Vec<AccountSummary>> {
        let url = format!("{}/budgets/{}/accounts", API_BASE, budget_id);
        let response = self
            .http
            .get(url)
            .headers(self.headers())
            .send()
            .await
            .context("failed to list accounts")?
            .error_for_status()
            .context("account list request failed")?;
        let data: YnabResponse<AccountList> =
            response.json().await.context("invalid account response")?;
        Ok(data.data.accounts)
    }

    pub async fn create_transactions(
        &self,
        budget_id: &str,
        transactions: &[Transaction],
    ) -> Result<TransactionResponse> {
        let url = format!("{}/budgets/{}/transactions", API_BASE, budget_id);
        let payload = TransactionPayload {
            transactions: transactions.to_vec(),
        };
        let response = self
            .http
            .post(url)
            .headers(self.headers())
            .json(&payload)
            .send()
            .await
            .context("failed to create transactions")?
            .error_for_status()
            .context("transaction create request failed")?;
        let data: YnabResponse<TransactionResponse> = response
            .json()
            .await
            .context("invalid transaction response")?;
        Ok(data.data)
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let value = format!("Bearer {}", self.token);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&value).unwrap());
        headers
    }
}
