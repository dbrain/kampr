//! Throwaway containers for the Docker suites (docs/19), always removed on the way out. A machine
//! with no daemon, or without the image, skips them: Docker is optional on a node.

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

pub const IMAGE: &str = "nats:2.15.2-alpine";

pub fn available() -> bool {
    let image = Command::new("docker")
        .args(["image", "inspect", IMAGE])
        .output()
        .is_ok_and(|o| o.status.success());
    if !image {
        eprintln!("skipped: no docker daemon answering, or no {IMAGE}");
    }
    image
}

pub fn name() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("kt-{:x}{nanos:08x}{n:x}", std::process::id())
}

/// `docker run -d` of [`IMAGE`] with `sh` as its entrypoint: `args` up to `--` are `docker run`'s,
/// and the one after it is the script.
pub struct Container(pub String);

impl Container {
    pub fn run(args: &[&str]) -> Self {
        let container = Self(name());
        let split = args.iter().position(|a| *a == "--").unwrap_or(args.len());
        let mut argv = vec!["run", "-d", "--name", &container.0, "--entrypoint", "sh"];
        argv.extend(&args[..split]);
        argv.extend([IMAGE, "-c"]);
        argv.extend(args.get(split + 1..).unwrap_or_default());
        docker(&argv);
        container
    }

    /// `docker run -d --name <name> <args>`: the image and its command are the caller's.
    pub fn of(args: &[&str]) -> Self {
        let container = Self(name());
        let mut argv = vec!["run", "-d", "--name", &container.0];
        argv.extend(args);
        docker(&argv);
        container
    }

    pub fn status(&self) -> String {
        let out = Command::new("docker")
            .args(["inspect", "-f", "{{.State.Status}}", &self.0])
            .output()
            .expect("docker");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

fn docker(argv: &[&str]) {
    let out = Command::new("docker").args(argv).output().expect("docker");
    assert!(
        out.status.success(),
        "docker {argv:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// An image with no shell at all: [`IMAGE`]'s static `nats-server` alone on `scratch`, as
/// distroless and scratch images are. Removed on the way out.
pub struct NoShellImage(pub String);

impl NoShellImage {
    pub fn build() -> Self {
        let image = Self(format!("kt-noshell-{}", &name()[3..]));
        let dockerfile = format!(
            "FROM {IMAGE} AS src\nFROM scratch\nCOPY --from=src /usr/local/bin/nats-server /nats-server\nENTRYPOINT [\"/nats-server\"]\n"
        );
        let mut build = Command::new("docker")
            .args(["build", "-q", "-t", &image.0, "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("docker build");
        std::io::Write::write_all(&mut build.stdin.take().expect("stdin"), dockerfile.as_bytes())
            .expect("the Dockerfile");
        let out = build.wait_with_output().expect("docker build");
        assert!(
            out.status.success(),
            "docker build: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        image
    }
}

impl Drop for NoShellImage {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rmi", "-f", &self.0]).output();
    }
}
