use anyhow::{bail, Context, Result};
use std::fmt::Display;
use std::io::{self, Write};

pub fn prompt(text: &str) -> Result<String> {
    print!("{}: ", text);
    io::stdout().flush().context("failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read input")?;
    Ok(input.trim().to_string())
}

pub fn prompt_default(text: &str, default: &str) -> Result<String> {
    print!("{} [{}]: ", text, default);
    io::stdout().flush().context("failed to flush stdout")?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read input")?;
    let value = input.trim();
    if value.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(value.to_string())
    }
}

pub fn prompt_select<T: Display>(title: &str, items: &[T]) -> Result<usize> {
    if items.is_empty() {
        bail!("no items to select for {}", title);
    }
    println!("{}:", title);
    for (index, item) in items.iter().enumerate() {
        println!("  {}) {}", index + 1, item);
    }
    loop {
        let selection = prompt("Select number")?;
        let number: usize = selection.parse().context("selection must be a number")?;
        if number == 0 || number > items.len() {
            println!("Selection out of range. Try again.");
            continue;
        }
        return Ok(number - 1);
    }
}
