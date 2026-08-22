use std::ffi::OsString;
use std::io::Write;

use crate::model::{
    CliRequest, OutputFormat, QualificationReport, RunEvent, SkillEvalError, TrialRecord,
};
use crate::ports::QualificationRuntime;

pub(crate) fn parse_arguments(arguments: &[OsString]) -> Result<CliRequest, SkillEvalError> {
    unimplemented!()
}

pub(crate) fn execute_command(
    request: CliRequest,
    runtime: &mut dyn QualificationRuntime,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    unimplemented!()
}

pub(crate) fn render_event(
    event: &RunEvent,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    unimplemented!()
}

pub(crate) fn render_report(
    report: &QualificationReport,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    unimplemented!()
}

pub(crate) fn render_trial(
    trial: &TrialRecord,
    format: OutputFormat,
    output: &mut dyn Write,
) -> Result<(), SkillEvalError> {
    unimplemented!()
}
