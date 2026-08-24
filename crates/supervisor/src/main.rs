use std::process::ExitCode;

use serde_json::json;

fn main() -> ExitCode {
    match mission_supervisor::run(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{}",
                json!({ "event": "supervisor.error", "errorCode": error.code() })
            );
            ExitCode::from(error.exit_code())
        }
    }
}
