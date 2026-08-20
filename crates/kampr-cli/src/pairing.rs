use crate::report::Local;
use anyhow::Result;
use kampr_auth::{Delivery, Pairing, Role};
use std::io::{IsTerminal, Write};

pub async fn create(local: &Local, role: Role) -> Result<Pairing> {
    Ok(local.auth.create_pairing(role, Delivery::Console).await?)
}

/// A code printed here is printed into a Herdr pane, and a read-only device receives every frame
/// of every pane. The keypress this waits for is the one channel such a device does not have, so
/// it is what turns a readable code back into a credential.
pub async fn arm(local: &Local, pairing: &Pairing) -> Result<()> {
    if pairing.armed {
        return Ok(());
    }
    let window = local.auth.policy().arming_window.as_secs();
    println!();
    println!("  A device is already paired, and a read-only device sees every pane — so this");
    println!("  code does nothing until you allow it from this console.");
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        println!("  Run `kampr setup` in a shell to allow a device to pair.");
        return Ok(());
    }
    print!("  enter the code on your device, then press Enter here ({window}s window) ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if local.auth.arm_pairing(&pairing.code).await? {
        println!("  pairing open for {window} seconds");
    } else {
        println!("  that code is no longer valid — make another one");
    }
    Ok(())
}
