use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

const MOST: u64 = 4 << 20;

// Claude reads a silent exit 0 as no decision (#627), so every failure here is one.
pub fn run(socket: &Path) {
    let _ = relay(socket);
}

fn relay(socket: &Path) -> std::io::Result<()> {
    let mut event = Vec::new();
    std::io::stdin().take(MOST + 1).read_to_end(&mut event)?;
    if event.len() as u64 > MOST {
        return Ok(());
    }
    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(&event)?;
    stream.shutdown(Shutdown::Write)?;
    let mut reply = Vec::new();
    stream.take(MOST).read_to_end(&mut reply)?;
    if !reply.is_empty() {
        let mut out = std::io::stdout().lock();
        out.write_all(&reply)?;
        out.flush()?;
    }
    Ok(())
}
