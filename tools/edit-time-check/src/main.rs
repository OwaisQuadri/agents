use edit_time_check::{
    decode_request, evaluate, Decision, DiagnosticRule, Options, Response, Stage, MAX_REQUEST_BYTES,
};
use std::io::{Read, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

fn main() -> ExitCode {
    let stage = Stage::new();
    let (sender, receiver) = mpsc::channel();
    let worker_stage = stage.clone();
    std::thread::spawn(move || {
        let result = run(&worker_stage);
        let _ = sender.send(result);
    });
    let mut response = loop {
        match receiver.recv_timeout(Duration::from_millis(1)) {
            Ok(response) => break response,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some((rule, reason)) = stage.expired() {
                    break stage.error(rule, reason);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break stage.error(DiagnosticRule::CHECKER, "checker worker failed".into())
            }
        }
    };
    if let Some((rule, reason)) = stage.expired() {
        response = stage.error(rule, reason);
    }
    stage.finish(&mut response);
    let is_error = response.decision == Decision::Error;
    let is_block = response.decision == Decision::Block;
    let bytes = match serde_json::to_vec(&response) {
        Ok(bytes) if response.is_valid() && bytes.len() <= MAX_REQUEST_BYTES => bytes,
        _ => {
            let mut fallback = stage.error(
                DiagnosticRule::CHECKER,
                "checker response exceeds output limit or is invalid".into(),
            );
            stage.finish(&mut fallback);
            if serde_json::to_writer(std::io::stdout().lock(), &fallback).is_err() {
                return ExitCode::from(2);
            }
            return ExitCode::from(2);
        }
    };
    let mut stdout = std::io::stdout().lock();
    if let Some((rule, reason)) = stage.expired() {
        let mut response = stage.error(rule, reason);
        stage.finish(&mut response);
        let _ = serde_json::to_writer(&mut stdout, &response);
        return ExitCode::from(2);
    }
    if stdout
        .write_all(&bytes)
        .and_then(|()| stdout.write_all(b"\n"))
        .is_err()
    {
        return ExitCode::from(2);
    }
    if is_error {
        ExitCode::from(2)
    } else if is_block {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn run(stage: &Stage) -> Response {
    let mut bytes = Vec::new();
    if std::io::stdin()
        .lock()
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Response::error(
            "invalid-request".into(),
            "/".into(),
            DiagnosticRule::CHECKER,
            "cannot read request".into(),
        );
    }
    let request = match decode_request(&bytes) {
        Ok(request) => request,
        Err(reason) => {
            return Response::error(
                "invalid-request".into(),
                "/".into(),
                DiagnosticRule::CHECKER,
                reason,
            )
        }
    };
    stage.bind(&request);
    let options = match Options::from_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(reason) => {
            return Response::error(
                request.request_id,
                request.path,
                DiagnosticRule::CHECKER,
                reason,
            )
        }
    };
    evaluate(&request, &options, stage)
}
