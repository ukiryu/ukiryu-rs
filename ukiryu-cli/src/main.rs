fn main() {
    let matches = ukiryu_cli::arguments::command().get_matches();
    if matches.subcommand_name() == Some("version") {
        println!("ukiryu {}", ukiryu::VERSION);
    } else {
        eprintln!(
            "Ukiryu CLI migration is incomplete; the selected command handler is not integrated yet."
        );
        std::process::exit(2);
    }
}
