fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [keep, dir] if keep == "keep" => kampr_keep::serve(std::path::Path::new(dir)),
        _ => anyhow::bail!("usage: kampr-keep keep <dir>"),
    }
}
