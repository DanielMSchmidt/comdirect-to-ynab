use anyhow::{bail, Context, Result};
use std::process::Command;

pub fn validate_reference(reference: &str) -> Result<()> {
    if reference.trim().starts_with("op://") {
        Ok(())
    } else {
        bail!("reference must start with op://")
    }
}

pub fn read_secret(reference: &str, token_env: &str) -> Result<String> {
    validate_reference(reference)?;
    if std::env::var(token_env).is_err() {
        bail!("missing {} environment variable", token_env);
    }
    let output = Command::new("op")
        .arg("read")
        .arg(reference)
        .output()
        .context("failed to run op read")?;
    if !output.status.success() {
        bail!(
            "op read failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let secret = String::from_utf8(output.stdout).context("op read output was not utf-8")?;
    Ok(secret.trim().to_string())
}
