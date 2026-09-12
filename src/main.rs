use std::path::Path;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => run_gui(),
        ["--probe"] => lily58_assistant::probe::run(&mut std::io::stdout(), None),
        ["--probe", "--dump-definition", file] => {
            lily58_assistant::probe::run(&mut std::io::stdout(), Some(Path::new(file)))
        }
        _ => {
            eprintln!("usage: lily58-assistant [--probe [--dump-definition FILE]]");
            std::process::exit(2);
        }
    }
}

fn run_gui() -> anyhow::Result<()> {
    lily58_assistant::ui::run().map_err(|e| anyhow::anyhow!("{e}"))
}
