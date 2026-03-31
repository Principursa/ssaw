fn main() {
    if let Err(error) = ssaw::cli::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
