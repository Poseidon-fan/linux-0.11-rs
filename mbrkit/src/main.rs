/// The binary entry point delegates all application logic to the library crate.
fn main() {
    if let Err(error) = mbrkit::run() {
        if error.should_print() {
            eprintln!("{error}");
        }

        std::process::exit(error.exit_code());
    }
}
