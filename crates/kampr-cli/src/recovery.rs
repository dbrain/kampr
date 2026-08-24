use crate::report::{self, Local};
use anyhow::{Result, bail};
use std::io::{IsTerminal, Write};
use std::process::Command;

/// Printed at `kampr init` and again every time a redemption mints a replacement. It is the only
/// moment the code exists in the clear anywhere, which is the whole message.
pub fn print_new_code(code: &str) {
    println!();
    println!("  RECOVERY CODE   {code}");
    println!();
    println!("  Write it down now, on paper. It is shown here once and never again — the node");
    println!("  keeps only a slow one-way digest, so nobody, this console included, can read it");
    println!("  back. If you lose every paired device this is the only way into this node; lose");
    println!("  it as well and the way in is a shell on this machine, or nothing.");
    println!("  Anyone who holds it can enrol a device that types into every terminal here, so");
    println!("  keep it where you would keep a house key rather than in the password manager");
    println!("  you sign into from the phone this protects.");
    println!("  Spending it is `kampr recover`: one full-access device, and a fresh code to");
    println!("  replace this one.");
}

pub async fn issue(local: &Local) -> Result<()> {
    let existing = local.auth.has_recovery().await?;
    let code = local.auth.issue_recovery().await?;
    if existing {
        println!("The code issued before this one no longer works.");
    }
    print_new_code(&code);
    Ok(())
}

/// The way back in when every device is gone and all that is left is a shell and the paper.
pub async fn redeem(local: &Local, device_name: &str) -> Result<()> {
    println!(
        "Kampr recovery — {} ({})",
        local.config.node_name, local.config.node_id
    );
    if !local.auth.has_recovery().await? {
        println!();
        println!("  This node has no recovery code. Issue one with `kampr recover --new`.");
        bail!("no recovery code is set");
    }
    let Some(code) = prompt_for_code()? else {
        bail!("no code given");
    };

    let recovered = match local
        .auth
        .redeem_recovery(&code, device_name, None, "console")
        .await
    {
        Ok(recovered) => recovered,
        Err(kampr_auth::AuthError::RateLimited) => {
            println!("  Too many attempts. Wait a minute and try again.");
            bail!("rate limited");
        }
        Err(e) => {
            println!("  That code is not valid. It is 20 characters in five groups; the dashes,");
            println!("  the case and any spaces do not matter.");
            bail!("{e}");
        }
    };

    let url = local.config.origin();
    println!();
    println!("  device      {} — full access", recovered.enrolment.device.name);
    println!("  token       {}", recovered.enrolment.token);
    println!();
    println!("  Open {url} and paste the token into the code box.");
    println!("  Anything watching this screen now has that token. If this pane is shared, revoke");
    println!("  the device from `kampr setup` and recover again from a private console.");
    println!();
    print!("{}", report::qr(&url));
    print_new_code(&recovered.next_code);
    Ok(())
}

/// Echo is off while the code is typed: `kampr recover` is run from a Herdr pane often enough,
/// and a read-only device receives every frame of every pane. A mistyped code is still live.
fn prompt_for_code() -> Result<Option<String>> {
    let hidden = conceal(std::io::stdin().is_terminal(), || echo(false))?;
    println!();
    println!("  Type the recovery code from your paper record.");
    print!("  code  ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    if hidden.is_some() {
        println!();
    }
    drop(hidden);
    read?;
    let code = line.trim().to_string();
    Ok((!code.is_empty()).then_some(code))
}

/// A console whose echo cannot be turned off is refused rather than fallen back on. The prompt
/// renders identically either way, so the operator's first sign that the code went to every
/// screen watching this pane would be seeing it there, after typing it.
fn conceal(console: bool, off: impl FnOnce() -> bool) -> Result<Option<Echo>> {
    if !console {
        return Ok(None);
    }
    if !off() {
        bail!(
            "this console will not turn echo off, so the recovery code would be typed in the \
             clear — and `kampr recover` is run from a herdr pane often enough, where every \
             read-only device receives the frame it lands in. Run it from a console where \
             `stty -echo` works."
        );
    }
    Ok(Some(Echo))
}

/// Echo goes back on however the prompt ends, including an unwind out of the read.
struct Echo;

impl Drop for Echo {
    fn drop(&mut self) {
        echo(true);
    }
}

fn echo(on: bool) -> bool {
    Command::new("stty")
        .arg(if on { "echo" } else { "-echo" })
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_console_that_cannot_hide_the_code_is_refused_rather_than_echoing_it() {
        let Err(refused) = conceal(true, || false) else {
            panic!("a prompt that echoes the code was accepted");
        };
        let said = refused.to_string();
        assert!(said.contains("echo"), "{said}");
        assert!(said.contains("read-only") || said.contains("pane"), "{said}");
    }

    #[test]
    fn a_stdin_that_is_not_a_console_has_no_screen_to_keep_the_code_off() {
        assert!(conceal(false, || false).expect("a pipe needs no stty").is_none());
    }
}
