use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=dist");
    println!("cargo:rerun-if-env-changed=KAMPR_REQUIRE_BUNDLE");

    if std::env::var_os("KAMPR_REQUIRE_BUNDLE").is_none() {
        return;
    }

    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let dist = Path::new(&manifest).join("dist");

    let mut shell = false;
    let mut code = false;
    if let Ok(entries) = std::fs::read_dir(&dist) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "index.html" {
                shell = entry.metadata().map(|m| m.len() > 0).unwrap_or(false);
            } else if name.ends_with(".wasm") || name.ends_with(".js") {
                code = true;
            }
        }
    }

    if !shell || !code {
        println!(
            "cargo:warning=KAMPR_REQUIRE_BUNDLE is set but {} holds no client bundle",
            dist.display()
        );
        panic!(
            "no client bundle in {}: this binary would ship the placeholder page.\n\
             Stage it first:  cd client && ./gradlew :webApp:stageNodeBundle\n\
             (index.html present: {shell}, wasm/js present: {code})",
            dist.display()
        );
    }
}
