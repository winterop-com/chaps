//! End-to-end tests for the commands that write a deployment directory.
//!
//! Every run is `--offline` with its own cache directory, so the embedded
//! marketplace snapshot is what the CLI sees and no test touches the network
//! or the developer's real cache.

mod auth;
mod backup;
mod chap;
mod cleanup;
mod common;
mod components;
mod dhis2;
mod dhis2_stand_in;
mod doctor;
mod expose;
mod help;
mod init;
mod jobs_api;
mod lock;
mod logs;
mod models;
mod models_add;
mod models_configs;
mod models_configs_edit;
mod models_configs_update;
mod models_test;
mod preflight;
mod run;
mod run_stop;
mod status;
mod top;
mod update;
mod volumes;
mod wait;
