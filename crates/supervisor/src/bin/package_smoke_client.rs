use std::ffi::OsString;
use std::process::ExitCode;
use std::time::Duration;

use mission_supervisor::package_smoke::{PackageSmokeClientError, authenticated_ping};
use serde_json::json;

fn main() -> ExitCode {
    let result = parse_args(std::env::args_os().skip(1)).and_then(|(pipe_name, timeout)| {
        authenticated_ping(&pipe_name, timeout).map_err(error_code)
    });
    match result {
        Ok(success) => {
            println!(
                "{}",
                json!({
                    "ok": true,
                    "protocolVersion": success.protocol_version,
                    "supervisorVersion": success.supervisor_version,
                })
            );
            ExitCode::SUCCESS
        }
        Err(code) => {
            println!("{}", json!({ "ok": false, "errorCode": code }));
            ExitCode::FAILURE
        }
    }
}

fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(String, Duration), &'static str> {
    let mut args = args.into_iter();
    let mut pipe_name = None;
    let mut timeout_milliseconds = None;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--pipe-name") => {
                pipe_name = args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .filter(|value| !value.is_empty() && !value.contains('\0'));
            }
            Some("--timeout-milliseconds") => {
                timeout_milliseconds = args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|value| *value != 0);
            }
            Some(_) | None => return Err("PACKAGE_ARGUMENT_INVALID"),
        }
    }
    Ok((
        pipe_name.ok_or("PACKAGE_ARGUMENT_INVALID")?,
        Duration::from_millis(timeout_milliseconds.ok_or("PACKAGE_ARGUMENT_INVALID")?),
    ))
}

const fn error_code(error: PackageSmokeClientError) -> &'static str {
    match error {
        PackageSmokeClientError::Timeout => "PACKAGE_SMOKE_TIMEOUT",
        PackageSmokeClientError::Authentication => "PACKAGE_SMOKE_AUTHENTICATION_FAILED",
        PackageSmokeClientError::Protocol => "PACKAGE_SMOKE_PROTOCOL_FAILED",
        PackageSmokeClientError::Unavailable => "PACKAGE_SMOKE_UNAVAILABLE",
        PackageSmokeClientError::WorkerDidNotStop => "PACKAGE_SMOKE_WORKER_DID_NOT_STOP",
    }
}
