fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--version" {
        println!("ukiryu {}", ukiryu::VERSION);
    } else {
        eprintln!(
            "Ukiryu CLI migration is incomplete; only --version is available in this foundation."
        );
        std::process::exit(2);
    }
}
