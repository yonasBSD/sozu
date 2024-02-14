//! A lightweight, fast, hot-reloadable manager of reverse proxy workers.
//!
//! This crate provides a binary that takes a configuration file and
//! launches a main process and proxy workers that all share the same state.
//!
//! ```
//! sozu --config config.toml start
//! ```
//!
//! The state is reloadable during runtime:
//! the main process can receive requests via a UNIX socket,
//! in order to add listeners, frontends, backends etc.
//!
//! The `sozu` binary works as a CLI to send requests to the main process via the UNIX socket.
//! For instance:
//!
//! ```
//! sozu --config config.toml listener http add --address 127.0.0.1:8080
//! ```
//!
//!
//! The requests sent to Sōzu are defined in protobuf in the `sozu_command_lib` crate,
//! which means other programs can use the protobuf definition and send roquests
//! to Sōzu via its UNIX socket.

#[macro_use]
extern crate sozu_lib as sozu;
#[macro_use]
extern crate sozu_command_lib;
#[cfg(target_os = "linux")]
extern crate num_cpus;

#[cfg(feature = "jemallocator")]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

/// the arguments to the sozu command line
mod cli;
/// Receives orders from the CLI, transmits to workers
// mod command;
mod command;
/// The command line logic
mod ctl;
/// Forking & restarting the main process using a more recent executable of Sōzu
mod upgrade;
/// Some unix helper functions
pub mod util;
/// Start and restart the worker UNIX processes
mod worker;

use std::panic;

#[cfg(target_os = "linux")]
use libc::{cpu_set_t, pid_t};

use sozu::metrics::METRICS;

use cli::Args;
use command::{begin_main_process, sessions::WorkerSession, StartError};
use ctl::CtlError;
use upgrade::UpgradeError;
use worker::WorkerError;

#[derive(thiserror::Error, Debug)]
enum MainError {
    #[error("failed to start Sōzu: {0}")]
    StartMain(StartError),
    #[error("failed to start new worker: {0}")]
    BeginWorker(WorkerError),
    #[error("failed to start new main process: {0}")]
    BeginNewMain(UpgradeError),
    #[error("{0}")]
    Cli(CtlError),
}

#[paw::main]
fn main(args: Args) {
    register_panic_hook();

    let result = match args.cmd {
        cli::SubCmd::Start => begin_main_process(&args).map_err(MainError::StartMain),
        // this is used only by the CLI when upgrading
        cli::SubCmd::Worker {
            fd: worker_to_main_channel_fd,
            scm: worker_to_main_scm_fd,
            configuration_state_fd,
            id,
            command_buffer_size,
            max_command_buffer_size,
        } => {
            let max_command_buffer_size =
                max_command_buffer_size.unwrap_or(command_buffer_size * 2);
            worker::begin_worker_process(
                worker_to_main_channel_fd,
                worker_to_main_scm_fd,
                configuration_state_fd,
                id,
                command_buffer_size,
                max_command_buffer_size,
            )
            .map_err(MainError::BeginWorker)
        }
        // this is used only by the CLI when upgrading
        cli::SubCmd::Main {
            fd,
            upgrade_fd,
            command_buffer_size,
            max_command_buffer_size,
        } => {
            let max_command_buffer_size =
                max_command_buffer_size.unwrap_or(command_buffer_size * 2);
            upgrade::begin_new_main_process(
                fd,
                upgrade_fd,
                command_buffer_size,
                max_command_buffer_size,
            )
            .map_err(MainError::BeginNewMain)
        }
        _ => ctl::ctl(args).map_err(MainError::Cli),
    };
    match result {
        Ok(_) => {}
        Err(main_error) => println!("{}", main_error),
    }
}

/// Set workers process affinity, see man sched_setaffinity
/// Bind each worker (including the main) process to a CPU core.
/// Can bind multiple processes to a CPU core if there are more processes
/// than CPU cores. Only works on Linux.
#[cfg(target_os = "linux")]
fn set_workers_affinity(workers: &Vec<WorkerSession>) {
    let mut cpu_count = 0;
    let max_cpu = num_cpus::get();

    // +1 for the main process that will also be bound to its CPU core
    if (workers.len() + 1) > max_cpu {
        warn!(
            "There are more workers than available CPU cores, \
          multiple workers will be bound to the same CPU core. \
          This may impact performances"
        );
    }

    let main_pid = unsafe { libc::getpid() };
    set_process_affinity(main_pid, cpu_count);
    cpu_count += 1;

    for worker in workers {
        if cpu_count >= max_cpu {
            cpu_count = 0;
        }

        set_process_affinity(worker.pid, cpu_count);

        cpu_count += 1;
    }
}

/// Set workers process affinity, see man sched_setaffinity
/// Bind each worker (including the main) process to a CPU core.
/// Can bind multiple processes to a CPU core if there are more processes
/// than CPU cores. Only works on Linux.
#[cfg(not(target_os = "linux"))]
fn set_workers_affinity(_: &Vec<cli::SubCmd>) {}

/// Set a specific process to run onto a specific CPU core
#[cfg(target_os = "linux")]
use std::mem;
#[cfg(target_os = "linux")]
fn set_process_affinity(pid: pid_t, cpu: usize) {
    unsafe {
        let mut cpu_set: cpu_set_t = mem::zeroed();
        let size_cpu_set = mem::size_of::<cpu_set_t>();
        libc::CPU_SET(cpu, &mut cpu_set);
        libc::sched_setaffinity(pid, size_cpu_set, &cpu_set);

        debug!("Worker {} bound to CPU core {}", pid, cpu);
    };
}

fn register_panic_hook() {
    // We save the original panic hook so we can call it later
    // to have the original behavior
    let original_panic_hook = panic::take_hook();

    panic::set_hook(Box::new(move |panic_info| {
        incr!("panic");
        METRICS.with(|metrics| {
            (*metrics.borrow_mut()).send_data();
        });

        (*original_panic_hook)(panic_info)
    }));
}
