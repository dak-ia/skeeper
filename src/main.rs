use std::process::ExitCode;

fn main() -> ExitCode {
    match skeeper::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
