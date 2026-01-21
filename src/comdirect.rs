use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

const API_BASE: &str = "https://api.comdirect.de/api";
const AUTH_URL: &str = "https://api.comdirect.de/oauth/token";

#[derive(Debug, Clone)]
pub struct ComdirectCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub username: String,
    pub pin: String,
}

#[derive(Debug, Clone)]
pub struct ComdirectClient {
    http: Client,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub scope: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Session {
    pub identifier: String,
    #[serde(rename = "sessionTanActive")]
    pub session_tan_active: bool,
    #[serde(rename = "activated2FA")]
    pub activated_2fa: bool,
}

#[derive(Debug, Deserialize)]
pub struct TanChallenge {
    pub id: String,
    #[serde(rename = "typ")]
    pub tan_type: String,
    pub challenge: Option<String>,
    #[serde(rename = "availableTypes")]
    pub available_types: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct AccountBalanceList {
    pub values: Vec<AccountBalance>,
}

#[derive(Debug, Deserialize)]
pub struct AccountBalance {
    pub account: Option<Account>,
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct Account {
    #[serde(rename = "accountId")]
    pub account_id: String,
    #[serde(rename = "accountDisplayId")]
    pub account_display_id: Option<String>,
    pub iban: Option<String>,
    #[serde(rename = "accountType")]
    pub account_type: Option<EnumText>,
}

#[derive(Debug, Deserialize)]
pub struct AccountTransactionList {
    pub values: Vec<AccountTransaction>,
    pub paging: Option<PagingInfo>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PagingInfo {
    pub index: Option<i32>,
    pub matches: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AccountTransaction {
    pub reference: Option<String>,
    #[serde(rename = "bookingStatus")]
    pub booking_status: Option<String>,
    #[serde(rename = "bookingDate")]
    pub booking_date: Option<String>,
    pub amount: AmountValue,
    pub creditor: Option<AccountInformation>,
    pub debtor: Option<AccountInformation>,
    #[serde(rename = "deptor")]
    pub deptor: Option<AccountInformation>,
    pub remitter: Option<AccountInformation>,
    #[serde(rename = "remittanceInfo")]
    pub remittance_info: Option<String>,
    #[serde(rename = "transactionType")]
    pub transaction_type: Option<EnumText>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AmountValue {
    pub value: String,
    pub unit: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AccountInformation {
    #[serde(rename = "holderName")]
    pub holder_name: Option<String>,
    pub iban: Option<String>,
    pub bic: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct EnumText {
    pub key: Option<String>,
    pub text: Option<String>,
}

impl ComdirectClient {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            .user_agent("comdirect-ynab/0.1")
            .build()
            .context("failed to build comdirect http client")?;
        Ok(Self { http })
    }

    pub async fn primary_token(&self, creds: &ComdirectCredentials) -> Result<TokenResponse> {
        let mut params = Vec::new();
        params.push(("client_id", creds.client_id.as_str()));
        params.push(("client_secret", creds.client_secret.as_str()));
        params.push(("grant_type", "password"));
        params.push(("username", creds.username.as_str()));
        params.push(("password", creds.pin.as_str()));

        let response = self
            .http
            .post(AUTH_URL)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await
            .context("failed to request primary token")?
            .error_for_status()
            .context("primary token request failed")?;
        Ok(response
            .json()
            .await
            .context("invalid primary token response")?)
    }

    pub async fn secondary_token(
        &self,
        creds: &ComdirectCredentials,
        primary_access_token: &str,
    ) -> Result<TokenResponse> {
        let mut params = Vec::new();
        params.push(("client_id", creds.client_id.as_str()));
        params.push(("client_secret", creds.client_secret.as_str()));
        params.push(("grant_type", "cd_secondary"));
        params.push(("token", primary_access_token));

        let response = self
            .http
            .post(AUTH_URL)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await
            .context("failed to request secondary token")?
            .error_for_status()
            .context("secondary token request failed")?;
        Ok(response
            .json()
            .await
            .context("invalid secondary token response")?)
    }

    pub async fn session_status(&self, user_id: &str, access_token: &str) -> Result<Session> {
        let url = format!("{}/session/clients/{}/v1/sessions", API_BASE, user_id);
        let response = self
            .http
            .get(url)
            .headers(self.session_headers(access_token)?)
            .send()
            .await
            .context("failed to fetch session")?
            .error_for_status()
            .context("session request failed")?;
        let sessions: Vec<Session> = response.json().await.context("invalid session response")?;
        sessions.into_iter().next().context("no session returned")
    }

    pub async fn validate_session(
        &self,
        user_id: &str,
        access_token: &str,
        session_id: &str,
        tan_type: Option<&str>,
    ) -> Result<TanChallenge> {
        let url = format!(
            "{}/session/clients/{}/v1/sessions/{}/validate",
            API_BASE, user_id, session_id
        );
        let mut headers = self.session_headers(access_token)?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(tan_type) = tan_type {
            let header_value = json!({"typ": tan_type}).to_string();
            headers.insert(
                "x-once-authentication-info",
                HeaderValue::from_str(&header_value).context("invalid tan type header value")?,
            );
        }

        let body = json!({
            "identifier": session_id,
            "sessionTanActive": true,
            "activated2FA": true
        });
        let response = self
            .http
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .context("failed to validate session")?
            .error_for_status()
            .context("session validate failed")?;

        let header = response
            .headers()
            .get("x-once-authentication-info")
            .context("missing x-once-authentication-info header")?
            .to_str()
            .context("invalid x-once-authentication-info header")?;
        let challenge = serde_json::from_str::<TanChallenge>(header)
            .context("invalid x-once-authentication-info json")?;
        Ok(challenge)
    }

    pub async fn activate_session(
        &self,
        user_id: &str,
        access_token: &str,
        session_id: &str,
        challenge_id: &str,
        tan: Option<&str>,
    ) -> Result<Session> {
        let url = format!(
            "{}/session/clients/{}/v1/sessions/{}",
            API_BASE, user_id, session_id
        );
        let mut headers = self.session_headers(access_token)?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let header_value = json!({"id": challenge_id}).to_string();
        headers.insert(
            "x-once-authentication-info",
            HeaderValue::from_str(&header_value).context("invalid challenge header")?,
        );
        if let Some(tan) = tan {
            headers.insert(
                "x-once-authentication",
                HeaderValue::from_str(tan).context("invalid tan header")?,
            );
        }

        let body = json!({
            "identifier": session_id,
            "sessionTanActive": true,
            "activated2FA": true
        });
        let response = self
            .http
            .patch(url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .context("failed to activate session")?
            .error_for_status()
            .context("session activation failed")?;
        Ok(response
            .json()
            .await
            .context("invalid activation response")?)
    }

    pub async fn list_accounts(&self, user_id: &str, access_token: &str) -> Result<Vec<Account>> {
        let url = format!(
            "{}/banking/clients/{}/v2/accounts/balances",
            API_BASE, user_id
        );
        let response = self
            .http
            .get(url)
            .headers(self.api_headers(access_token)?)
            .send()
            .await
            .context("failed to list accounts")?;
        let response = error_for_status_with_body(response, "account list request failed").await?;
        let list: AccountBalanceList = response
            .json()
            .await
            .context("invalid account list response")?;
        let mut accounts = Vec::new();
        for item in list.values {
            if let Some(mut account) = item.account {
                if account.account_id.is_empty() {
                    if let Some(account_id) = item.account_id.as_deref() {
                        account.account_id = account_id.to_string();
                    }
                }
                accounts.push(account);
            } else if let Some(account_id) = item.account_id {
                accounts.push(Account {
                    account_id,
                    account_display_id: None,
                    iban: None,
                    account_type: None,
                });
            }
        }
        if accounts.is_empty() {
            bail!("account list returned no accounts")
        }
        Ok(accounts)
    }

    pub async fn list_transactions(
        &self,
        access_token: &str,
        account_id: &str,
        paging_first: i32,
    ) -> Result<AccountTransactionList> {
        let url = format!(
            "{}/banking/v1/accounts/{}/transactions",
            API_BASE, account_id
        );
        let params = vec![
            ("transactionState".to_string(), "BOOKED".to_string()),
            (
                "transactionDirection".to_string(),
                "CREDIT_AND_DEBIT".to_string(),
            ),
            ("paging-first".to_string(), paging_first.to_string()),
        ];
        let response = self
            .http
            .get(url)
            .headers(self.api_headers(access_token)?)
            .query(&params)
            .send()
            .await
            .context("failed to list transactions")?;
        let response = error_for_status_with_body(response, "transaction request failed").await?;
        Ok(response
            .json()
            .await
            .context("invalid transactions response")?)
    }

    fn api_headers(&self, access_token: &str) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let value = format!("Bearer {}", access_token);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&value).context("invalid authorization header")?,
        );
        headers.insert("x-http-request-info", request_info_header()?);
        Ok(headers)
    }

    fn session_headers(&self, access_token: &str) -> Result<HeaderMap> {
        self.api_headers(access_token)
    }
}

fn request_info_header() -> Result<HeaderValue> {
    let session_id = Uuid::new_v4().to_string();
    let request_id = Uuid::new_v4().to_string();
    let payload = json!({
        "clientRequestId": {
            "sessionId": session_id,
            "requestId": request_id
        }
    });
    let value = payload.to_string();
    HeaderValue::from_str(&value).context("invalid x-http-request-info header")
}

async fn error_for_status_with_body(
    response: reqwest::Response,
    context: &str,
) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    bail!("{} ({}): {}", context, status, body)
}

pub fn extract_holder_name(info: &Option<AccountInformation>) -> Option<String> {
    info.as_ref()
        .and_then(|info| info.holder_name.clone())
        .filter(|value| !value.trim().is_empty())
}

pub fn format_challenge(challenge: &TanChallenge) -> String {
    let mut details = Vec::new();
    if let Some(challenge_value) = &challenge.challenge {
        details.push(format!("challenge: {}", challenge_value));
    }
    if let Some(types) = &challenge.available_types {
        if !types.is_empty() {
            details.push(format!("available: {}", types.join(", ")));
        }
    }
    if details.is_empty() {
        format!("TAN type: {}", challenge.tan_type)
    } else {
        format!("TAN type: {} ({})", challenge.tan_type, details.join("; "))
    }
}
