use candid::{Nat, Principal};
use chrono::prelude::*;
use handlebars::{handlebars_helper, Handlebars};
use ic_cdk::api::management_canister::main::{
    canister_info, canister_status, CanisterChange, CanisterInfoRequest, CanisterStatusResponse,
};
use ic_cdk::api::management_canister::main::{CanisterIdRecord, CanisterStatusType, LogVisibility};
use ic_http_certification::{HttpRequest, HttpResponse};
use include_dir::File;
use num_format::{Buffer, CustomFormat, Grouping};
use num_traits::cast::ToPrimitive;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::time::Duration;

mod asset;

thread_local! {
    static LAST_CYCLES_FOR_TIMER: RefCell<u64> = RefCell::new(0);
}

pub const ENABLE_TEMPLATING: bool = true;
const UPDATE_INTERVAL_SECS: u64 = 120;

handlebars_helper!(toJSON: |value: CanisterChange| serde_json::to_string_pretty(&value).unwrap().to_string());
handlebars_helper!(toNumLocale: |value: u64| {
    let format = CustomFormat::builder().grouping(Grouping::Standard).separator("_").build().unwrap();
    let mut buf = Buffer::new();
    buf.write_formatted(&(value), &format);
    buf.to_string()
});

#[ic_cdk::init]
fn init() {
    certify_on_timer();
}

#[ic_cdk::post_upgrade]
fn post_upgrade() {
    certify_on_timer();
}

fn certify_on_timer() {
    ic_cdk_timers::set_timer_interval(Duration::from_secs(UPDATE_INTERVAL_SECS), || {
        ic_cdk::spawn(async {
            let now = ic_cdk::api::performance_counter(1);
            asset::certify_all_assets().await;
            let elapsed = ic_cdk::api::performance_counter(1);

            // instruction to cycles
            // subnet size = 13 and ten_update_inst = 10 cycles
            // cycles = inst * (10 / 10) * (13 / 13) = inst
            LAST_CYCLES_FOR_TIMER.with_borrow_mut(|v| *v = elapsed - now);
        })
    });
}

#[ic_cdk::query]
fn http_request(req: HttpRequest) -> HttpResponse {
    let _path = req.get_path().expect("Failed to parse request path");
    asset::serve_asset(&req)
}

async fn serve_canister_info<'a>(file: &File<'a>) -> String {
    let response = canister_status(CanisterIdRecord {
        canister_id: ic_cdk::id(),
    })
    .await
    .unwrap()
    .0;

    let info = canister_info(CanisterInfoRequest {
        canister_id: ic_cdk::id(),
        num_requested_changes: Some(20),
    })
    .await
    .unwrap()
    .0;

    let mut handlebars = Handlebars::new();
    let source = file.contents();
    assert!(handlebars
        .register_template_string("metrics", String::from_utf8_lossy(source))
        .is_ok());
    handlebars.register_helper("toJSON", Box::new(toJSON));
    handlebars.register_helper("toNumLocale", Box::new(toNumLocale));

    let mut definite_response = DefiniteCanisterStatus::from(response);
    definite_response.last_updated_at = timestamp(ic_cdk::api::time());
    definite_response.canister_history = info.recent_changes;
    definite_response.last_cycles_cost = LAST_CYCLES_FOR_TIMER.with_borrow(|v| *v);
    handlebars.render("metrics", &definite_response).unwrap()
}

#[derive(Serialize, Deserialize, Debug)]
struct DefiniteCanisterStatus {
    pub status: CanisterStatusType,
    pub module_hash: String,
    pub memory_size: u64,
    pub cycles: u64,
    pub idle_cycles_burned_per_day: u64,
    pub reserved_cycles: u64,

    pub query_num_calls_total: u64,
    pub query_num_instructions_total: u64,
    pub query_request_payload_bytes_total: u64,
    pub query_response_payload_bytes_total: u64,

    pub controllers: Vec<Principal>,
    pub compute_allocation: u64,
    pub memory_allocation: u64,
    pub freezing_threshold: u64,
    pub reserved_cycles_limit: u64,
    pub log_visibility: LogVisibility,
    pub wasm_memory_limit: u64,

    pub last_updated_at: String,
    pub canister_history: Vec<CanisterChange>,
    pub last_cycles_cost: u64,
}

impl From<CanisterStatusResponse> for DefiniteCanisterStatus {
    fn from(value: CanisterStatusResponse) -> Self {
        Self {
            status: value.status,
            module_hash: hex::encode(value.module_hash.expect("Wasm should exist")),
            memory_size: nu64(value.memory_size),
            cycles: nu64(value.cycles),
            idle_cycles_burned_per_day: nu64(value.idle_cycles_burned_per_day),
            reserved_cycles: nu64(value.reserved_cycles),

            query_num_calls_total: nu64(value.query_stats.num_calls_total),
            query_num_instructions_total: nu64(value.query_stats.num_instructions_total),
            query_request_payload_bytes_total: nu64(value.query_stats.request_payload_bytes_total),
            query_response_payload_bytes_total: nu64(
                value.query_stats.response_payload_bytes_total,
            ),

            controllers: value.settings.controllers,
            compute_allocation: nu64(value.settings.compute_allocation),
            memory_allocation: nu64(value.settings.memory_allocation),
            freezing_threshold: nu64(value.settings.freezing_threshold),
            reserved_cycles_limit: nu64(value.settings.reserved_cycles_limit),
            log_visibility: value.settings.log_visibility,
            wasm_memory_limit: nu64(value.settings.wasm_memory_limit),

            last_updated_at: String::new(),
            canister_history: vec![],
            last_cycles_cost: 0,
        }
    }
}

fn nu64(num: Nat) -> u64 {
    num.0.to_u64().expect("Nat doesn't fit into u64")
}

fn timestamp(time: u64) -> String {
    let timestamp = time as i64;
    let datetime = DateTime::from_timestamp_nanos(timestamp);
    datetime.format("%Y-%m-%d %H:%M:%S").to_string()
}
