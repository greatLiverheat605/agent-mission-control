use std::process::ExitCode;

use mission_supervisor::RunError;

fn main() -> ExitCode {
    match mission_supervisor::run(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(RunError::AlreadyRunning) => ExitCode::from(23),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
