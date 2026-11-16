fn main() {
    std::process::exit(ukiryu_cli::dispatch::run(std::env::args_os()));
}
