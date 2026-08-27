#![forbid(unsafe_code)]

// TODO(AGNT-0032.T150): Expose frontier source and store modules to the production runtime.
#[cfg(not(test))]
mod audit;
#[cfg(not(test))]
mod cli;
#[cfg(not(test))]
mod judge;
#[cfg(not(test))]
mod model;
#[cfg(not(test))]
mod model_capabilities;
#[cfg(not(test))]
mod models;
#[cfg(not(test))]
mod pi_runner;
#[cfg(not(test))]
mod pool_source;
#[cfg(not(test))]
mod pool_store;
#[cfg(not(test))]
mod ports;
#[cfg(not(test))]
mod publication;
#[cfg(not(test))]
mod service;
#[cfg(not(test))]
mod source;
#[cfg(not(test))]
mod statistics;
#[cfg(not(test))]
mod store;
#[cfg(not(test))]
mod t1_screen_campaign_store;
#[cfg(not(test))]
mod t1_screen_store;
#[cfg(not(test))]
mod testing;
#[cfg(not(test))]
mod tier_writer;
#[cfg(not(test))]
mod verifier;

#[cfg(not(test))]
fn main() {
    if let Err(error) = cli::run_main() {
        eprintln!("{error:?}");
        std::process::exit(1);
    }
}

#[cfg(test)]
fn main() {}
