fn main() -> std::process::ExitCode {
    // An ordinary return, not `std::process::exit`: whatever the app holds is
    // dropped on the way out.
    match orbit_desktop::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Orbit no pudo arrancar: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
